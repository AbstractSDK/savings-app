use abstract_app::objects::namespace::ABSTRACT_NAMESPACE;
use abstract_client::Namespace;
use abstract_interface::Abstract;
use cw_orch::{
    anyhow,
    contract::Deploy,
    daemon::{networks::OSMOSIS_1, Daemon, DaemonBuilder},
    prelude::Stargate,
    tokio::runtime::Runtime,
};
use dotenv::dotenv;

use carrot_app::AppInterface;

fn main() -> anyhow::Result<()> {
    todo!();
    dotenv().ok();
    env_logger::init();
    let chain = OSMOSIS_1;
    let rt = Runtime::new()?;
    let daemon = DaemonBuilder::default()
        .chain(chain)
        .handle(rt.handle())
        .build()?;

    let abstr = Abstract::load_from(daemon)?;
    // daemon.commit_any(msgs, memo)
    // let abstr = abstract_client::AbstractClient::new(daemon)?;

    // let publisher = abstr
    //     .publisher_builder(Namespace::new(ABSTRACT_NAMESPACE)?)
    //     .build()?;

    // publisher.publish_app::<AppInterface<Daemon>>()?;
    Ok(())
}
