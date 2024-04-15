use super::{internal::execute_internal_action, swap_helpers::MAX_SPREAD};
use crate::{
    autocompound::AutocompoundState,
    check::Checkable,
    contract::{App, AppResult},
    distribution::deposit::generate_deposit_strategy,
    error::AppError,
    exchange_rate::query_exchange_rate,
    handlers::query::query_balance,
    helpers::assert_contract,
    msg::{AppExecuteMsg, ExecuteMsg},
    state::{AUTOCOMPOUND_STATE, CONFIG, STRATEGY_CONFIG},
    yield_sources::{AssetShare, Strategy, StrategyUnchecked},
};
use abstract_app::traits::AbstractNameService;
use abstract_app::{abstract_sdk::features::AbstractResponse, objects::AssetEntry};
use abstract_dex_adapter::DexInterface;
use abstract_sdk::ExecutorMsg;
use cosmwasm_std::{
    to_json_binary, Coin, Coins, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Uint128,
    WasmMsg,
};
use cw_asset::Asset;

pub fn execute_handler(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    app: App,
    msg: AppExecuteMsg,
) -> AppResult {
    match msg {
        AppExecuteMsg::Deposit {
            funds,
            yield_sources_params,
        } => deposit(deps, env, info, funds, yield_sources_params, app),
        AppExecuteMsg::Withdraw { value, swap_to } => {
            withdraw(deps, env, info, value, swap_to, app)
        }
        AppExecuteMsg::Autocompound {} => autocompound(deps, env, info, app),
        AppExecuteMsg::UpdateStrategy { strategy, funds } => {
            update_strategy(deps, env, info, strategy, funds, app)
        }
        // Endpoints called by the contract directly
        AppExecuteMsg::Internal(internal_msg) => {
            assert_contract(&info, &env)?;
            execute_internal_action(deps, env, internal_msg, app)
        }
    }
}

fn deposit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    funds: Vec<Coin>,
    yield_source_params: Option<Vec<Option<Vec<AssetShare>>>>,
    app: App,
) -> AppResult {
    // Only the admin (manager contracts or account owner) can deposit as well as the contract itself
    app.admin
        .assert_admin(deps.as_ref(), &info.sender)
        .or(assert_contract(&info, &env))?;

    let target_strategy = STRATEGY_CONFIG.load(deps.storage)?;
    let deposit_msgs = _inner_deposit(
        deps.as_ref(),
        &env,
        funds,
        target_strategy,
        yield_source_params,
        &app,
    )?;

    AUTOCOMPOUND_STATE.save(
        deps.storage,
        &AutocompoundState {
            last_compound: env.block.time,
        },
    )?;

    Ok(app.response("deposit").add_messages(deposit_msgs))
}

fn withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Option<Uint128>,
    swap_to: Option<AssetEntry>,
    app: App,
) -> AppResult {
    // Only the authorized addresses (admin ?) can withdraw
    app.admin.assert_admin(deps.as_ref(), &info.sender)?;

    let msgs = _inner_withdraw(deps, &env, amount, swap_to, &app)?;

    Ok(app.response("withdraw").add_messages(msgs))
}

fn update_strategy(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    strategy: StrategyUnchecked,
    funds: Vec<Coin>,
    app: App,
) -> AppResult {
    // We load it raw because we're changing the strategy
    let old_strategy = STRATEGY_CONFIG.load(deps.storage)?;

    // We check the new strategy
    let strategy = strategy.check(deps.as_ref(), &app)?;

    // We execute operations to rebalance the funds between the strategies
    let mut available_funds: Coins = funds.try_into()?;
    // 1. We withdraw all yield_sources that are not included in the new strategies
    let all_stale_sources: Vec<_> = old_strategy
        .0
        .into_iter()
        .filter(|x| !strategy.0.contains(x))
        .collect();

    let (withdrawn_funds, withdraw_msgs): (Vec<Vec<Coin>>, Vec<Option<ExecutorMsg>>) =
        all_stale_sources
            .into_iter()
            .map(|s| {
                Ok::<_, AppError>((
                    s.withdraw_preview(deps.as_ref(), None, &app)
                        .unwrap_or_default(),
                    s.withdraw(deps.as_ref(), None, &app).ok(),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();

    withdrawn_funds
        .into_iter()
        .try_for_each(|f| f.into_iter().try_for_each(|f| available_funds.add(f)))?;

    // 2. We replace the strategy with the new strategy
    STRATEGY_CONFIG.save(deps.storage, &strategy)?;

    // 3. We deposit the funds into the new strategy
    let deposit_msgs = _inner_deposit(
        deps.as_ref(),
        &env,
        available_funds.into(),
        strategy,
        None,
        &app,
    )?;

    Ok(app
        .response("rebalance")
        .add_messages(
            withdraw_msgs
                .into_iter()
                .flatten()
                .collect::<Vec<ExecutorMsg>>(),
        )
        .add_messages(deposit_msgs))
}

// /// Auto-compound the position with earned fees and incentives.

fn autocompound(deps: DepsMut, env: Env, info: MessageInfo, app: App) -> AppResult {
    // Everyone can autocompound
    let strategy = STRATEGY_CONFIG.load(deps.storage)?;

    // We withdraw all rewards from protocols
    let (all_rewards, collect_rewards_msgs) = strategy.withdraw_rewards(deps.as_ref(), &app)?;

    // If there are no rewards, we can't do anything
    if all_rewards.is_empty() {
        return Err(crate::error::AppError::NoRewards {});
    }

    // Finally we deposit of all rewarded tokens into the position
    let msg_deposit = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::Module(AppExecuteMsg::Deposit {
            funds: all_rewards,
            yield_sources_params: None,
        }))?,
        funds: vec![],
    });

    let response = app
        .response("auto-compound")
        .add_messages(collect_rewards_msgs)
        .add_message(msg_deposit);

    let config = CONFIG.load(deps.storage)?;
    let executor_reward_messages =
        config.get_executor_reward_messages(deps.as_ref(), &env, info, &app)?;

    AUTOCOMPOUND_STATE.save(
        deps.storage,
        &AutocompoundState {
            last_compound: env.block.time,
        },
    )?;

    Ok(response.add_messages(executor_reward_messages))
}

pub fn _inner_deposit(
    deps: Deps,
    env: &Env,
    funds: Vec<Coin>,
    target_strategy: Strategy,
    yield_source_params: Option<Vec<Option<Vec<AssetShare>>>>,
    app: &App,
) -> AppResult<Vec<CosmosMsg>> {
    let (withdraw_strategy, deposit_msgs) =
        generate_deposit_strategy(deps, funds, target_strategy, yield_source_params, app)?;
    let deposit_withdraw_msgs = withdraw_strategy
        .into_iter()
        .map(|(el, share)| el.withdraw(deps, Some(share), app).map(Into::into))
        .collect::<Result<Vec<_>, _>>()?;
    let deposit_msgs = deposit_msgs
        .into_iter()
        .map(|msg| msg.to_cosmos_msg(env))
        .collect::<Result<Vec<_>, _>>()?;

    Ok([deposit_withdraw_msgs, deposit_msgs].concat())
}

fn _inner_withdraw(
    deps: DepsMut,
    _env: &Env,
    value: Option<Uint128>,
    swap_to: Option<AssetEntry>,
    app: &App,
) -> AppResult<Vec<CosmosMsg>> {
    // We need to select the share of each investment that needs to be withdrawn
    let withdraw_share = value
        .map(|value| {
            let total_deposit = query_balance(deps.as_ref(), app)?;

            if total_deposit.total_value.is_zero() {
                return Err(AppError::NoDeposit {});
            }
            Ok(Decimal::from_ratio(value, total_deposit.total_value))
        })
        .transpose()?;

    // We withdraw the necessary share from all registered investments
    let mut withdraw_msgs: Vec<CosmosMsg> = STRATEGY_CONFIG
        .load(deps.storage)?
        .withdraw(deps.as_ref(), withdraw_share, app)?
        .into_iter()
        .map(Into::into)
        .collect();

    if let Some(swap_to) = swap_to {
        let withdraw_preview = STRATEGY_CONFIG.load(deps.storage)?.withdraw_preview(
            deps.as_ref(),
            withdraw_share,
            app,
        )?;

        // We swap all withdraw_preview coins to the swap asset
        let config = CONFIG.load(deps.storage)?;
        let ans = app.name_service(deps.as_ref());
        let dex = app.ans_dex(deps.as_ref(), config.dex);
        withdraw_preview.into_iter().try_for_each(|fund| {
            let asset = ans.query(&Asset::from(fund.clone()))?;
            if asset.name != swap_to {
                let swap_msg = dex.swap(
                    asset,
                    swap_to.clone(),
                    Some(MAX_SPREAD),
                    Some(query_exchange_rate(deps.as_ref(), fund.denom, app)?),
                )?;
                withdraw_msgs.push(swap_msg);
            }
            Ok::<_, AppError>(())
        })?;
    }

    Ok(withdraw_msgs)
}
