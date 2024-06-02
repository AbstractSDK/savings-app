use abstract_sdk::{AccountAction, Execution, ExecutorMsg};
use cosmwasm_std::{Coin, Coins, Decimal, Deps};

use crate::{
    contract::{App, AppResult},
    error::AppError,
    yield_sources::{yield_type::YieldTypeImplementation, Strategy, StrategyElement},
};

impl Strategy {
    pub fn withdraw(
        self,
        deps: Deps,
        withdraw_share: Option<Decimal>,
        app: &App,
    ) -> AppResult<Vec<ExecutorMsg>> {
        self.0
            .into_iter()
            .map(|s| s.withdraw(deps, withdraw_share, app))
            .collect()
    }
    pub fn withdraw_preview(
        &mut self,
        deps: Deps,
        withdraw_share: Option<Decimal>,
        app: &App,
    ) -> AppResult<Vec<Coin>> {
        let mut withdraw_result = Coins::default();
        self.0.iter_mut().try_for_each(|s| {
            let funds = s.withdraw_preview(deps, withdraw_share, app)?;
            funds.into_iter().try_for_each(|f| withdraw_result.add(f))?;
            Ok::<_, AppError>(())
        })?;
        Ok(withdraw_result.into())
    }
}

impl StrategyElement {
    pub fn withdraw(
        mut self,
        deps: Deps,
        withdraw_share: Option<Decimal>,
        app: &App,
    ) -> AppResult<ExecutorMsg> {
        let this_withdraw_amount = withdraw_share
            .map(|share| {
                let this_amount = self.yield_source.params.user_liquidity(deps, app)?;
                let this_withdraw_amount = share * this_amount;

                Ok::<_, AppError>(this_withdraw_amount)
            })
            .transpose()?;
        let raw_msg = self
            .yield_source
            .params
            .withdraw(deps, this_withdraw_amount, app)?;

        Ok::<_, AppError>(
            app.executor(deps)
                .execute(vec![AccountAction::from_vec(raw_msg)])?,
        )
    }

    pub fn withdraw_preview(
        &mut self,
        deps: Deps,
        withdraw_share: Option<Decimal>,
        app: &App,
    ) -> AppResult<Vec<Coin>> {
        let current_deposit = self.yield_source.params.user_deposit(deps, app)?;

        if let Some(share) = withdraw_share {
            Ok(current_deposit
                .into_iter()
                .map(|funds| Coin {
                    denom: funds.denom,
                    amount: funds.amount * share,
                })
                .collect())
        } else {
            Ok(current_deposit)
        }
    }
}
