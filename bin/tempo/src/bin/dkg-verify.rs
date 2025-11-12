//! DKG Outcome Verification Tool
//!
//! This tool traverses a blockchain database and verifies DKG ceremony outcomes
//! by extracting IntermediateOutcomes and PublicOutcomes from block headers.
//!
//! Usage:
//!   cargo run --bin dkg-verify -- --datadir /path/to/datadir --epoch-length <LENGTH> [OPTIONS]
//!
//! Features:
//! - Extracts all IntermediateOutcomes from block extra_data (second half of epochs)
//! - Extracts all PublicOutcomes from boundary block extra_data
//! - Verifies that PublicOutcomes match expected ceremony results
//! - Reports discrepancies, missing outcomes, and ceremony failures

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use reth_provider::HeaderProvider;
use alloy_genesis::Genesis;
use bytes::Buf;
use clap::Parser;
use commonware_codec::{DecodeExt, Encode as _, RangeCfg, Read as CodecRead, ReadExt as _};
use commonware_cryptography::{
    bls12381::{
        dkg::ops::{construct_public, recover_public},
        primitives::{poly::Public, variant::MinSig},
    },
    ed25519::PublicKey,
};
use commonware_utils::{quorum, set::Ordered};
use eyre::{Context, Result};
use reth_db::open_db_read_only;
use reth_node_builder::NodeTypesWithDBAdapter;
use reth_provider::{
    providers::StaticFileProvider, BlockNumReader, BlockReader, ProviderFactory,
};
use tempo_chainspec::{TempoChainSpec, spec::ANDANTINO};
use tempo_node::node::TempoNode;

/// DKG Verification Tool
#[derive(Parser, Debug)]
#[command(name = "dkg-verify")]
#[command(about = "Verify DKG ceremony outcomes from blockchain data", long_about = None)]
struct Args {
    /// Path to the data directory
    #[arg(long, value_name = "PATH")]
    datadir: PathBuf,

    /// Start block height (default: 0)
    #[arg(long, default_value = "0")]
    start_height: u64,

    /// End block height (default: latest)
    #[arg(long)]
    end_height: Option<u64>,

    /// Epoch length (required - typically from consensus config)
    #[arg(long)]
    epoch_length: u64,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Only show failures/discrepancies
    #[arg(long)]
    only_failures: bool,
}

// Simple local structs to decode on-chain artifacts
#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicOutcome {
    epoch: u64,
    participants: Ordered<PublicKey>,
    public: Public<MinSig>,
}

impl CodecRead for PublicOutcome {
    type Cfg = ();

    fn read_cfg(
        buf: &mut impl Buf,
        _cfg: &Self::Cfg,
    ) -> Result<Self, commonware_codec::Error> {
        let epoch = commonware_codec::varint::UInt::read(buf)?.into();
        let participants = Ordered::read_cfg(buf, &(RangeCfg::from(0..=usize::MAX), ()))?;
        let public =
            Public::<MinSig>::read_cfg(buf, &(quorum(participants.len() as u32) as usize))?;
        Ok(Self {
            epoch,
            participants,
            public,
        })
    }
}

#[derive(Debug, Clone)]
struct IntermediateOutcome {
    dealer: PublicKey,
    epoch: u64,
    num_acks: usize,
    commitment: Public<MinSig>,
}

impl CodecRead for IntermediateOutcome {
    type Cfg = ();

    fn read_cfg(
        buf: &mut impl Buf,
        _cfg: &Self::Cfg,
    ) -> Result<Self, commonware_codec::Error> {
        use commonware_codec::varint::UInt;
        use commonware_cryptography::ed25519::Signature;

        // IntermediateOutcome wire format:
        // n_players: UInt
        // dealer: PublicKey
        // dealer_signature: Signature
        // epoch: UInt
        // commitment: Public<MinSig>
        // acks: Vec<Ack>
        // reveals: Vec<group::Share>

        let n_players: u64 = UInt::read(buf)?.into();
        let dealer = PublicKey::read(buf)?;
        let _dealer_signature = Signature::read(buf)?;
        let epoch: u64 = UInt::read(buf)?.into();

        // Read commitment (we need this to verify the final outcome)
        let commitment =
            Public::<MinSig>::read_cfg(buf, &(quorum(n_players as u32) as usize))?;

        // Read acks to count them
        let acks_len_uint = UInt::read(buf)?;
        let acks_len: usize = u64::from(acks_len_uint) as usize;

        // We only need the count, so skip reading the actual acks
        // Each Ack is: PublicKey (32 bytes) + Signature (64 bytes) = 96 bytes
        for _ in 0..acks_len {
            let _player = PublicKey::read(buf)?;
            let _sig = Signature::read(buf)?;
        }

        // Skip reveals (we don't need them)
        // Just advance the buffer to consume them
        let reveals_len_uint = UInt::read(buf)?;
        let reveals_len: usize = u64::from(reveals_len_uint) as usize;
        for _ in 0..reveals_len {
            use commonware_cryptography::bls12381::primitives::group::Share;
            let _ = Share::read(buf)?;
        }

        Ok(Self {
            dealer,
            epoch,
            num_acks: acks_len,
            commitment,
        })
    }
}

#[derive(Debug)]
struct EpochData {
    epoch: u64,
    intermediate_outcomes: BTreeMap<PublicKey, IntermediateOutcome>,
    public_outcome: Option<PublicOutcome>,
    boundary_height: u64,
    boundary_extra_data: Option<Vec<u8>>,
    /// The public polynomial from the previous epoch (needed for recover_public)
    previous_public: Option<Public<MinSig>>,
}

impl EpochData {
    fn new(epoch: u64, epoch_length: u64, previous_public: Option<Public<MinSig>>) -> Self {
        Self {
            epoch,
            intermediate_outcomes: BTreeMap::new(),
            public_outcome: None,
            boundary_height: (epoch + 1) * epoch_length - 1,
            boundary_extra_data: None,
            previous_public,
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    if args.verbose {
        tracing_subscriber::fmt()
            .with_target(false)
            .with_level(true)
            .init();
    }

    println!("🔍 DKG Outcome Verification Tool");
    println!("📂 Data directory: {}", args.datadir.display());
    println!();

    // Open database
    let db_path = args.datadir.join("db");
    println!("Opening database at {}...", db_path.display());
    let db = open_db_read_only(db_path.as_path(), Default::default())
        .wrap_err("failed to open database")?;

    // // Load chainspec
    // let chainspec_path = args.datadir.join("chainspec.json");
    // println!("Loading chainspec from {}...", chainspec_path.display());
    // let genesis: Genesis = serde_json::from_reader(
    //     std::fs::File::open(&chainspec_path).wrap_err("failed to open chainspec.json")?,
    // )
    // .wrap_err("failed to parse chainspec")?;

    // let chain_spec = TempoChainSpec::tes
    let chain_spec = ANDANTINO.clone();
    let epoch_length = args.epoch_length;

    println!("Epoch length: {}", epoch_length);
    println!();

    // Create static file provider for read-only access
    let static_file_provider = StaticFileProvider::read_only(dbg!(args.datadir.join("static_files")), false)
        .wrap_err("failed to create static file provider")?;

    // Create provider factory
    let factory = ProviderFactory::<NodeTypesWithDBAdapter<TempoNode, _>>::new(
        Arc::new(db),
        chain_spec.into(),
        static_file_provider.clone(),
    )
    .wrap_err("failed to create provider factory")?;

    let provider = factory.provider().wrap_err("failed to create provider")?;

    // Determine block range
    let start_height = args.start_height;
    let end_height = if let Some(end) = args.end_height {
        end
    } else {
        provider
            .last_block_number()
            .wrap_err("failed to get last block number")?
    };

    println!("Scanning blocks {} to {}...", start_height, end_height);
    println!();

    // Collect outcomes by epoch
    let mut epochs: BTreeMap<u64, EpochData> = BTreeMap::new();
    let mut last_public: Option<Public<MinSig>> = None;

    for height in start_height..=end_height {
        let header_opt = static_file_provider
            .header_by_hash_or_number(height.into())
            .wrap_err_with(|| format!("failed to read block {}", height))?;

        let Some(header) = header_opt else {
            // Block doesn't exist yet - note it for boundary blocks
            let epoch = height / epoch_length;
            let epoch_data = epochs
                .entry(epoch)
                .or_insert_with(|| EpochData::new(epoch, epoch_length, last_public.clone()));

            if height == epoch_data.boundary_height {
                epoch_data.boundary_extra_data = None; // Mark that boundary block doesn't exist
            }
            continue;
        };

        let extra_data = &header.inner.extra_data;

        if extra_data.is_empty() {
            continue;
        }

        let epoch = height / epoch_length;
        let epoch_data = epochs
            .entry(epoch)
            .or_insert_with(|| EpochData::new(epoch, epoch_length, last_public.clone()));

        // Check if this is a boundary block OR block 0 (genesis)
        let is_boundary = height == epoch_data.boundary_height;
        let is_genesis = height == 0;

        if is_boundary || is_genesis {
            // Store the boundary block's extra_data for debugging
            epoch_data.boundary_extra_data = Some(extra_data.to_vec());

            // Try to decode PublicOutcome
            match PublicOutcome::decode(extra_data.as_ref()) {
                Ok(outcome) => {
                    if args.verbose || is_genesis {
                        let block_type = if is_genesis { "Genesis block" } else { "Boundary block" };
                        println!(
                            "✅ {} {}: Found PublicOutcome for epoch {}",
                            block_type, height, outcome.epoch
                        );
                        println!("   Participants: {}", outcome.participants.len());
                    }
                    // Check if outcome epoch matches expected
                    if outcome.epoch != epoch {
                        println!(
                            "⚠️  Block {}: PublicOutcome epoch mismatch! Found {}, expected {}",
                            height, outcome.epoch, epoch
                        );
                    }
                    // Update last_public for the next epoch
                    last_public = Some(outcome.public.clone());
                    epoch_data.public_outcome = Some(outcome);
                }
                Err(e) => {
                    if args.verbose || is_genesis {
                        let block_type = if is_genesis { "Genesis block" } else { "Block" };
                        println!(
                            "⚠️  {} {}: Failed to decode PublicOutcome: {}",
                            block_type, height, e
                        );
                    }
                }
            }
        } else {
            // Try to decode IntermediateOutcome
            match IntermediateOutcome::decode(extra_data.as_ref()) {
                Ok(outcome) => {
                    if args.verbose {
                        println!(
                            "📝 Block {}: Found IntermediateOutcome from dealer",
                            height
                        );
                        println!("   Epoch: {}", outcome.epoch);
                        println!("   Acks: {}", outcome.num_acks);
                        println!("   Dealer: {:?}", outcome.dealer);
                    }

                    // Use the dealer's public key directly from the outcome
                    let dealer_key = outcome.dealer.clone();
                    epoch_data.intermediate_outcomes.insert(dealer_key, outcome);
                }
                Err(e) => {
                    if args.verbose {
                        println!(
                            "⚠️  Block {}: Failed to decode IntermediateOutcome: {}",
                            height, e
                        );
                    }
                }
            }
        }
    }

    println!("📊 Analysis Results");
    println!("==================");
    println!();

    let mut total_epochs = 0;
    let mut successful_ceremonies = 0;
    let mut failed_ceremonies = 0;
    let mut missing_outcomes = 0;

    for (epoch, data) in &epochs {
        total_epochs += 1;

        let has_public = data.public_outcome.is_some();
        let num_intermediates = data.intermediate_outcomes.len();

        if args.only_failures && has_public && num_intermediates > 0 {
            continue;
        }

        println!("Epoch {}", epoch);
        println!("  Boundary height: {}", data.boundary_height);
        println!("  IntermediateOutcomes collected: {}", num_intermediates);

        if let Some(ref outcome) = data.public_outcome {
            println!("  ✅ PublicOutcome found:");
            println!("     Participants: {}", outcome.participants.len());
            let player_threshold = quorum(outcome.participants.len() as u32);
            println!("     Expected threshold: {}", player_threshold);

            // Verify we had enough dealers
            let required_dealers = player_threshold as usize;
            if num_intermediates < required_dealers {
                println!(
                    "  ⚠️  WARNING: Only {} IntermediateOutcomes, but need {} for threshold",
                    num_intermediates, required_dealers
                );
            } else {
                // Compute what the PublicOutcome should be from the IntermediateOutcomes
                println!("  🔍 Verifying PublicOutcome matches IntermediateOutcomes...");

                // Sort intermediate outcomes by dealer index (need to map PublicKey to index)
                let dealers: Vec<_> = outcome.participants.iter().cloned().collect();
                let mut sorted_commitments: Vec<(usize, &Public<MinSig>)> = data
                    .intermediate_outcomes
                    .iter()
                    .filter_map(|(dealer_key, intermediate)| {
                        dealers
                            .iter()
                            .position(|d| d == dealer_key)
                            .map(|idx| (idx, &intermediate.commitment))
                    })
                    .collect();
                sorted_commitments.sort_by_key(|(idx, _)| *idx);

                if sorted_commitments.len() < required_dealers {
                    println!(
                        "  ❌ ERROR: Could only match {} dealers from IntermediateOutcomes",
                        sorted_commitments.len()
                    );
                } else {
                    // Compute the expected public polynomial
                    // Use recover_public if we have a previous polynomial (resharing),
                    // otherwise use construct_public (initial ceremony)
                    let computed_result = if let Some(ref previous) = data.previous_public {
                        // Resharing: recover from previous polynomial + commitments
                        let mut commitments_map = std::collections::BTreeMap::new();
                        for (idx, commitment) in sorted_commitments.iter().take(required_dealers) {
                            commitments_map.insert(*idx as u32, (*commitment).clone());
                        }
                        recover_public::<MinSig>(
                            previous,
                            &commitments_map,
                            (player_threshold as usize).try_into().unwrap(),
                            1, // concurrency
                        )
                    } else {
                        // Initial ceremony: construct from commitments
                        let commitments: Vec<_> = sorted_commitments
                            .iter()
                            .take(required_dealers)
                            .map(|(_, commitment)| (*commitment).clone())
                            .collect();
                        construct_public::<MinSig>(
                            commitments.iter(),
                            (player_threshold as usize).try_into().unwrap(),
                        )
                    };

                    match computed_result {
                        Ok(computed_public) => {
                            if computed_public == outcome.public {
                                println!("  ✅ PublicOutcome VERIFIED: matches computed result from IntermediateOutcomes");
                            } else {
                                println!("  ❌ PublicOutcome MISMATCH: does NOT match computed result!");
                                println!("     This indicates the PublicOutcome in the block is incorrect.");
                                println!();
                                println!("     Computed PublicOutcome from IntermediateOutcomes:");
                                println!("       Epoch: {}", epoch);
                                println!("       Participants: {}", outcome.participants.len());
                                println!("       Public polynomial (hex): 0x{}", hex::encode(computed_public.encode()));
                                println!();
                                println!("     PublicOutcome in boundary block:");
                                println!("       Epoch: {}", outcome.epoch);
                                println!("       Participants: {}", outcome.participants.len());
                                println!("       Public polynomial (hex): 0x{}", hex::encode(outcome.public.encode()));
                            }
                        }
                        Err(e) => {
                            println!("  ⚠️  Failed to compute public polynomial: {:?}", e);
                            if data.previous_public.is_none() && *epoch > 0 {
                                println!("     Cannot verify epoch {} - missing previous epoch's PublicOutcome", epoch);
                                println!("     Need to scan from epoch 0's boundary block to get the initial PublicOutcome");
                            }
                        }
                    }
                }
            }

            successful_ceremonies += 1;
        } else {
            println!("  ❌ No PublicOutcome found at boundary block!");

            // Print the extra_data from the boundary block for debugging
            match data.boundary_extra_data.as_ref() {
                Some(boundary_extra_data) if !boundary_extra_data.is_empty() => {
                    println!("     Boundary block extra_data (hex): 0x{}", hex::encode(boundary_extra_data));
                    println!("     Boundary block extra_data (len): {} bytes", boundary_extra_data.len());
                }
                Some(_) => {
                    println!("     Boundary block had empty extra_data");
                }
                None => {
                    println!("     ⚠️  Boundary block {} does not exist yet", data.boundary_height);
                }
            }

            if num_intermediates == 0 {
                println!("     (No IntermediateOutcomes collected either)");
                missing_outcomes += 1;
            } else {
                println!(
                    "     (Had {} IntermediateOutcomes but ceremony failed)",
                    num_intermediates
                );

                // Still try to compute what the PublicOutcome should have been
                println!("  🔍 Attempting to compute what PublicOutcome should be...");

                // We need to know the participants list. Since we don't have a PublicOutcome,
                // we'll infer from the dealers who submitted IntermediateOutcomes
                let dealers: Vec<PublicKey> = data.intermediate_outcomes.keys().cloned().collect();
                let player_threshold = quorum(dealers.len() as u32);
                let required_dealers = player_threshold as usize;

                println!("     Inferred {} dealers from IntermediateOutcomes", dealers.len());
                println!("     Required threshold: {}", required_dealers);

                if num_intermediates >= required_dealers {
                    // Sort commitments by dealer
                    let mut sorted_commitments: Vec<(usize, &Public<MinSig>)> = data
                        .intermediate_outcomes
                        .iter()
                        .filter_map(|(dealer_key, intermediate)| {
                            dealers
                                .iter()
                                .position(|d| d == dealer_key)
                                .map(|idx| (idx, &intermediate.commitment))
                        })
                        .collect();
                    sorted_commitments.sort_by_key(|(idx, _)| *idx);

                    let commitments: Vec<_> = sorted_commitments
                        .into_iter()
                        .take(required_dealers)
                        .map(|(_, commitment)| commitment.clone())
                        .collect();

                    // Use recover_public if we have a previous polynomial, otherwise construct_public
                    let computed_result = if let Some(ref previous) = data.previous_public {
                        let mut commitments_map = std::collections::BTreeMap::new();
                        for (idx, (orig_idx, commitment)) in data
                            .intermediate_outcomes
                            .iter()
                            .filter_map(|(dealer_key, intermediate)| {
                                dealers
                                    .iter()
                                    .position(|d| d == dealer_key)
                                    .map(|idx| (idx, &intermediate.commitment))
                            })
                            .enumerate()
                            .take(required_dealers)
                        {
                            commitments_map.insert(orig_idx as u32, commitment.clone());
                        }
                        recover_public::<MinSig>(
                            previous,
                            &commitments_map,
                            (player_threshold as usize).try_into().unwrap(),
                            1,
                        )
                    } else {
                        construct_public::<MinSig>(
                            commitments.iter(),
                            (player_threshold as usize).try_into().unwrap(),
                        )
                    };

                    match computed_result {
                        Ok(computed_public) => {
                            println!("  ✅ Successfully computed PublicOutcome from IntermediateOutcomes!");
                            println!("     This means the ceremony COULD have succeeded.");
                            println!("     The PublicOutcome may not have been included in the boundary block.");
                            println!();
                            println!("     Computed PublicOutcome:");
                            println!("       Epoch: {}", epoch);
                            println!("       Participants: {}", dealers.len());
                            println!("       Public polynomial (hex): 0x{}", hex::encode(computed_public.encode()));
                        }
                        Err(e) => {
                            println!("  ❌ Failed to compute PublicOutcome: {:?}", e);
                            if data.previous_public.is_none() && *epoch > 0 {
                                println!("     Cannot verify epoch {} - missing previous epoch's PublicOutcome", epoch);
                                println!("     Need to scan from epoch 0's boundary block to get the initial PublicOutcome");
                            } else {
                                println!("     This explains why the ceremony failed - invalid commitments.");
                            }
                        }
                    }
                } else {
                    println!(
                        "     Only {} IntermediateOutcomes, need {} - ceremony failed due to insufficient dealings",
                        num_intermediates, required_dealers
                    );
                }

                failed_ceremonies += 1;
            }
        }

        // Check for discrepancies in IntermediateOutcomes
        if num_intermediates > 1 {
            let mut epochs_set = std::collections::HashSet::new();
            for outcome in data.intermediate_outcomes.values() {
                epochs_set.insert(outcome.epoch);
            }

            if epochs_set.len() > 1 {
                println!(
                    "  ⚠️  WARNING: IntermediateOutcomes have mismatched epochs: {:?}",
                    epochs_set
                );
            }
        }

        println!();
    }

    println!("Summary");
    println!("=======");
    println!("Total epochs analyzed: {}", total_epochs);
    println!("Successful ceremonies: {}", successful_ceremonies);
    println!("Failed ceremonies: {}", failed_ceremonies);
    println!("Missing outcomes: {}", missing_outcomes);

    if failed_ceremonies > 0 || missing_outcomes > 0 {
        println!();
        println!(
            "❌ Found {} issues",
            failed_ceremonies + missing_outcomes
        );
        std::process::exit(1);
    } else {
        println!();
        println!("✅ All epochs verified successfully");
    }

    Ok(())
}
