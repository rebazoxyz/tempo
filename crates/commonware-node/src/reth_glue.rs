//! Glue to run a reth node inside a commonware runtime context.
//!
//! None of the code here is actually specific to commonware-xyz. It just so happens
//! that in order to spawn a reth instance, all that is needed is a
//! [`reth_tasks::TaskManager`] instance, which can be done from inside any tokio
//! runtime instance.
//!
//! # Why this exists
//!
//! The peculiarity of commonwarexyz is that it fully wraps a tokio runtime and passes
//! an abstracted `S: Spawner` into all code that needs to spawn tasks. So rather than
//! using `tokio::runtime::Handle::spawn` it uses `<S as Spawner>::spawn` to run async
//! tasks on the runtime while tracking the amount and (named) context of all tasks.
//!
//! In a similar manner, reth's primary way of launching nodes is through a
//! [`reth_cli::CliRunner`], which also takes possession of a tokio runtime.
//! However, it then uses a tokio runtime's *handle* to construct a
//! `[reth_tasks::TaskManager]` and [`reth_tasks::Executor`], passing the latter
//! to through its stack to spawn tasks (and track tasks, etc).

use std::sync::Arc;

use eyre::WrapErr as _;
use reth_chainspec::ChainSpec;
use reth_cli_commands::NodeCommand;
use reth_db::{DatabaseEnv, init_db};
use reth_node_builder::{NodeBuilder, NodeConfig, WithLaunchContext};
use reth_tasks::TaskManager;

pub struct NodeWithGlue {
    pub task_manager: TaskManager,
    // To be honest, we don't need the builder. But the type complexity of the
    // actual node doesn't seem worth it putting it in here.
    pub builder: WithLaunchContext<NodeBuilder<Arc<DatabaseEnv>, ChainSpec>>,
}

impl NodeWithGlue {
    pub fn from_node_command<TNodeCmdExt>(
        node_cmd: NodeCommand<crate::chainspec::Parser, TNodeCmdExt>,
    ) -> eyre::Result<Self>
    where
        TNodeCmdExt: clap::Args + std::fmt::Debug,
    {
        // XXX: The body of this function was copied from
        // `reth_cli_commands::NodeCommand::execute` to return a node builder but
        // without the launcher logic.

        let NodeCommand {
            datadir,
            config,
            chain,
            metrics,
            instance,
            with_unused_ports,
            network,
            rpc,
            txpool,
            builder,
            debug,
            db,
            dev,
            pruning,
            engine,
            era,
            ext: _ext,
        } = node_cmd;

        // set up node config
        let mut node_config = NodeConfig {
            datadir,
            config,
            chain,
            metrics,
            instance,
            network,
            rpc,
            txpool,
            builder,
            debug,
            db,
            dev,
            pruning,
            engine,
            era,
        };

        let data_dir = node_config.datadir();
        let db_path = data_dir.db();

        let database = Arc::new(
            init_db(db_path.clone(), db.database_args())
                .wrap_err_with(|| {
                    format!("failed initializing database at `{}`", db_path.display())
                })?
                .with_metrics(),
        );

        if with_unused_ports {
            node_config = node_config.with_unused_ports();
        }

        let task_manager = TaskManager::current();

        let builder = NodeBuilder::new(node_config)
            .with_database(database)
            .with_launch_context(task_manager.executor());

        Ok(Self {
            task_manager,
            builder,
        })
    }
}
