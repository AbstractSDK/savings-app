use std::{marker::PhantomData, str::FromStr};

use super::{yield_type::YieldTypeImplementation, ShareType};
use crate::{
    ans_assets::AnsAssets,
    check::{Checked, Unchecked},
    contract::{App, AppResult},
    error::AppError,
    handlers::swap_helpers::DEFAULT_SLIPPAGE,
    helpers::unwrap_native,
    replies::{OSMOSIS_ADD_TO_POSITION_REPLY_ID, OSMOSIS_CREATE_POSITION_REPLY_ID},
    state::CONFIG,
};
use abstract_app::traits::AbstractNameService;
use abstract_app::{objects::AnsAsset, traits::AccountIdentification};
use abstract_dex_adapter::DexInterface;
use abstract_sdk::Execution;
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure, Coin, Coins, CosmosMsg, Decimal, Deps, ReplyOn, SubMsg, Uint128};
use cw_asset::{Asset, AssetInfo};
use osmosis_std::{
    cosmwasm_to_proto_coins, try_proto_to_cosmwasm_coins,
    types::osmosis::concentratedliquidity::v1beta1::{
        ConcentratedliquidityQuerier, FullPositionBreakdown, MsgAddToPosition,
        MsgCollectIncentives, MsgCollectSpreadRewards, MsgCreatePosition, MsgWithdrawPosition,
    },
};

#[cw_serde]
pub struct ConcentratedPoolParamsBase<T> {
    // This is part of the pool parameters
    pub pool_id: u64,
    // This is part of the pool parameters
    pub lower_tick: i64,
    // This is part of the pool parameters
    pub upper_tick: i64,
    // This is something that is filled after position creation
    // This is not actually a parameter but rather state
    // This can be used as a parameter for existing positions
    pub position_id: Option<u64>,
    pub _phantom: PhantomData<T>,
}

pub type ConcentratedPoolParamsUnchecked = ConcentratedPoolParamsBase<Unchecked>;
pub type ConcentratedPoolParams = ConcentratedPoolParamsBase<Checked>;

impl YieldTypeImplementation for ConcentratedPoolParams {
    fn deposit(&self, deps: Deps, assets: AnsAssets, app: &App) -> AppResult<Vec<SubMsg>> {
        // We verify there is a position stored
        if let Ok(position) = self.position(deps) {
            self.raw_deposit(deps, assets, app, position)
        } else {
            // We need to create a position
            self.create_position(deps, assets, app)
        }
    }

    fn withdraw(
        &self,
        deps: Deps,
        amount: Option<Uint128>,
        app: &App,
    ) -> AppResult<Vec<CosmosMsg>> {
        let position = self.position(deps)?;
        let position_details = position.position.unwrap();

        let total_liquidity = position_details.liquidity.replace('.', "");

        let liquidity_amount = if let Some(amount) = amount {
            amount.to_string()
        } else {
            // TODO: it's decimals inside contracts
            total_liquidity.clone()
        };
        let user = app.account_base(deps)?.proxy;

        // We need to execute withdraw on the user's behalf
        Ok(vec![MsgWithdrawPosition {
            position_id: position_details.position_id,
            sender: user.to_string(),
            liquidity_amount: liquidity_amount.clone(),
        }
        .into()])
    }

    fn withdraw_rewards(&self, deps: Deps, app: &App) -> AppResult<(AnsAssets, Vec<CosmosMsg>)> {
        let position = self.position(deps)?;
        let position_details = position.position.unwrap();
        let ans = app.name_service(deps);

        let user = app.account_base(deps)?.proxy;
        let mut rewards = AnsAssets::default();
        let mut msgs: Vec<CosmosMsg> = vec![];
        // If there are external incentives, claim them.
        if !position.claimable_incentives.is_empty() {
            for coin in try_proto_to_cosmwasm_coins(position.claimable_incentives)? {
                rewards.add(ans.query(&Asset::from(coin))?)?;
            }
            msgs.push(
                MsgCollectIncentives {
                    position_ids: vec![position_details.position_id],
                    sender: user.to_string(),
                }
                .into(),
            );
        }

        // If there is income from swap fees, claim them.
        if !position.claimable_spread_rewards.is_empty() {
            for coin in try_proto_to_cosmwasm_coins(position.claimable_spread_rewards)? {
                rewards.add(ans.query(&Asset::from(coin))?)?;
            }
            msgs.push(
                MsgCollectSpreadRewards {
                    position_ids: vec![position_details.position_id],
                    sender: position_details.address.clone(),
                }
                .into(),
            )
        }

        Ok((rewards, msgs))
    }

    /// This may return 0, 1 or 2 elements depending on the position's status
    fn user_deposit(&self, deps: Deps, app: &App) -> AppResult<AnsAssets> {
        let position = self.position(deps)?;
        let ans = app.name_service(deps);

        Ok([
            try_proto_to_cosmwasm_coins(position.asset0)?,
            try_proto_to_cosmwasm_coins(position.asset1)?,
        ]
        .into_iter()
        .flatten()
        .map(|mut fund| {
            // This is used because osmosis seems to charge 1 amount for withdrawals on all positions
            fund.amount -= Uint128::one();
            ans.query(&Asset::from(fund))
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()?)
    }

    fn user_rewards(&self, deps: Deps, app: &App) -> AppResult<AnsAssets> {
        let ans = app.name_service(deps);

        let position = self.position(deps)?;

        let mut rewards = AnsAssets::default();
        for coin in try_proto_to_cosmwasm_coins(position.claimable_incentives)? {
            rewards.add(ans.query(&Asset::from(coin))?)?;
        }

        for coin in try_proto_to_cosmwasm_coins(position.claimable_spread_rewards)? {
            rewards.add(ans.query(&Asset::from(coin))?)?;
        }

        Ok(rewards)
    }

    fn user_liquidity(&self, deps: Deps, _app: &App) -> AppResult<Uint128> {
        let position = self.position(deps)?;
        let total_liquidity = position.position.unwrap().liquidity.replace('.', "");

        Ok(Uint128::from_str(&total_liquidity)?)
    }

    fn share_type(&self) -> super::ShareType {
        ShareType::Dynamic
    }
}

impl ConcentratedPoolParams {
    /// This function creates a position for the user,
    /// 1. Swap the indicated funds to match the asset0/asset1 ratio and deposit as much as possible in the pool for the given parameters
    /// 2. Create a new position
    /// 3. Store position id from create position response
    ///
    /// * `lower_tick` - Concentrated liquidity pool parameter
    /// * `upper_tick` - Concentrated liquidity pool parameter
    /// * `funds` -  Funds that will be deposited from the user wallet directly into the pool. DO NOT SEND FUNDS TO THIS ENDPOINT
    /// * `asset0` - The target amount of asset0.denom that the user will deposit inside the pool
    /// * `asset1` - The target amount of asset1.denom that the user will deposit inside the pool
    ///
    /// asset0 and asset1 are only used in a ratio to each other. They are there to make sure that the deposited funds will ALL land inside the pool.
    /// We don't use an asset ratio because either one of the amounts can be zero
    /// See https://docs.osmosis.zone/osmosis-core/modules/concentrated-liquidity for more details
    ///
    fn create_position(
        &self,
        deps: Deps,
        funds: AnsAssets,
        app: &App,
        // create_position_msg: CreatePositionMessage,
    ) -> AppResult<Vec<SubMsg>> {
        let ans = app.name_service(deps);
        let proxy_addr = app.account_base(deps)?.proxy;
        // 2. Create a position
        let tokens_provided = funds
            .into_iter()
            .map(|f| {
                let asset = ans.query(&f)?;
                unwrap_native(&asset.info).map(|denom| Coin {
                    denom,
                    amount: asset.amount,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ordered_tokens_provided: Coins = tokens_provided.try_into()?;

        let tokens_provided = cosmwasm_to_proto_coins(ordered_tokens_provided);
        let msg = app.executor(deps).execute_with_reply_and_data(
            MsgCreatePosition {
                pool_id: self.pool_id,
                sender: proxy_addr.to_string(),
                lower_tick: self.lower_tick,
                upper_tick: self.upper_tick,
                tokens_provided,
                token_min_amount0: "0".to_string(),
                token_min_amount1: "0".to_string(),
            }
            .into(),
            ReplyOn::Success,
            OSMOSIS_CREATE_POSITION_REPLY_ID,
        )?;

        Ok(vec![msg])
    }

    fn raw_deposit(
        &self,
        deps: Deps,
        deposit_assets: AnsAssets,
        app: &App,
        position: FullPositionBreakdown,
    ) -> AppResult<Vec<SubMsg>> {
        let ans = app.name_service(deps);
        let position_id = position.position.unwrap().position_id;

        let proxy_addr = app.account_base(deps)?.proxy;

        // We need to make sure the amounts are in the right order
        let resolved_asset0 = position
            .asset0
            .map(|a| ans.query(&AssetInfo::native(a.denom)))
            .transpose()?;
        let resolved_asset1 = position
            .asset1
            .map(|a| ans.query(&AssetInfo::native(a.denom)))
            .transpose()?;

        let mut assets = deposit_assets.into_iter();
        let deposit0 = assets.next();
        let deposit1 = assets.next();

        let (amount0, amount1) = match resolved_asset0
            .map(|c| c == deposit0.clone().map(|d| d.name).unwrap_or_default())
            .or(resolved_asset1.map(|c| c == deposit1.clone().map(|d| d.name).unwrap_or_default()))
        {
            Some(true) => (
                deposit0.map(|d| d.amount).unwrap_or_default(),
                deposit1.map(|d| d.amount).unwrap_or_default(),
            ), // we already had the right order
            Some(false) => (
                deposit1.map(|d| d.amount).unwrap_or_default(),
                deposit0.map(|d| d.amount).unwrap_or_default(),
            ), // we had the wrong order
            None => return Err(AppError::NoPosition {}), // A position has to exist in order to execute this function. This should be unreachable
        };

        deps.api.debug("After amounts");

        let deposit_msg = app.executor(deps).execute_with_reply_and_data(
            MsgAddToPosition {
                position_id,
                sender: proxy_addr.to_string(),
                amount0: amount0.to_string(),
                amount1: amount1.to_string(),
                token_min_amount0: "0".to_string(),
                token_min_amount1: "0".to_string(),
            }
            .into(),
            cosmwasm_std::ReplyOn::Success,
            OSMOSIS_ADD_TO_POSITION_REPLY_ID,
        )?;
        deps.api.debug("After messages");

        Ok(vec![deposit_msg])
    }

    fn position(&self, deps: Deps) -> AppResult<FullPositionBreakdown> {
        let position_id = self.position_id.ok_or(AppError::NoPosition {})?;
        ConcentratedliquidityQuerier::new(&deps.querier)
            .position_by_id(position_id)
            .map_err(|e| AppError::UnableToQueryPosition(position_id, e))?
            .position
            .ok_or(AppError::NoPosition {})
    }
}

pub fn query_swap_price(
    deps: Deps,
    app: &App,
    max_spread: Option<Decimal>,
    belief_price0: Option<Decimal>,
    belief_price1: Option<Decimal>,
    asset0: AnsAsset,
    asset1: AnsAsset,
) -> AppResult<Decimal> {
    let config = CONFIG.load(deps.storage)?;

    // We take the biggest amount and simulate a swap for the corresponding asset
    let price = if asset0.amount > asset1.amount {
        let simulation_result = app
            .ans_dex(deps, config.dex.clone())
            .simulate_swap(asset0.clone(), asset1.name)?;

        let price = Decimal::from_ratio(asset0.amount, simulation_result.return_amount);
        if let Some(belief_price) = belief_price1 {
            ensure!(
                belief_price.abs_diff(price) <= max_spread.unwrap_or(DEFAULT_SLIPPAGE),
                AppError::MaxSpreadAssertion { price }
            );
        }
        price
    } else {
        let simulation_result = app
            .ans_dex(deps, config.dex.clone())
            .simulate_swap(asset1.clone(), asset0.name)?;

        let price = Decimal::from_ratio(simulation_result.return_amount, asset1.amount);
        if let Some(belief_price) = belief_price0 {
            ensure!(
                belief_price.abs_diff(price) <= max_spread.unwrap_or(DEFAULT_SLIPPAGE),
                AppError::MaxSpreadAssertion { price }
            );
        }
        price
    };

    Ok(price)
}
