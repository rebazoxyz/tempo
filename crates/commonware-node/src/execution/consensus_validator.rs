use std::sync::Arc;

use reth_chainspec::ChainSpec;
use reth_consensus::noop::NoopConsensus;
use reth_node_builder::{FullNodeTypes, NodeTypes, components::ConsensusBuilder};

pub struct Builder(());

impl Builder {
    pub fn new() -> Self {
        Self(())
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl<TNodeTypes> ConsensusBuilder<TNodeTypes> for Builder
where
    TNodeTypes: FullNodeTypes,
    TNodeTypes::Types: NodeTypes<ChainSpec = ChainSpec>,
{
    // TODO: Replace this by an actual consensus validator.
    type Consensus = Arc<NoopConsensus>;

    async fn build_consensus(
        self,
        _ctx: &reth_node_builder::BuilderContext<TNodeTypes>,
    ) -> eyre::Result<Self::Consensus> {
        Ok(NoopConsensus::arc())
    }
}
