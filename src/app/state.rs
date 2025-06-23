use bytes::Bytes;
use eyre::Result;
use crate::context::{
    BasePeerAddress, BasePeerSet, BaseProposalPart, BaseValue, MalachiteContext, ValueIdWrapper,
};
use crate::height::Height;
use crate::provider::Ed25519Provider;
use crate::types::Address;
use crate::utils::seed_from_address;
use malachitebft_app_channel::app::streaming::StreamMessage;
use malachitebft_app_channel::app::types::{
    LocallyProposedValue, PeerId as MalachitePeerId, ProposedValue,
};
use malachitebft_core_types::{CommitCertificate, Height as HeightTrait, Round, VoteExtensions};
use rand::SeedableRng;
use rand::rngs::StdRng;
use reth::payload::PayloadBuilderHandle;
use reth_ethereum_engine_primitives::EthPayloadTypes;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::info;

/// Represents the internal state of the application node
/// Contains information about current height, round, proposals and blocks
#[derive(Debug, Clone)]
pub struct State {
    pub ctx: MalachiteContext,
    pub config: Config,
    pub genesis: Genesis,
    pub address: Address,
    pub current_height: Height,
    pub current_round: Round,
    pub current_proposer: Option<BasePeerAddress>,
    pub current_role: Role,
    pub peers: HashSet<MalachitePeerId>,
    pub store: Store,

    pub signing_provider: Ed25519Provider,
    pub streams_map: PartStreamsMap,
    pub rng: StdRng,

    // Handle to the payload builder service
    pub engine_handle: PayloadBuilderHandle<EthPayloadTypes>,
}

impl State {
    pub fn new(ctx: MalachiteContext, config: Config, genesis: Genesis, address: Address) -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();

        Self {
            ctx,
            config,
            genesis,
            address,
            current_height: Height::default(),
            current_round: Round::Nil,
            current_proposer: None,
            current_role: Role::None,
            peers: HashSet::new(),
            store: Store::new(),
            signing_provider: Ed25519Provider::new(),
            streams_map: PartStreamsMap::new(),
            rng: StdRng::seed_from_u64(seed_from_address(&address, std::process::id() as u64)),
            engine_handle: PayloadBuilderHandle::new(tx),
        }
    }

    pub fn signing_provider(&self) -> &Ed25519Provider {
        &self.signing_provider
    }

    pub fn rng(&mut self) -> &mut StdRng {
        &mut self.rng
    }

    /// Returns the validator set for the given height
    /// For now, returns a fixed validator set from genesis
    pub fn get_validator_set(&self, _height: Height) -> BasePeerSet {
        // For now, return a simple validator set based on genesis
        // In a real implementation, this would query the actual validator set
        BasePeerSet {
            peers: vec![],
            total_voting_power: 0,
        }
    }

    /// Creates a new proposal value for the given height and round
    pub async fn propose_value(
        &mut self,
        height: Height,
        round: Round,
    ) -> Result<LocallyProposedValue<MalachiteContext>> {
        // Simulate building a block
        sleep(Duration::from_millis(100)).await;

        // Create a simple value - in real implementation this would be a proper block
        let value = BaseValue {
            data: vec![1, 2, 3, 4], // Placeholder data
        };

        info!("Proposed value for height {} round {}", height, round);

        Ok(LocallyProposedValue::new(height, round, value))
    }

    /// Processes a received proposal part and potentially returns a complete proposal
    pub async fn received_proposal_part(
        &mut self,
        from: MalachitePeerId,
        _part: StreamMessage<BaseProposalPart>,
    ) -> Result<Option<ProposedValue<MalachiteContext>>> {
        // For now, just return None - this would normally reassemble streaming proposals
        info!("Received proposal part from {}", from);
        Ok(None)
    }

    /// Creates stream messages for a proposal
    pub fn stream_proposal(
        &mut self,
        value: LocallyProposedValue<MalachiteContext>,
        _pol_round: Round,
    ) -> impl Iterator<Item = StreamMessage<BaseProposalPart>> {
        // For now, return empty iterator - this would normally split proposal into parts
        info!("Streaming proposal for height {}", value.height);
        std::iter::empty()
    }

    /// Commits a decided value
    pub async fn commit(
        &mut self,
        certificate: CommitCertificate<MalachiteContext>,
        _extensions: VoteExtensions<MalachiteContext>,
    ) -> Result<()> {
        info!("Committing value at height {}", certificate.height);
        // In real implementation, this would commit the block to the chain
        Ok(())
    }

    /// Gets a decided value at the given height
    pub async fn get_decided_value(&self, height: Height) -> Option<DecidedValue> {
        // For now, return None - this would query the committed blocks
        info!("Requested decided value for height {}", height);
        None
    }

    /// Gets the earliest available height
    pub async fn get_earliest_height(&self) -> Height {
        Height::INITIAL // Start from height 1
    }

    /// Gets a previously built value for re-use
    pub async fn get_previously_built_value(
        &self,
        height: Height,
        round: Round,
    ) -> Result<Option<LocallyProposedValue<MalachiteContext>>> {
        // For now, return None - this would check for previously built proposals
        info!(
            "Requested previously built value for height {} round {}",
            height, round
        );
        Ok(None)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Genesis {
    pub chain_id: String,
    pub validators: Vec<ValidatorInfo>,
    pub app_state: Vec<u8>,
}

impl Genesis {
    pub fn new(chain_id: String) -> Self {
        Self {
            chain_id,
            validators: Vec::new(),
            app_state: Vec::new(),
        }
    }

    pub fn with_validators(mut self, validators: Vec<ValidatorInfo>) -> Self {
        self.validators = validators;
        self
    }

    pub fn with_app_state(mut self, app_state: Vec<u8>) -> Self {
        self.app_state = app_state;
        self
    }
}

impl Default for Genesis {
    fn default() -> Self {
        Self::new("malachite-test".to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidatorInfo {
    pub address: Address,
    pub voting_power: u64,
    pub public_key: Vec<u8>,
}

impl ValidatorInfo {
    pub fn new(address: Address, voting_power: u64, public_key: Vec<u8>) -> Self {
        Self {
            address,
            voting_power,
            public_key,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub block_time: std::time::Duration,
    pub create_empty_blocks: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            block_time: std::time::Duration::from_secs(1),
            create_empty_blocks: true,
        }
    }
}

/// The role that the node is playing in the consensus protocol during a round.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// The node is the proposer for the current round.
    Proposer,
    /// The node is a validator for the current round.
    Validator,
    /// The node is not participating in the consensus protocol for the current round.
    None,
}

// Use reth store implementation
#[derive(Debug, Clone)]
pub struct Store {
    // This would typically interface with reth's storage layer
    // For now, we'll use a simple in-memory store
    data: HashMap<Vec<u8>, Vec<u8>>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.data.get(key)
    }

    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.data.insert(key, value);
    }

    pub fn delete(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.data.remove(key)
    }

    pub fn contains_key(&self, key: &[u8]) -> bool {
        self.data.contains_key(key)
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct PartStreamsMap {
    // Maps from peer ID to their partial stream state
    streams: HashMap<MalachitePeerId, PartialStreamState>,
}

impl PartStreamsMap {
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
        }
    }

    pub fn get_stream(&self, peer_id: &MalachitePeerId) -> Option<&PartialStreamState> {
        self.streams.get(peer_id)
    }

    pub fn get_stream_mut(&mut self, peer_id: &MalachitePeerId) -> Option<&mut PartialStreamState> {
        self.streams.get_mut(peer_id)
    }

    pub fn insert_stream(&mut self, peer_id: MalachitePeerId, stream: PartialStreamState) {
        self.streams.insert(peer_id, stream);
    }

    pub fn remove_stream(&mut self, peer_id: &MalachitePeerId) -> Option<PartialStreamState> {
        self.streams.remove(peer_id)
    }
}

impl Default for PartStreamsMap {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct PartialStreamState {
    pub height: Height,
    pub round: Round,
    pub step: ConsensusStep,
    pub last_activity: std::time::Instant,
}

impl PartialStreamState {
    pub fn new(height: Height, round: Round) -> Self {
        Self {
            height,
            round,
            step: ConsensusStep::NewHeight,
            last_activity: std::time::Instant::now(),
        }
    }

    pub fn update_activity(&mut self) {
        self.last_activity = std::time::Instant::now();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusStep {
    NewHeight,
    NewRound,
    Propose,
    Prevote,
    Precommit,
    Commit,
}

// Additional types needed for the consensus interface

#[derive(Debug, Clone)]
pub struct DecidedValue {
    pub value: BaseValue,
    pub certificate: CommitCertificate<MalachiteContext>,
}

impl Store {
    /// Returns the maximum decided value height
    pub async fn max_decided_value_height(&self) -> Option<Height> {
        // For now, return None - this would query the highest committed height
        None
    }

    /// Gets undecided proposals for a height and round
    pub async fn get_undecided_proposals(
        &self,
        _height: Height,
        _round: Round,
    ) -> Result<Vec<ProposedValue<MalachiteContext>>> {
        // For now, return empty vec - this would query pending proposals
        Ok(vec![])
    }

    /// Stores an undecided proposal
    pub async fn store_undecided_proposal(
        &mut self,
        _proposal: ProposedValue<MalachiteContext>,
    ) -> Result<()> {
        // For now, do nothing - this would store the proposal
        Ok(())
    }

    /// Gets an undecided proposal by height, round, and value ID
    pub async fn get_undecided_proposal(
        &self,
        _height: Height,
        _round: Round,
        _value_id: ValueId,
    ) -> Result<Option<ProposedValue<MalachiteContext>>> {
        // For now, return None - this would query a specific proposal
        Ok(None)
    }
}

// Standalone functions

/// Reload the tracing subscriber log level based on the current height and round
pub fn reload_log_level(_height: Height, _round: Round) {
    // For now, do nothing - this would adjust log levels
}

/// Encode a value to its byte representation
pub fn encode_value(_value: &BaseValue) -> Bytes {
    // For now, return empty bytes - this would serialize the value
    Bytes::new()
}

/// Decode a value from its byte representation
pub fn decode_value(_bytes: Bytes) -> Option<BaseValue> {
    // For now, return None - this would deserialize the value
    None
}

// Type alias for compatibility
pub type ValueId = ValueIdWrapper;

