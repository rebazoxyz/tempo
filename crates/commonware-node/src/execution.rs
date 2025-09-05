//! Contains definitions to pass to the execution layer / reth.

use reth_chainspec::{ChainSpec, EthereumHardforks, Hardforks};
use reth_consensus::{Consensus, ConsensusError, FullConsensus, HeaderValidator};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::eth::spec::EthExecutorSpec;
use reth_execution_types::BlockExecutionResult;
use reth_node_builder::{Block, BuilderContext, FullNodeTypes, components::ConsensusBuilder};
use reth_primitives_traits::{SealedBlock, SealedHeader};
use std::sync::Arc;

#[derive(Debug, Clone)]
#[expect(
    dead_code,
    reason = "for now only exists to line up arguments in crate::reth_glue::with_runner_and_components"
)]
pub struct TempoConsensus<C = ChainSpec> {
    chain_spec: Arc<C>,
}

impl<C> TempoConsensus<C> {
    pub fn new(chain_spec: Arc<C>) -> Self {
        Self { chain_spec }
    }
}

impl Default for TempoConsensus {
    fn default() -> Self {
        Self {
            chain_spec: Arc::new(ChainSpec::default()),
        }
    }
}

impl<H, C> HeaderValidator<H> for TempoConsensus<C>
where
    C: std::fmt::Debug + Send + Sync,
{
    fn validate_header(&self, _header: &SealedHeader<H>) -> Result<(), ConsensusError> {
        // For now, return Ok - implement validation logic here
        Ok(())
    }

    fn validate_header_against_parent(
        &self,
        _header: &SealedHeader<H>,
        _parent: &SealedHeader<H>,
    ) -> Result<(), ConsensusError> {
        // For now, return Ok - implement validation logic here
        Ok(())
    }
}

impl<B, C> Consensus<B> for TempoConsensus<C>
where
    B: Block,
    C: std::fmt::Debug + Send + Sync,
{
    type Error = ConsensusError;

    fn validate_body_against_header(
        &self,
        _body: &B::Body,
        _header: &SealedHeader<B::Header>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn validate_block_pre_execution(&self, _block: &SealedBlock<B>) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<N, C> FullConsensus<N> for TempoConsensus<C>
where
    N: reth_primitives_traits::NodePrimitives,
    C: std::fmt::Debug + Send + Sync,
{
    fn validate_block_post_execution(
        &self,
        _block: &reth_primitives_traits::RecoveredBlock<N::Block>,
        _result: &BlockExecutionResult<N::Receipt>,
    ) -> Result<(), ConsensusError> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct TempoConsensusBuilder;

impl<Node> ConsensusBuilder<Node> for TempoConsensusBuilder
where
    Node: FullNodeTypes<
        Types: reth_node_builder::NodeTypes<
            ChainSpec: Hardforks + EthereumHardforks + EthExecutorSpec,
            Primitives = EthPrimitives,
        >,
    >,
{
    type Consensus = Arc<TempoConsensus<<Node::Types as reth_node_builder::NodeTypes>::ChainSpec>>;

    async fn build_consensus(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::Consensus> {
        Ok(Arc::new(TempoConsensus::new(ctx.chain_spec())))
    }
}

impl Default for TempoConsensusBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TempoConsensusBuilder {
    pub fn new() -> Self {
        Self
    }
}
