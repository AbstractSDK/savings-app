mod common;

use crate::common::{deposit_with_funds, setup_test_tube, USDC, USDT};
use abstract_app::objects::{AnsAsset, AssetEntry};
use abstract_client::Application;
use carrot_app::{
    msg::{AppExecuteMsgFns, AppQueryMsgFns, AssetsBalanceResponse},
    yield_sources::{
        mars::MarsDepositParams, osmosis_cl_pool::ConcentratedPoolParamsBase,
        yield_type::YieldParamsBase, AssetShare, StrategyBase, StrategyElementBase,
        YieldSourceBase,
    },
    AppInterface,
};
use common::{INITIAL_LOWER_TICK, INITIAL_UPPER_TICK};
use cosmwasm_std::{coin, coins, Decimal, Uint128};
use cw_orch::{anyhow, prelude::*};

fn query_balances<Chain: CwEnv>(
    carrot_app: &Application<Chain, AppInterface<Chain>>,
) -> anyhow::Result<Uint128> {
    let balance = carrot_app.balance();
    if balance.is_err() {
        return Ok(Uint128::zero());
    }
    let sum = balance?
        .balances
        .iter()
        .fold(Uint128::zero(), |acc, e| acc + e.amount);

    Ok(sum)
}

#[test]
fn deposit_lands() -> anyhow::Result<()> {
    let (_, carrot_app) = setup_test_tube(false)?;

    // We should add funds to the account proxy
    let deposit_amount = 5_000u128;
    let balances_before = query_balances(&carrot_app)?;
    // Do the deposit
    deposit_with_funds(&carrot_app, vec![AnsAsset::new(USDT, deposit_amount)])?;

    // Check almost everything landed
    let balances_after = query_balances(&carrot_app)?;
    println!(
        "Expected deposit amount {}, actual deposit {}, remaining",
        deposit_amount,
        balances_after - balances_before,
    );
    assert!(
        balances_after > balances_before + Uint128::from(deposit_amount) * Decimal::percent(98)
    );

    // Do the second deposit
    let response = deposit_with_funds(&carrot_app, vec![AnsAsset::new(USDT, deposit_amount)])?;
    // Check almost everything landed
    let balances_after_second = query_balances(&carrot_app)?;
    println!(
        "Expected deposit amount {}, actual deposit {}, remaining",
        deposit_amount,
        balances_after_second - balances_after,
    );
    assert!(
        balances_after_second
            > balances_after + Uint128::from(deposit_amount) * Decimal::percent(98)
    );

    // We assert the deposit response is an add to position and not a create position
    response.event_attr_value("add_to_position", "new_position_id")?;

    Ok(())
}

#[test]
fn withdraw_position() -> anyhow::Result<()> {
    let (_, carrot_app) = setup_test_tube(false)?;

    let chain = carrot_app.get_chain().clone();

    // Add some more funds
    let deposit_amount = 10_000u128;
    // Do the deposit
    deposit_with_funds(&carrot_app, vec![AnsAsset::new(USDT, deposit_amount)])?;
    let proxy_addr = carrot_app.account().proxy()?;

    let balance: AssetsBalanceResponse = carrot_app.balance()?;
    let balance_usdc_before_withdraw = chain
        .bank_querier()
        .balance(&proxy_addr, Some(USDT.to_owned()))?
        .pop()
        .unwrap();
    let balance_usdt_before_withdraw = chain
        .bank_querier()
        .balance(&proxy_addr, Some(USDC.to_owned()))?
        .pop()
        .unwrap();

    // Withdraw some of the value
    let liquidity_amount: Uint128 = balance.balances[0].amount;
    let half_of_liquidity = liquidity_amount / Uint128::new(2);
    carrot_app.withdraw(Some(half_of_liquidity))?;

    let balance_usdc_after_half_withdraw = chain
        .bank_querier()
        .balance(&proxy_addr, Some(USDT.to_owned()))?
        .pop()
        .unwrap();
    let balance_usdt_after_half_withdraw = chain
        .bank_querier()
        .balance(&proxy_addr, Some(USDC.to_owned()))?
        .pop()
        .unwrap();

    assert!(balance_usdc_after_half_withdraw.amount > balance_usdc_before_withdraw.amount);
    assert!(balance_usdt_after_half_withdraw.amount > balance_usdt_before_withdraw.amount);

    // Withdraw rest of liquidity
    carrot_app.withdraw(None)?;
    let balance_usdc_after_full_withdraw = chain
        .bank_querier()
        .balance(chain.sender(), Some(USDT.to_owned()))?
        .pop()
        .unwrap();
    let balance_usdt_after_full_withdraw = chain
        .bank_querier()
        .balance(chain.sender(), Some(USDC.to_owned()))?
        .pop()
        .unwrap();

    assert!(balance_usdc_after_full_withdraw.amount > balance_usdc_after_half_withdraw.amount);
    assert!(balance_usdt_after_full_withdraw.amount > balance_usdt_after_half_withdraw.amount);
    Ok(())
}

#[test]
fn deposit_multiple_assets() -> anyhow::Result<()> {
    let (_, carrot_app) = setup_test_tube(false)?;

    let mut chain = carrot_app.get_chain().clone();
    let proxy_addr = carrot_app.account().proxy()?;
    let deposit_coins = vec![coin(234, USDC.to_owned()), coin(258, USDT.to_owned())];
    let deposit_assets = vec![AnsAsset::new(USDC, 234u128), AnsAsset::new(USDT, 258u128)];
    chain.add_balance(proxy_addr.to_string(), deposit_coins.clone())?;
    carrot_app.deposit(deposit_assets, None)?;

    Ok(())
}

#[test]
fn deposit_multiple_positions() -> anyhow::Result<()> {
    let (pool_id, carrot_app) = setup_test_tube(false)?;

    let new_strat = StrategyBase(vec![
        StrategyElementBase {
            yield_source: YieldSourceBase {
                asset_distribution: vec![
                    AssetShare {
                        asset: AssetEntry::new(USDT),
                        share: Decimal::percent(50),
                    },
                    AssetShare {
                        asset: AssetEntry::new(USDC),
                        share: Decimal::percent(50),
                    },
                ],
                params: YieldParamsBase::ConcentratedLiquidityPool(ConcentratedPoolParamsBase {
                    pool_id,
                    lower_tick: INITIAL_LOWER_TICK,
                    upper_tick: INITIAL_UPPER_TICK,
                    position_id: None,
                    _phantom: std::marker::PhantomData,
                }),
            },
            share: Decimal::percent(50),
        },
        StrategyElementBase {
            yield_source: YieldSourceBase {
                asset_distribution: vec![
                    AssetShare {
                        asset: AssetEntry::new(USDT),
                        share: Decimal::percent(50),
                    },
                    AssetShare {
                        asset: AssetEntry::new(USDC),
                        share: Decimal::percent(50),
                    },
                ],
                params: YieldParamsBase::ConcentratedLiquidityPool(ConcentratedPoolParamsBase {
                    pool_id,
                    lower_tick: 2 * INITIAL_LOWER_TICK,
                    upper_tick: 2 * INITIAL_UPPER_TICK,
                    position_id: None,
                    _phantom: std::marker::PhantomData,
                }),
            },
            share: Decimal::percent(50),
        },
    ]);
    let balances_before = query_balances(&carrot_app)?;
    let deposit_amount = 5_000u128;
    let mut chain = carrot_app.get_chain().clone();
    chain.add_balance(
        carrot_app.account().proxy()?.to_string(),
        coins(deposit_amount, USDT),
    )?;
    carrot_app.update_strategy(vec![AnsAsset::new(USDT, deposit_amount)], new_strat.clone())?;

    let balances_after = query_balances(&carrot_app)?;

    let slippage = Decimal::percent(4);
    assert!(
        balances_after
            > balances_before + (Uint128::from(deposit_amount) * (Decimal::one() - slippage))
    );
    Ok(())
}

#[test]
fn deposit_multiple_positions_with_empty() -> anyhow::Result<()> {
    let (pool_id, carrot_app) = setup_test_tube(false)?;

    let new_strat = StrategyBase(vec![
        StrategyElementBase {
            yield_source: YieldSourceBase {
                asset_distribution: vec![
                    AssetShare {
                        asset: AssetEntry::new(USDT),
                        share: Decimal::percent(50),
                    },
                    AssetShare {
                        asset: AssetEntry::new(USDC),
                        share: Decimal::percent(50),
                    },
                ],
                params: YieldParamsBase::ConcentratedLiquidityPool(ConcentratedPoolParamsBase {
                    pool_id,
                    lower_tick: INITIAL_LOWER_TICK,
                    upper_tick: INITIAL_UPPER_TICK,
                    position_id: None,
                    _phantom: std::marker::PhantomData,
                }),
            },
            share: Decimal::percent(50),
        },
        StrategyElementBase {
            yield_source: YieldSourceBase {
                asset_distribution: vec![
                    AssetShare {
                        asset: AssetEntry::new(USDT),
                        share: Decimal::percent(50),
                    },
                    AssetShare {
                        asset: AssetEntry::new(USDC),
                        share: Decimal::percent(50),
                    },
                ],
                params: YieldParamsBase::ConcentratedLiquidityPool(ConcentratedPoolParamsBase {
                    pool_id,
                    lower_tick: 2 * INITIAL_LOWER_TICK,
                    upper_tick: 2 * INITIAL_UPPER_TICK,
                    position_id: None,
                    _phantom: std::marker::PhantomData,
                }),
            },
            share: Decimal::percent(50),
        },
        StrategyElementBase {
            yield_source: YieldSourceBase {
                asset_distribution: vec![AssetShare {
                    asset: AssetEntry::new(USDT),
                    share: Decimal::percent(100),
                }],
                params: YieldParamsBase::Mars(MarsDepositParams {
                    asset: AssetEntry::new(USDT),
                }),
            },
            share: Decimal::percent(0),
        },
    ]);
    let balances_before = query_balances(&carrot_app)?;
    let deposit_amount = 5_000u128;

    let mut chain = carrot_app.get_chain().clone();
    chain.add_balance(
        carrot_app.account().proxy()?.to_string(),
        coins(deposit_amount, USDT),
    )?;
    carrot_app.update_strategy(vec![AnsAsset::new(USDT, deposit_amount)], new_strat.clone())?;

    // Do the deposit
    let balances_after = query_balances(&carrot_app)?;

    println!("{balances_before} --> {balances_after}");
    let slippage = Decimal::percent(4);
    assert!(
        balances_after
            > balances_before + (Uint128::from(deposit_amount) * (Decimal::one() - slippage))
    );
    Ok(())
}

#[test]
fn create_position_on_instantiation() -> anyhow::Result<()> {
    let (_, carrot_app) = setup_test_tube(true)?;

    let position = carrot_app.positions()?;
    assert!(!position.positions.is_empty());

    let balance = carrot_app.balance()?;
    assert!(balance.total_value > Uint128::from(20_000u128) * Decimal::percent(99));
    Ok(())
}

// #[test]
// fn withdraw_after_user_withdraw_liquidity_manually() -> anyhow::Result<()> {
//     let (_, carrot_app) = setup_test_tube(true)?;
//     let chain = carrot_app.get_chain().clone();

//     let position: PositionResponse = carrot_app.position()?;
//     let position_id = position.position.unwrap().position_id;

//     let test_tube = chain.app.borrow();
//     let cl = ConcentratedLiquidity::new(&*test_tube);
//     let position_breakdown = cl
//         .query_position_by_id(&PositionByIdRequest { position_id })?
//         .position
//         .unwrap();
//     let position = position_breakdown.position.unwrap();

//     cl.withdraw_position(
//         MsgWithdrawPosition {
//             position_id: position.position_id,
//             sender: chain.sender().to_string(),
//             liquidity_amount: position.liquidity,
//         },
//         &chain.sender,
//     )?;

//     // Ensure it errors
//     carrot_app.withdraw_all().unwrap_err();

//     // Ensure position deleted
//     let position_not_found = cl
//         .query_position_by_id(&PositionByIdRequest { position_id })
//         .unwrap_err();
//     assert!(position_not_found
//         .to_string()
//         .contains("position not found"));
//     Ok(())
// }

// #[test]
// fn deposit_slippage() -> anyhow::Result<()> {
//     let (_, carrot_app) = setup_test_tube(false)?;

//     let deposit_amount = 5_000;
//     let max_fee = Uint128::new(deposit_amount).mul_floor(Decimal::percent(3));
//     // Create position
//     create_position(
//         &carrot_app,
//         coins(deposit_amount, USDT.to_owned()),
//         coin(1_000_000, USDT.to_owned()),
//         coin(1_000_000, USDC.to_owned()),
//     )?;

//     // Do the deposit of asset0 with incorrect belief_price1
//     let e = carrot_app
//         .deposit(
//             vec![coin(deposit_amount, USDT.to_owned())],
//             None,
//             Some(Decimal::zero()),
//             None,
//         )
//         .unwrap_err();
//     assert!(e.to_string().contains("exceeds max spread limit"));

//     // Do the deposit of asset1 with incorrect belief_price0
//     let e = carrot_app
//         .deposit(
//             vec![coin(deposit_amount, USDC.to_owned())],
//             Some(Decimal::zero()),
//             None,
//             None,
//         )
//         .unwrap_err();
//     assert!(e.to_string().contains("exceeds max spread limit"));

//     // Do the deposits of asset0 with correct belief_price
//     carrot_app.deposit(
//         vec![coin(deposit_amount, USDT.to_owned())],
//         None,
//         Some(Decimal::one()),
//         Some(Decimal::percent(10)),
//     )?;
//     // Do the deposits of asset1 with correct belief_price
//     carrot_app.deposit(
//         vec![coin(deposit_amount, USDT.to_owned())],
//         Some(Decimal::one()),
//         None,
//         Some(Decimal::percent(10)),
//     )?;

//     // Check almost everything landed
//     let balance: AssetsBalanceResponse = carrot_app.balance()?;
//     let sum = balance
//         .balances
//         .iter()
//         .fold(Uint128::zero(), |acc, e| acc + e.amount);
//     assert!(sum.u128() > (deposit_amount - max_fee.u128()) * 3);
//     Ok(())
// }
