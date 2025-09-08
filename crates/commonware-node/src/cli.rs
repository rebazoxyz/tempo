use std::{net::SocketAddr, sync::Arc};

use clap::Parser;
use commonware_cryptography::Signer;
use commonware_p2p::authenticated::discovery;
use commonware_runtime::{Handle, Metrics as _};
use eyre::{WrapErr as _, eyre};
use futures_util::{FutureExt as _, future::try_join_all};
use reth_ethereum_cli;
use reth_node_builder::{
    FullNode, FullNodeComponents, FullNodeTypes, NodeHandle, NodePrimitives, NodeTypes,
    PayloadTypes, rpc::RethRpcAddOns,
};
use reth_node_ethereum::EthEvmConfig;
use reth_provider::DatabaseProviderFactory;
use tempo_chainspec::spec::{TempoChainSpec, TempoChainSpecParser};
use tempo_faucet::faucet::{TempoFaucetExt, TempoFaucetExtApiServer as _};
use tempo_node::{args::TempoArgs, node::TempoNode};

use crate::{
    config::{
        BACKFILL_BY_DIGEST_CHANNE_IDENTL, BACKFILL_QUOTA, BLOCKS_FREEZER_TABLE_INITIAL_SIZE_BYTES,
        BROADCASTER_CHANNEL_IDENT, BROADCASTER_LIMIT, FETCH_TIMEOUT,
        FINALIZED_FREEZER_TABLE_INITIAL_SIZE_BYTES, LEADER_TIMEOUT, MAX_FETCH_SIZE_BYTES,
        NOTARIZATION_TIMEOUT, NUMBER_CONCURRENT_FETCHES, NUMBER_MAX_FETCHES,
        NUMBER_OF_VIEWS_TO_TRACK, NUMBER_OF_VIEWS_UNTIL_LEADER_SKIP, PENDING_CHANNEL_IDENT,
        PENDING_LIMIT, RECOVERED_CHANNEL_IDENT, RECOVERED_LIMIT, RESOLVER_CHANNEL_IDENT,
        RESOLVER_LIMIT, TIME_TO_NULLIFY_RETRY,
    },
    reth_glue::ContextEnrichedArgs,
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
    // TODO: look into how commonware and reth set up their logging/tracing/metrics.
    // reth has `LogArgs` for example, which we are altogether ignoring for now.
    #[clap(
        long,
        value_name = "DIRECTIVE",
        default_value = "info,net=warn,reth_ecies=warn"
    )]
    filter_directives: String,

    #[command(subcommand)]
    inner: reth_ethereum_cli::interface::Commands<TempoChainSpecParser, TempoSpecificArgs>,
}

/// Args for setting up the consensuns-part of the node (everything non-reth).
#[derive(Clone, Debug, clap::Args)]
pub(crate) struct TempoSpecificArgs {
    #[clap(long, value_name = "FILE")]
    consensus_config: camino::Utf8PathBuf,

    #[command(flatten)]
    faucet_args: tempo_faucet::args::FaucetArgs,
}

impl Args {
    pub fn run(self) -> eyre::Result<()> {
        use tracing_subscriber::{fmt, fmt::format::FmtSpan, prelude::*};

        let env_filter = tracing_subscriber::EnvFilter::builder()
            .parse(&self.filter_directives)
            .wrap_err("failed to parse provided filter directives")?;
        tracing_subscriber::registry()
            .with(fmt::layer().with_span_events(FmtSpan::NEW))
            .with(env_filter)
            .init();

        let mut runtime_config = commonware_runtime::tokio::Config::default();
        if let reth_ethereum_cli::interface::Commands::Node(node_cmd) = &self.inner {
            let consensus_config =
                tempo_commonware_node_config::Config::from_file(&node_cmd.ext.consensus_config)
                    .wrap_err_with(|| {
                        format!(
                            "failed parsing consensus config from provided argument `{}`",
                            node_cmd.ext.consensus_config
                        )
                    })?;
            runtime_config = runtime_config
                .with_tcp_nodelay(Some(true))
                .with_worker_threads(consensus_config.worker_threads)
                .with_storage_directory(&consensus_config.storage_directory)
                .with_catch_panics(true);
        };

        let runner = commonware_runtime::tokio::Runner::new(runtime_config);

        let components = |spec: Arc<TempoChainSpec>| {
            (
                EthEvmConfig::new(spec.clone()),
                tempo_consensus::TempoConsensus::new(spec),
            )
        };
        crate::reth_glue::with_runner_and_components::<TempoChainSpecParser, TempoNode>(
            runner,
            self.inner,
            components,
            async move |builder, args| {
                let ContextEnrichedArgs {
                    context: Some(context),
                    args,
                } = args
                else {
                    panic!(
                        "runtime context must have been passed in via the reth-commonware glue; this is a bug"
                    );
                };

                let consensus_config =
                    tempo_commonware_node_config::Config::from_file(&args.consensus_config)
                        .wrap_err_with(|| {
                            format!(
                                "failed parsing consensus config from provided argument `{}`",
                                args.consensus_config
                            )
                        })?;

                let NodeHandle {
                    node,
                    node_exit_future,
                } = builder
                    // TODO: just a placeholder until we can line up the types.
                    //
                    // Should this `TempoNode` even be aware of consensus specific args?
                    // It's odd that all of the arguments are only ever applied
                    // *outside* of it.
                    .node(TempoNode::new(TempoArgs {
                        no_consensus: false,
                        malachite_args: Default::default(),
                        faucet_args: args.faucet_args.clone(),
                    }))
                    .extend_rpc_modules(move |ctx| {
                        if args.faucet_args.enabled {
                            let txpool = ctx.pool().clone();
                            let ext = TempoFaucetExt::new(
                                txpool,
                                args.faucet_args.address(),
                                args.faucet_args.amount(),
                                args.faucet_args.provider(),
                            );

                            ctx.modules.merge_configured(ext.into_rpc())?;
                        }
                        Ok(())
                    })
                    // TODO: figure out if commonware information should be
                    // stored in reth or not.
                    //
                    // The commented out code is a remnant of malachite, but
                    // commonware keeps its own storage next to reth.
                    // .apply(|mut ctx| {
                    //     let db = ctx.db_mut();
                    //     db.create_tables_for::<reth_malachite::store::tables::Tables>()
                    //         .expect("Failed to create consensus tables");
                    //     ctx
                    // })
                    .launch()
                    .await
                    .wrap_err("launching execution node failed")?;

                let ConsensusStack {
                    network,
                    consensus_engine,
                } = launch_consensus_stack(
                    &context,
                    &consensus_config,
                    node.clone(),
                    // chainspec,
                    // execution_engine,
                    // execution_payload_builder,
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
            },
        )
    }
}

struct ConsensusStack {
    network: Handle<()>,
    consensus_engine: Handle<eyre::Result<()>>,
}

// async fn launch_consensus_stack<TNodeTypes: NodeTypes>(
//     context: &commonware_runtime::tokio::Context,
//     config: &tempo_commonware_node_config::Config,
//     chainspec: Arc<TempoChainSpec>,
//     execution_engine: ConsensusEngineHandle<TNodeTypes::Payload>,
//     execution_payload_builder: PayloadBuilderHandle<TNodeTypes::Payload>,
// ) -> eyre::Result<ConsensusStack> {
async fn launch_consensus_stack<TFullNodeComponents, TRethRpcAddons>(
    context: &commonware_runtime::tokio::Context,
    config: &tempo_commonware_node_config::Config,
    execution_node: FullNode<TFullNodeComponents, TRethRpcAddons>,
    // chainspec: Arc<TempoChainSpec>,
    // execution_engine: ConsensusEngineHandle<TNodeTypes::Payload>,
    // execution_payload_builder: PayloadBuilderHandle<TNodeTypes::Payload>,
) -> eyre::Result<ConsensusStack>
where
    TFullNodeComponents: FullNodeComponents,
    TFullNodeComponents::Types: NodeTypes,
    <TFullNodeComponents::Types as NodeTypes>::Payload: PayloadTypes<
            PayloadAttributes = alloy_rpc_types_engine::PayloadAttributes,
            ExecutionData = alloy_rpc_types_engine::ExecutionData,
            BuiltPayload = reth_ethereum_engine_primitives::EthBuiltPayload,
        >,
    <TFullNodeComponents::Types as NodeTypes>::Primitives: NodePrimitives<
            Block = reth_ethereum_primitives::Block,
            BlockHeader = alloy_consensus::Header,
        >,
    <<TFullNodeComponents as FullNodeTypes>::Provider as DatabaseProviderFactory>::ProviderRW: Send,
    TRethRpcAddons: RethRpcAddOns<TFullNodeComponents> + 'static,
{
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

        execution_node,
        // chainspec: node.chain_spec(),
        // execution_engine: node.add_ons_handle.consensus_engine_handle().clone(),
        // execution_payload_builder: node.payload_builder_handle.clone(),
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
    .try_init()
    .await
    .wrap_err("failed initializing consensus engine")?;

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
