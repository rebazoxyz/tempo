//! Contains [`TempoNode`] definition of the execution node.

use reth_chainspec::ChainSpec;
use reth_ethereum_primitives::EthPrimitives;
use reth_node_builder::{
    BuilderContext, FullNodeTypes, NodeComponentsBuilder, NodeTypes, PayloadBuilderConfig as _,
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
};
use reth_node_ethereum::{
    EthEngineTypes, EthEvmConfig, EthereumAddOns, EthereumEngineValidatorBuilder,
    EthereumEthApiBuilder, EthereumNetworkBuilder, EthereumPoolBuilder,
};
use reth_provider::EthStorage;
use reth_trie_db::MerklePatriciaTrie;

pub mod consensus_validator;
pub mod evm;

#[derive(Debug, Clone)]
pub struct Node(());

impl Node {
    pub fn new() -> Self {
        Self(())
    }
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExecutorBuilder(());

impl<N: FullNodeTypes<Types = Node>> reth_node_builder::components::ExecutorBuilder<N>
    for ExecutorBuilder
{
    type EVM = EthEvmConfig<ChainSpec, crate::execution::evm::Factory>;

    async fn build_evm(self, ctx: &BuilderContext<N>) -> eyre::Result<Self::EVM> {
        Ok(EthEvmConfig::new_with_evm_factory(
            ctx.chain_spec(),
            crate::execution::evm::Factory::default(),
        )
        .with_extra_data(ctx.payload_builder_config().extra_data_bytes()))
    }
}

impl NodeTypes for Node {
    type Primitives = EthPrimitives;
    type ChainSpec = ChainSpec;
    type StateCommitment = MerklePatriciaTrie;
    type Storage = EthStorage;
    type Payload = EthEngineTypes;
}

impl<TNodeTypes> reth_node_builder::Node<TNodeTypes> for Node
where
    TNodeTypes: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        TNodeTypes,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<reth_node_ethereum::EthereumPayloadBuilder>,
        EthereumNetworkBuilder,
        ExecutorBuilder,
        consensus_validator::Builder,
    >;

    type AddOns = EthereumAddOns<
        reth_node_builder::NodeAdapter<
            TNodeTypes,
            <Self::ComponentsBuilder as NodeComponentsBuilder<TNodeTypes>>::Components,
        >,
        EthereumEthApiBuilder,
        EthereumEngineValidatorBuilder,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        ComponentsBuilder::default()
            .node_types::<TNodeTypes>()
            .pool(EthereumPoolBuilder::default())
            .executor(ExecutorBuilder::default())
            .payload(BasicPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .consensus(consensus_validator::Builder::new())
    }

    fn add_ons(&self) -> Self::AddOns {
        EthereumAddOns::default()
    }
}
