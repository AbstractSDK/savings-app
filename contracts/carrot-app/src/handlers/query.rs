use std::collections::HashMap;

use abstract_app::traits::AccountIdentification;
use abstract_app::{
    abstract_core::objects::AnsAsset,
    traits::{AbstractNameService, Resolve},
};
use abstract_dex_adapter::DexInterface;
use cosmwasm_std::{to_json_binary, Binary, Coins, Decimal, Deps, Env};
use cw_asset::Asset;

use crate::{
    contract::{App, AppResult},
    error::AppError,
    helpers::get_balance,
    msg::{
        AppQueryMsg, AssetsBalanceResponse, AvailableRewardsResponse, CompoundStatusResponse,
        StrategyResponse,
    },
    state::{get_autocompound_status, Config, CONFIG},
};

pub fn query_handler(deps: Deps, env: Env, app: &App, msg: AppQueryMsg) -> AppResult<Binary> {
    match msg {
        AppQueryMsg::Balance {} => to_json_binary(&query_balance(deps, app)?),
        AppQueryMsg::AvailableRewards {} => to_json_binary(&query_rewards(deps, app)?),
        AppQueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        AppQueryMsg::Strategy {} => to_json_binary(&query_strategy(deps)?),
        AppQueryMsg::CompoundStatus {} => to_json_binary(&query_compound_status(deps, env, app)?),
        AppQueryMsg::RebalancePreview {} => todo!(),
    }
    .map_err(Into::into)
}

/// Gets the status of the compounding logic of the application
/// Accounts for the user's ability to pay for the gas fees of executing the contract.
fn query_compound_status(deps: Deps, env: Env, app: &App) -> AppResult<CompoundStatusResponse> {
    let config = CONFIG.load(deps.storage)?;
    let status = get_autocompound_status(
        deps.storage,
        &env,
        config.autocompound_config.cooldown_seconds.u64(),
    )?;

    let gas_denom = config
        .autocompound_config
        .rewards
        .gas_asset
        .resolve(&deps.querier, &app.ans_host(deps)?)?;

    let reward = Asset::new(gas_denom.clone(), config.autocompound_config.rewards.reward);

    let user = app.account_base(deps)?.proxy;

    let user_gas_balance = gas_denom.query_balance(&deps.querier, user.clone())?;

    let rewards_available = if user_gas_balance >= reward.amount {
        true
    } else {
        // check if can swap
        let rewards_config = config.autocompound_config.rewards;
        let dex = app.ans_dex(deps, config.dex);

        // Reverse swap to see how many swap coins needed
        let required_gas_coins = reward.amount - user_gas_balance;
        let response = dex.simulate_swap(
            AnsAsset::new(rewards_config.gas_asset, required_gas_coins),
            rewards_config.swap_asset.clone(),
        )?;

        // Check if user has enough of swap coins
        let user_swap_balance = get_balance(rewards_config.swap_asset, deps, user, app)?;
        let required_swap_amount = response.return_amount;

        user_swap_balance > required_swap_amount
    };

    Ok(CompoundStatusResponse {
        status,
        reward: reward.into(),
        rewards_available,
    })
}

pub fn query_strategy(deps: Deps) -> AppResult<StrategyResponse> {
    let config = CONFIG.load(deps.storage)?;

    Ok(StrategyResponse {
        strategy: config.balance_strategy,
    })
}

fn query_config(deps: Deps) -> AppResult<Config> {
    Ok(CONFIG.load(deps.storage)?)
}

pub fn query_balance(deps: Deps, app: &App) -> AppResult<AssetsBalanceResponse> {
    let mut funds = Coins::default();
    query_strategy(deps)?.strategy.0.iter().try_for_each(|s| {
        let deposit_value = s.yield_source.ty.user_deposit(deps, app)?;
        for fund in deposit_value {
            funds.add(fund)?;
        }
        Ok::<_, AppError>(())
    })?;

    Ok(AssetsBalanceResponse {
        balances: funds.into(),
    })
}

fn query_rewards(deps: Deps, app: &App) -> AppResult<AvailableRewardsResponse> {
    let strategy = query_strategy(deps)?.strategy;

    let mut rewards = Coins::default();
    strategy.0.into_iter().try_for_each(|s| {
        let this_rewards = s.yield_source.ty.user_rewards(deps, app)?;
        for fund in this_rewards {
            rewards.add(fund)?;
        }
        Ok::<_, AppError>(())
    })?;

    Ok(AvailableRewardsResponse {
        available_rewards: rewards.into(),
    })
}

pub fn query_exchange_rate(_deps: Deps, _denom: String, _app: &App) -> AppResult<Decimal> {
    // In the first iteration, all deposited tokens are assumed to be equal to 1
    Ok(Decimal::one())
}

// Returns a hashmap with all request exchange rates
pub fn query_all_exchange_rates(
    deps: Deps,
    denoms: impl Iterator<Item = String>,
    app: &App,
) -> AppResult<HashMap<String, Decimal>> {
    denoms
        .into_iter()
        .map(|denom| Ok((denom.clone(), query_exchange_rate(deps, denom, app)?)))
        .collect()
}
