//! xtask is a Swiss army knife of tools that help with running and testing tempo.
use std::net::SocketAddr;

use crate::{
    consensus_config::{GenerateConfig, generate_config},
    devnet::{DevnetConfig, generate_devnet_configs},
    genesis::GenesisArgs,
};

use alloy::signers::{
    local::{MnemonicBuilder, coins_bip39::English},
    utils::secret_key_to_address,
};
use clap::Parser;
use commonware_codec::DecodeExt;
use commonware_cryptography::Signer;
use eyre::Context;

mod consensus_config;
mod devnet;
mod genesis;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    match args.action {
        Action::GenerateConfig(cfg) => generate_config(cfg).wrap_err("failed generating config"),
        Action::GenerateGenesis(args) => args.run().await.wrap_err("failed generating genesis"),
        Action::GenerateDevnet(cfg) => {
            generate_devnet_configs(cfg).wrap_err("failed generating devnet configs")
        }
        Action::GenerateAddPeer(cfg) => generate_config_to_add_peer(cfg),
    }
}

#[derive(Debug, clap::Parser)]
#[command(author)]
#[command(version)]
#[command(about)]
#[command(long_about = None)]
struct Args {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, clap::Subcommand)]
#[allow(clippy::enum_variant_names)]
enum Action {
    GenerateConfig(GenerateConfig),
    GenerateGenesis(GenesisArgs),
    GenerateDevnet(DevnetConfig),
    GenerateAddPeer(GenerateAddPeer),
}

#[derive(Debug, clap::Args)]
struct GenerateAddPeer {
    #[arg(long)]
    signer: String,

    #[arg(long)]
    addr: SocketAddr,

    #[arg(long, default_value_t = 0)]
    admin_index: u32,

    #[arg(long, default_value_t = 20)]
    validator_index: u32,

    #[arg(
        short,
        long,
        default_value = "test test test test test test test test test test test junk"
    )]
    pub mnemonic: String,
}

fn generate_config_to_add_peer(cfg: GenerateAddPeer) -> eyre::Result<()> {
    use tempo_precompiles::VALIDATOR_CONFIG_ADDRESS;
    let signer_bytes = const_hex::decode(&cfg.signer)?;
    let signer = commonware_cryptography::ed25519::PrivateKey::decode(&signer_bytes[..])?;

    let admin_key = MnemonicBuilder::<English>::default()
        .phrase(cfg.mnemonic.clone())
        .index(cfg.admin_index)?
        .build()?;

    let admin_key = const_hex::encode(admin_key.credential().to_bytes());

    let validator_address = {
        let key = MnemonicBuilder::<English>::default()
            .phrase(cfg.mnemonic.clone())
            .index(cfg.validator_index)?
            .build()?;
        secret_key_to_address(key.credential())
    };
    let public_key = signer.public_key();
    let inbound = cfg.addr.to_string();
    let outbound = cfg.addr.to_string();
    println!("
        cast send {VALIDATOR_CONFIG_ADDRESS} \
        \\\n\"addValidator(address newValidatorAddress, bytes32 publicKey, bool active, string calldata inboundAddress, string calldata outboundAddress)\" \
        \\\n\"{validator_address}\" \
        \\\n\"{public_key}\" \
        \\\n\"true\" \
        \\\n\"{inbound}\" \
        \\\n\"{outbound}\" \
        \\\n--private-key {admin_key} \
        \\\n-r 127.0.0.1:8545");
    Ok(())
}
