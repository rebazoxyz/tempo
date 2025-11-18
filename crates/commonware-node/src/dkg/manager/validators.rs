use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs as _},
};

use alloy_evm::EvmInternals;
use alloy_primitives::Address;
use commonware_codec::{DecodeExt as _, EncodeSize, RangeCfg, Read, Write};
use commonware_consensus::{types::Epoch, utils};
use commonware_cryptography::ed25519::PublicKey;
use commonware_utils::set::{Ordered, OrderedAssociated};
use eyre::{OptionExt as _, WrapErr as _, ensure};
use reth_ethereum::evm::revm::{State, database::StateProviderDatabase};
use reth_node_builder::{Block as _, ConfigureEvm as _};
use reth_provider::{BlockReader as _, StateProviderFactory as _};
use ringbuffer::RingBuffer as _;
use tempo_node::TempoFullNode;
use tempo_precompiles::{
    storage::evm::EvmPrecompileStorageProvider,
    validator_config::{
        IValidatorConfig, ValidatorConfig, ensure_inbound_is_host_port, ensure_outbound_is_ip_port,
    },
};

use tracing::{Level, info, instrument, warn};

/// Reads the validator config of `epoch`.
///
/// The validator config for `epoch` is always read from the last height of
/// `epoch-1`.
#[instrument(
    skip_all,
    fields(
        attempt = _attempt,
        for_epoch,
        last_height_before = last_height_before_epoch(for_epoch, epoch_length),
    ),
    err
)]
pub(super) async fn read_from_contract(
    _attempt: u32,
    node: &TempoFullNode,
    for_epoch: Epoch,
    epoch_length: u64,
) -> eyre::Result<OrderedAssociated<PublicKey, DecodedValidator>> {
    let last_height = last_height_before_epoch(for_epoch, epoch_length);
    let block = node
        .provider
        .block_by_number(last_height)
        .map_err(Into::<eyre::Report>::into)
        .and_then(|maybe| maybe.ok_or_eyre("execution layer returned empty block"))
        .wrap_err_with(|| format!("failed reading block at height `{last_height}`"))?;

    let db = State::builder()
        .with_database(StateProviderDatabase::new(
            node.provider
                .state_by_block_id(last_height.into())
                .wrap_err_with(|| {
                    format!("failed to get state from node provider for height `{last_height}`")
                })?,
        ))
        .build();

    // XXX: Ensure that evm and internals go out of scope before the await point
    // below.
    let contract_validators = {
        let mut evm = node
            .evm_config
            .evm_for_block(db, block.header())
            .wrap_err("failed instantiating evm for genesis block")?;

        let ctx = evm.ctx_mut();
        let internals = EvmInternals::new(&mut ctx.journaled_state, &ctx.block);
        let mut provider = EvmPrecompileStorageProvider::new_max_gas(internals, &ctx.cfg);

        let mut validator_config = ValidatorConfig::new(&mut provider);
        validator_config
            .get_validators(IValidatorConfig::getValidatorsCall {})
            .wrap_err("failed to query contract for validator config")?
    };

    Ok(decode_from_contract(contract_validators).await)
}

#[instrument(skip_all, fields(validators_to_decode = contract_vals.len()))]
async fn decode_from_contract(
    contract_vals: Vec<IValidatorConfig::Validator>,
) -> OrderedAssociated<PublicKey, DecodedValidator> {
    let mut decoded = HashMap::new();
    for val in contract_vals.into_iter().filter(|val| val.active) {
        // NOTE: not reporting errors because `decode_from_contract` emits
        // events on success and error
        if let Ok(val) = DecodedValidator::decode_from_contract(val)
            && let Some(old) = decoded.insert(val.public_key.clone(), val)
        {
            warn!(
                %old,
                new = %decoded.get(&old.public_key).expect("just inserted it"),
                "replaced peer because public keys were duplicated",
            );
        }
    }
    decoded.into_iter().collect::<_>()
}

/// Tracks the participants of each DKG ceremony, and, by extension, the p2p network.
///
/// The participants tracked here are in order:
///
/// 1. the dealers, that will drop out of the next ceremony
/// 2. the player, that will become dealers in the next ceremony
/// 3. the syncing players, that will become players in the next ceremony
pub(super) struct Participants {
    buffered: ringbuffer::ConstGenericRingBuffer<OrderedAssociated<PublicKey, DecodedValidator>, 3>,
}

impl Participants {
    pub(super) fn new(validators: OrderedAssociated<PublicKey, DecodedValidator>) -> Self {
        let mut buffered = ringbuffer::ConstGenericRingBuffer::new();
        buffered.enqueue(validators.clone());
        buffered.enqueue(validators.clone());
        buffered.enqueue(validators);
        Self { buffered }
    }

    pub(super) fn dealers(&self) -> &OrderedAssociated<PublicKey, DecodedValidator> {
        &self.buffered[0]
    }

    pub(super) fn syncing_players(&self) -> &Ordered<PublicKey> {
        &self.buffered[0]
    }

    pub(super) fn dealer_pubkeys(&self) -> Ordered<PublicKey> {
        self.buffered[0].keys().clone()
    }

    pub(super) fn player_pubkeys(&self) -> Ordered<PublicKey> {
        self.buffered[1].keys().clone()
    }

    /// Constructs a peerset to register on the peer manager.
    ///
    /// The peerset is constructed by merging the participants of all the
    /// validator sets tracked in this queue, and resolving each of their
    /// addresses (parsing socket address or looking up domain name).
    ///
    /// If a validator has entries across the tracked sets, then then its entry
    /// for the latest pushed set is taken. For those cases where looking up
    /// domain names failed, the last successfully looked up name is taken.
    pub(super) fn construct_peers_to_register(&self) -> PeersRegistered {
        PeersRegistered(
            self.buffered
                .iter()
                // IMPORTANT: iterator starting from the latest registered set.
                .rev()
                .flat_map(|valset| valset.iter_pairs())
                .filter_map(|(pubkey, validator)| {
                    let addr = validator.inbound_to_socket_addr().ok()?;
                    Some((pubkey.clone(), addr))
                })
                .collect(),
        )
    }

    /// Pushes `validators` into the participants queue.
    ///
    /// Returns the oldest peers that were pushed into this queue (usually
    /// the dealers of the previous ceremony).
    pub(super) fn push(
        &mut self,
        validators: OrderedAssociated<PublicKey, DecodedValidator>,
    ) -> OrderedAssociated<PublicKey, DecodedValidator> {
        self.buffered
            .enqueue(validators)
            .expect("the buffer must always be full")
    }
}

/// A ContractValidator is a peer read from the validator config smart const.
///
/// The inbound and outbound addresses stored herein are guaranteed to be of the
/// form `<host>:<port>` for inbound, and `<ip>:<port>` for outbound. Here,
/// `<host>` is either an IPv4 or IPV6 address, or a fully qualified domain name.
/// `<ip>` is an IPv4 or IPv6 address.
#[derive(Clone, Debug)]
pub(super) struct DecodedValidator {
    pub(super) public_key: PublicKey,
    pub(super) inbound: String,
    pub(super) outbound: String,
    pub(super) index: u64,
    pub(super) address: Address,
}

impl DecodedValidator {
    /// Attempts to decode a single validator from the values read in the smart contract.
    ///
    /// This function does not perform hostname lookup on either of the addresses.
    /// Instead, only the shape of the addresses are checked for whether they are
    /// socket addresses (IP:PORT pairs), or fully qualified domain names.
    #[instrument(ret(Display, level = Level::INFO), err(level = Level::WARN))]
    pub(super) fn decode_from_contract(
        IValidatorConfig::Validator {
            publicKey,
            active,
            index,
            validatorAddress,
            inboundAddress,
            outboundAddress,
            ..
        }: IValidatorConfig::Validator,
    ) -> eyre::Result<Self> {
        ensure!(
            active,
            "field `active` is set to false; this method should only be called \
            for active validators"
        );
        let public_key = PublicKey::decode(publicKey.as_ref())
            .wrap_err("failed decoding publicKey field as ed25519 public key")?;
        ensure_inbound_is_host_port(&inboundAddress).wrap_err("inboundAddress was not valid")?;
        ensure_outbound_is_ip_port(&outboundAddress).wrap_err("outboundAddress was not valid")?;
        Ok(Self {
            public_key,
            inbound: inboundAddress,
            outbound: outboundAddress,
            index,
            address: validatorAddress,
        })
    }

    /// Converts a decoded validator to a (pubkey, socket addr) pair.
    ///
    /// At the moment, only the inbound address is considered (constraint of
    /// [`commonware_p2p::authenticated::lookup`]). If the inbound value is a
    /// socket address, then the conversion is immediate. If is a domain name,
    /// the domain name is resolved. If DNS resolution returns more than 1 value,
    /// the last one is taken.
    #[instrument(skip_all, fields(public_key = %self.public_key, inbound = self.inbound), err)]
    fn inbound_to_socket_addr(&self) -> eyre::Result<SocketAddr> {
        let all_addrs = self
            .inbound
            .to_socket_addrs()
            .wrap_err_with(|| format!("failed resolving inbound address `{}`", self.inbound))?
            .collect::<Vec<_>>();
        let addr = match &all_addrs[..] {
            [] => return Err(eyre::eyre!("found no addresses for `{}`", self.inbound)),
            [addr] => *addr,
            [dropped @ .., addr] => {
                info!(
                    ?dropped,
                    "resolved to more than one; dropping all except the last"
                );
                *addr
            }
        };
        info!(%addr, "using address");
        Ok(addr)
    }
}

impl std::fmt::Display for DecodedValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "public key = `{}`, inbound = `{}`, outbound = `{}`, index = `{}`, address = `{}`",
            self.public_key, self.inbound, self.outbound, self.index, self.address
        ))
    }
}

/// Peers that were registered on the peer manager.
#[derive(Clone)]
pub(super) struct PeersRegistered(OrderedAssociated<PublicKey, SocketAddr>);

impl PeersRegistered {
    pub(super) fn into_inner(self) -> OrderedAssociated<PublicKey, SocketAddr> {
        self.0
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Debug for PeersRegistered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Write for PeersRegistered {
    fn write(&self, buf: &mut impl bytes::BufMut) {
        self.0.write(buf);
    }
}

impl EncodeSize for PeersRegistered {
    fn encode_size(&self) -> usize {
        self.0.encode_size()
    }
}

impl Read for PeersRegistered {
    type Cfg = ();

    fn read_cfg(
        buf: &mut impl bytes::Buf,
        _cfg: &Self::Cfg,
    ) -> Result<Self, commonware_codec::Error> {
        let inner = OrderedAssociated::read_cfg(buf, &(RangeCfg::from(0..=usize::MAX), (), ()))?;
        Ok(Self(inner))
    }
}

fn last_height_before_epoch(epoch: Epoch, epoch_length: u64) -> u64 {
    epoch
        .checked_sub(1)
        .map_or(0, |epoch| utils::last_block_in_epoch(epoch_length, epoch))
}
