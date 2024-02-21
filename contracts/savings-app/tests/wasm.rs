use abstract_testing::OWNER;
use cw_orch::prelude::*;
use carrot_app::{contract::APP_ID, AppInterface};

#[test]
fn successful_wasm() {
    // Create a sender
    let sender = Addr::unchecked(OWNER);
    // Create the mock
    let mock = Mock::new(sender);

    // Construct the counter interface
    let contract = AppInterface::new(APP_ID, mock);
    // Panics if no path to a .wasm file is found
    contract.wasm();
}
