//! Tempo ↔ Ethereum Bridge Relayer
//!
//! A stateless relayer that monitors bridge events on both chains and submits
//! proofs to the destination chain for packet delivery.

mod ethereum;
mod proofs;
mod relayer;
mod tempo;

use clap::{Parser, ValueEnum};
use eyre::Result;
use std::str::FromStr;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Direction {
    TempoToEth,
    EthToTempo,
    Both,
}

#[derive(Parser, Debug)]
#[command(name = "tempo-bridge-relayer")]
#[command(about = "Relayer for the Tempo ↔ Ethereum trustless bridge")]
pub struct Args {
    /// Tempo RPC URL
    #[arg(long, env = "TEMPO_RPC_URL")]
    tempo_rpc: String,

    /// Ethereum RPC URL
    #[arg(long, env = "ETH_RPC_URL")]
    eth_rpc: String,

    /// Bridge contract address on Tempo
    #[arg(long, env = "TEMPO_BRIDGE_ADDRESS")]
    tempo_bridge: String,

    /// Bridge contract address on Ethereum
    #[arg(long, env = "ETH_BRIDGE_ADDRESS")]
    eth_bridge: String,

    /// Relayer wallet private key (hex, with or without 0x prefix)
    #[arg(long, env = "RELAYER_PRIVATE_KEY")]
    private_key: String,

    /// Relay direction
    #[arg(long, value_enum, default_value = "both")]
    direction: Direction,

    /// Polling interval in seconds
    #[arg(long, default_value = "12")]
    poll_interval: u64,

    /// Number of retries for failed transactions
    #[arg(long, default_value = "3")]
    max_retries: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("tempo_bridge_relayer=info".parse()?))
        .init();

    let args = Args::parse();

    info!(
        tempo_rpc = %args.tempo_rpc,
        eth_rpc = %args.eth_rpc,
        direction = ?args.direction,
        "Starting bridge relayer"
    );

    let tempo_bridge = alloy_primitives::Address::from_str(&args.tempo_bridge)?;
    let eth_bridge = alloy_primitives::Address::from_str(&args.eth_bridge)?;

    let config = relayer::RelayerConfig {
        tempo_rpc: args.tempo_rpc,
        eth_rpc: args.eth_rpc,
        tempo_bridge,
        eth_bridge,
        private_key: args.private_key,
        direction: args.direction,
        poll_interval_secs: args.poll_interval,
        max_retries: args.max_retries,
    };

    let relayer = relayer::Relayer::new(config).await?;
    relayer.run().await
}
