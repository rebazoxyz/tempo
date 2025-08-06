//! Genesis file generator for Tempo
//!
//! This utility generates a genesis.json file with accounts derived from mnemonics.

use alloy::signers::{
    local::{MnemonicBuilder, coins_bip39::English},
    utils::secret_key_to_address
};
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{Address, B256, Bytes, U256};
use clap::Parser;
use eyre::Result;
use rayon::prelude::*;
use simple_tqdm::ParTqdm;
use std::{collections::BTreeMap, fs, path::PathBuf};

/// Genesis generator CLI arguments
#[derive(Debug, Clone, Parser)]
#[command(name = "tempo-chainspec")]
#[command(about = "Generate a genesis.json file with mnemonic-derived accounts")]
pub struct Args {
    /// Output file path for the generated genesis file
    #[arg(short, long, default_value = "genesis.json")]
    pub output: PathBuf,

    /// Mnemonic phrase for account generation (12 or 24 words)
    #[arg(
        short,
        long,
        default_value = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    )]
    pub mnemonic: String,

    /// Number of accounts to generate from the mnemonic
    #[arg(short = 'n', long, default_value = "10")]
    pub accounts: u32,

    /// Initial balance for each account (in ETH)
    #[arg(short, long, default_value = "10000")]
    pub balance: u64,

    /// Chain ID
    #[arg(short, long, default_value = "2600")]
    pub chain_id: u64,

    /// Gas limit for genesis block
    #[arg(short, long, default_value = "30000000")]
    pub gas_limit: u64,

    /// Pretty print the JSON output
    #[arg(long)]
    pub pretty: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!(
        "Generating genesis file with {} accounts from mnemonic...",
        args.accounts
    );

    // Generate accounts from mnemonic
    let accounts = generate_accounts_from_mnemonic(&args.mnemonic, args.accounts);

    // Convert balance from ETH to wei
    let balance_wei = U256::from(args.balance) * U256::from(10u64.pow(18));

    // Create genesis accounts
    let mut alloc = BTreeMap::new();
    for (_, address) in accounts.iter().enumerate() {
        alloc.insert(
            *address,
            GenesisAccount {
                balance: balance_wei,
                ..Default::default()
            },
        );
    }

    // Create genesis configuration
    let genesis = Genesis {
        config: Default::default(),
        nonce: 0x42,
        timestamp: 0x0,
        extra_data: Bytes::from_static(b"tempo-genesis"),
        gas_limit: args.gas_limit,
        difficulty: U256::from(0x400000000_u64),
        mix_hash: B256::ZERO,
        coinbase: Address::ZERO,
        number: Some(0),
        alloc,
        ..Default::default()
    };

    // Serialize genesis to JSON
    let json_output = if args.pretty {
        serde_json::to_string_pretty(&genesis)
    } else {
        serde_json::to_string(&genesis)
    }?;

    // Write to file
    fs::write(&args.output, json_output)?;

    println!("\nGenesis file generated successfully!");
    println!("Output file: {}", args.output.display());
    println!("Chain ID: {}", args.chain_id);
    println!("Accounts: {}", args.accounts);
    println!("Balance per account: {} ETH", args.balance);

    Ok(())
}

/// Generate Ethereum addresses from a mnemonic using BIP32 derivation path
/// Uses the standard Ethereum derivation path: m/44'/60'/0'/0/{index}
fn generate_accounts_from_mnemonic(mnemonic: &str, count: u32) -> Vec<Address> {
    println!("Generating {count} accounts...");

    (0..count)
        .into_par_iter()
        .tqdm()
        .map(|worker_id| {
            let signer = MnemonicBuilder::<English>::default()
                .phrase(mnemonic)
                .index(worker_id)
                .unwrap()
                .build()
                .unwrap();

            secret_key_to_address(signer.credential())
        })
        .collect()
}
