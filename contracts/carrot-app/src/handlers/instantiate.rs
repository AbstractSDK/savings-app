use crate::{
    autocompound::AutocompoundState,
    contract::{App, AppResult},
    msg::AppInstantiateMsg,
    state::{Config, AUTOCOMPOUND_STATE, CONFIG},
};
use abstract_app::abstract_sdk::{features::AbstractNameService, AbstractResponse};
use cosmwasm_std::{DepsMut, Env, MessageInfo};

use super::execute::_inner_deposit;

pub fn instantiate_handler(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    app: App,
    msg: AppInstantiateMsg,
) -> AppResult {
    // We check the balance strategy is valid
    msg.balance_strategy.check(deps.as_ref(), &app)?;

    // We don't check the dex on instantiation

    // We query the ANS for useful information on the tokens and pool
    let ans = app.name_service(deps.as_ref());

    // Check validity of autocompound rewards
    msg.autocompound_config
        .rewards
        .check(deps.as_ref(), &msg.dex, ans.host())?;

    let config: Config = Config {
        dex: msg.dex,
        balance_strategy: msg.balance_strategy,
        autocompound_config: msg.autocompound_config,
    };
    CONFIG.save(deps.storage, &config)?;
    AUTOCOMPOUND_STATE.save(
        deps.storage,
        &AutocompoundState {
            last_compound: env.block.time,
        },
    )?;

    let mut response = app.response("instantiate_savings_app");

    // If provided - do an initial deposit
    if let Some(funds) = msg.deposit {
        let deposit_msgs = _inner_deposit(deps.as_ref(), &env, funds, None, &app)?;

        response = response.add_messages(deposit_msgs);
    }
    Ok(response)
}
