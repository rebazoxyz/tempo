use std::{net::SocketAddr, sync::Arc};

use clap::Parser;
use commonware_cryptography::Signer;
use commonware_p2p::authenticated::discovery;
use commonware_runtime::{Handle, Metrics as _};
use eyre::{WrapErr as _, eyre};
use futures_util::{FutureExt as _, future::try_join_all};
use reth::payload::PayloadBuilderHandle;
use reth_chainspec::ChainSpec;
use reth_node_builder::{BeaconConsensusEngineHandle, NodeHandle, NodeTypes};
use reth_node_ethereum::EthereumNode;

use crate::config::{
    BACKFILL_BY_DIGEST_CHANNE_IDENTL, BACKFILL_QUOTA, BLOCKS_FREEZER_TABLE_INITIAL_SIZE_BYTES,
    BROADCASTER_CHANNEL_IDENT, BROADCASTER_LIMIT, FETCH_TIMEOUT,
    FINALIZED_FREEZER_TABLE_INITIAL_SIZE_BYTES, LEADER_TIMEOUT, MAX_FETCH_SIZE_BYTES,
    NOTARIZATION_TIMEOUT, NUMBER_CONCURRENT_FETCHES, NUMBER_MAX_FETCHES, NUMBER_OF_VIEWS_TO_TRACK,
    NUMBER_OF_VIEWS_UNTIL_LEADER_SKIP, PENDING_CHANNEL_IDENT, PENDING_LIMIT,
    RECOVERED_CHANNEL_IDENT, RECOVERED_LIMIT, RESOLVER_CHANNEL_IDENT, RESOLVER_LIMIT,
    TIME_TO_NULLIFY_RETRY,
};
use tempo_commonware_node_cryptography::{PrivateKey, PublicKey};

/// Parses command line args and launches the node.
///
/// This function will spawn a tokio runtime and run the node on it.
/// It will block until the node finishes.
pub fn run() -> eyre::Result<()> {
    let args = Args::parse();
    args.run()
}

#[derive(Debug, clap::Parser)]
#[command(author, version, about = "runs a tempo node")]
pub struct Args {
    /// Additional filter directives to filter out unwanted tracing events or spans.
    ///
    /// Because the tracing subscriber emits events when methods are entered,
    /// by default the filter directives quiet `net` and `reth_ecies` because
    /// they are very noisy. For more information on how to specify filter
    /// directives see the tracing-subscriber documentation [1].
    ///
    /// 1: https://docs.rs/tracing-subscriber/0.3.19/tracing_subscriber/filter/struct.EnvFilter.html
    #[clap(
        long,
        value_name = "DIRECTIVE",
        default_value = "info,net=warn,reth_ecies=warn"
    )]
    filter_directives: String,

    // XXX: Don't use any extra subcmds for now. It'd just be confusing until we figure out a
    // good way to launch nodes that follows reth's convention.
    #[command(flatten)]
    inner: reth_cli_commands::NodeCommand<crate::chainspec::Parser, ConsensusSpecificArgs>,
}

/// Args for setting up the consensuns-part of the node (everything non-reth).
#[derive(Clone, Debug, clap::Args)]
struct ConsensusSpecificArgs {
    #[clap(long, value_name = "FILE")]
    consensus_config: camino::Utf8PathBuf,
}

impl Args {
    pub fn run(self) -> eyre::Result<()> {
        use commonware_runtime::Runner as _;
        use tracing_subscriber::fmt;
        use tracing_subscriber::fmt::format::FmtSpan;
        use tracing_subscriber::prelude::*;

        let env_filter = tracing_subscriber::EnvFilter::builder()
            .parse(&self.filter_directives)
            .wrap_err("failed to parse provided filter directives")?;
        tracing_subscriber::registry()
            .with(fmt::layer().with_span_events(FmtSpan::NEW))
            .with(env_filter)
            .init();

        let consensus_config = tempo_commonware_node_config::Config::from_file(
            &self.inner.ext.consensus_config,
        )
        .wrap_err_with(|| {
            format!(
                "failed parsing consensus config from provided argument `{}`",
                self.inner.ext.consensus_config
            )
        })?;

        let runtime_config = commonware_runtime::tokio::Config::default()
            .with_tcp_nodelay(Some(true))
            .with_worker_threads(consensus_config.worker_threads)
            .with_storage_directory(&consensus_config.storage_directory)
            .with_catch_panics(true);

        let executor = commonware_runtime::tokio::Runner::new(runtime_config);
        executor.start(move |context| async move {
            // TODO: tuck this glue + node launching logic into a helper function.
            // The type complexity naming the `reth_node_builder::NodeHandle` is pretty
            // annoying, so we leave it in-line for now.
            let crate::reth_glue::NodeWithGlue {
                task_manager: _task_manager,
                builder,
            } = crate::reth_glue::NodeWithGlue::from_node_command(self.inner)
                .wrap_err("failed initializing reth node")?;

            let execution_node = builder
                .node(crate::execution::Node::new())
                // TODO: add tables to store information in the execution node's db?
                // An alternative would be to make use of the commonwarexyz storage system.
                // Probably makes sense to have it all in one spot.
                // .try_apply(|mut ctx| ctx.db_mut().create_tables_for::<Tables>())
                // .wrap_err(
                //     "failed initializing consensus-specific database tables on execution node",
                // )?
                .launch()
                .await
                .wrap_err("launching execution node failed")?;

            let NodeHandle {
                node,
                node_exit_future,
            } = execution_node;

            let chainspec = node.chain_spec();
            let execution_engine = node.add_ons_handle.beacon_engine_handle.clone();
            let execution_payload_builder = node.payload_builder_handle.clone();

            let ConsensusStack {
                network,
                consensus_engine,
            } = launch_consensus_stack(
                &context,
                &consensus_config,
                chainspec,
                execution_engine,
                execution_payload_builder,
            )
            .await
            .wrap_err("failed to initialize consensus stack")?;

            try_join_all(vec![
                async move { network.await.wrap_err("network failed") }.boxed(),
                async move {
                    consensus_engine
                        .await
                        .wrap_err("consensus engine failed")
                        .flatten()
                }
                .boxed(),
                async move { node_exit_future.await.wrap_err("execution node failed") }.boxed(),
            ])
            .await
            .map(|_| ())
        })
    }
}

struct ConsensusStack {
    network: Handle<()>,
    consensus_engine: Handle<eyre::Result<()>>,
}

async fn launch_consensus_stack(
    context: &commonware_runtime::tokio::Context,
    config: &tempo_commonware_node_config::Config,
    chainspec: Arc<ChainSpec>,
    execution_engine: BeaconConsensusEngineHandle<<EthereumNode as NodeTypes>::Payload>,
    execution_payload_builder: PayloadBuilderHandle<<EthereumNode as NodeTypes>::Payload>,
) -> eyre::Result<ConsensusStack> {
    let (mut network, mut oracle) =
        instantiate_network(context, config).wrap_err("failed to start network")?;

    oracle
        .register(0, config.peers.keys().cloned().collect())
        .await;
    let message_backlog = config.message_backlog;
    let pending = network.register(PENDING_CHANNEL_IDENT, PENDING_LIMIT, message_backlog);
    let recovered = network.register(RECOVERED_CHANNEL_IDENT, RECOVERED_LIMIT, message_backlog);
    let resolver = network.register(RESOLVER_CHANNEL_IDENT, RESOLVER_LIMIT, message_backlog);
    let broadcaster = network.register(
        BROADCASTER_CHANNEL_IDENT,
        BROADCASTER_LIMIT,
        message_backlog,
    );
    let backfill = network.register(
        BACKFILL_BY_DIGEST_CHANNE_IDENTL,
        BACKFILL_QUOTA,
        message_backlog,
    );

    let consensus_engine = crate::consensus::engine::Builder {
        context: context.with_label("engine"),

        fee_recipient: config.fee_recipient,
        chainspec,
        execution_engine,
        execution_payload_builder,

        blocker: oracle,
        // TODO: Set this through config?
        partition_prefix: "engine".into(),
        blocks_freezer_table_initial_size: BLOCKS_FREEZER_TABLE_INITIAL_SIZE_BYTES,
        finalized_freezer_table_initial_size: FINALIZED_FREEZER_TABLE_INITIAL_SIZE_BYTES,
        signer: config.signer.clone(),
        polynomial: config.polynomial.clone(),
        share: config.share.clone(),
        participants: config.peers.keys().cloned().collect::<Vec<_>>(),
        mailbox_size: config.mailbox_size,
        backfill_quota: BACKFILL_QUOTA,
        deque_size: config.deque_size,

        leader_timeout: LEADER_TIMEOUT,
        notarization_timeout: NOTARIZATION_TIMEOUT,
        nullify_retry: TIME_TO_NULLIFY_RETRY,
        fetch_timeout: FETCH_TIMEOUT,
        activity_timeout: NUMBER_OF_VIEWS_TO_TRACK,
        skip_timeout: NUMBER_OF_VIEWS_UNTIL_LEADER_SKIP,
        max_fetch_count: NUMBER_MAX_FETCHES,
        max_fetch_size: MAX_FETCH_SIZE_BYTES,
        fetch_concurrent: NUMBER_CONCURRENT_FETCHES,
        fetch_rate_per_peer: RESOLVER_LIMIT,
        // indexer: Option<TIndexer>,
    }
    .init()
    .await;

    Ok(ConsensusStack {
        network: network.start(),
        consensus_engine: consensus_engine.start(
            pending,
            recovered,
            resolver,
            broadcaster,
            backfill,
        ),
    })
}

fn instantiate_network(
    context: &commonware_runtime::tokio::Context,
    config: &tempo_commonware_node_config::Config,
) -> eyre::Result<(
    discovery::Network<commonware_runtime::tokio::Context, PrivateKey>,
    discovery::Oracle<commonware_runtime::tokio::Context, PublicKey>,
)> {
    use commonware_p2p::authenticated::discovery;
    use std::net::Ipv4Addr;

    let my_public_key = config.signer.public_key();
    let my_ip = config.peers.get(&config.signer.public_key()).ok_or_else(||
        eyre!("peers entry does not contain an entry for this node's public key (generated from the signer key): `{my_public_key}`")
    )?.ip();

    let bootstrappers = config.bootstrappers().collect();

    // TODO: Find out why `union_unique` should be used at all. This is the only place
    // where `NAMESPACE` is used at all. We follow alto's example for now.
    let p2p_namespace = commonware_utils::union_unique(crate::config::NAMESPACE, b"_P2P");
    let p2p_cfg = discovery::Config {
        mailbox_size: config.mailbox_size,
        ..discovery::Config::aggressive(
            config.signer.clone(),
            &p2p_namespace,
            // TODO: should the listen addr be restricted to ipv4?
            SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), config.listen_port),
            SocketAddr::new(my_ip, config.listen_port),
            bootstrappers,
            crate::config::MAX_MESSAGE_SIZE_BYTES,
        )
    };

    Ok(discovery::Network::new(
        context.with_label("network"),
        p2p_cfg,
    ))
}
