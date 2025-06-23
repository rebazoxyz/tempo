use clap::{Args, Parser};
use reth_malachite::app::node::MalachiteNode;
use reth_malachite::app::{Config, Genesis, State};
use reth_malachite::cli::{Cli, MalachiteChainSpecParser};
use reth_malachite::context::MalachiteContext;
use reth_malachite::types::Address;
use reth::builder::NodeHandle;

/// No Additional arguments
#[derive(Debug, Clone, Copy, Default, Args)]
#[non_exhaustive]
struct NoArgs;

fn main() -> eyre::Result<()> {
    reth_cli_util::sigsegv_handler::install();

    let ctx = MalachiteContext::default();
    let config = Config::new();
    let genesis = Genesis::new("1".to_string());
    let address = Address::new([0; 20]);

    Cli::<MalachiteChainSpecParser, NoArgs>::parse().run(|builder, _: NoArgs| async move {
        let state = State::new(ctx, config, genesis, address);

        let malachite_node = MalachiteNode::new(state);
        let NodeHandle {
            node,
            node_exit_future,
        } = builder.node(malachite_node).launch().await?;

        let _engine_handle = node.add_ons_handle.beacon_engine_handle;

        node_exit_future.await
    })?;

    Ok(())
}
