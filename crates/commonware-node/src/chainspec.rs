use std::sync::Arc;

use eyre::Context as _;
use reth_chainspec::ChainSpec;
use reth_cli::chainspec::ChainSpecParser;

use crate::config::{TEMPO_CHAIN_ID, TEMPO_CHAIN_NAME};

/// Tempo chain spec parser
#[derive(Debug, Clone, Default)]
pub struct Parser;

impl ChainSpecParser for Parser {
    type ChainSpec = ChainSpec;

    // TODO: come up with some good names here? This was "malachite", but
    // that really does not make a whole lot of sense. Calling it "commonware"
    // seems equally odd.
    const SUPPORTED_CHAINS: &'static [&'static str] = &[TEMPO_CHAIN_NAME];

    // XXX: The definition of ChainSpecParser in reth-cli unfortunately requires eyre.
    // Provide a patch to make it more flexible?
    fn parse(s: &str) -> eyre::Result<Arc<Self::ChainSpec>> {
        match s {
            TEMPO_CHAIN_NAME => Ok(Arc::new(tempo_chain_spec())),
            other => read_genesis(other)
                .wrap_err_with(|| format!("failed constructing an eth genesis from `{other}`; either a chain under that name is not known, a file at that path does not exist, or the file is otherwise invalid")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ReadGenesisError {
    #[error("failed to open file for reading")]
    OpenFile(#[from] std::io::Error),
    #[error("failed parsing file contents as genesis")]
    ParseFile(#[from] serde_json::Error),
}

fn read_genesis<P: AsRef<std::path::Path>>(path: P) -> Result<Arc<ChainSpec>, ReadGenesisError> {
    use alloy_genesis::Genesis;
    use reth_chainspec::ChainSpecBuilder;

    let f = std::fs::File::open(path)?;
    let genesis: Genesis = serde_json::from_reader(&f)?;

    // XXX: Flag if the chain clashes with a named chain?
    let chain_id = if genesis.config.chain_id == 0 {
        TEMPO_CHAIN_ID
    } else {
        genesis.config.chain_id
    };
    let chain = reth_chainspec::Chain::from_id(chain_id);

    let chain_spec = ChainSpecBuilder::default()
        .chain(chain)
        .genesis(genesis)
        .paris_activated()
        .shanghai_activated()
        .cancun_activated()
        .build();

    Ok(Arc::new(chain_spec))
}

/// Generates the default tempo chain spec.
//
// FIXME: Replace this by a vetted genesis without test accounts.
fn tempo_chain_spec() -> ChainSpec {
    use alloy_genesis::{Genesis, GenesisAccount};
    use alloy_primitives::{Address, B256, Bytes, U256};
    use reth_chainspec::{Chain, ChainSpecBuilder};

    use maplit::btreemap;
    use maplit::convert_args;

    // Create a basic genesis block
    let genesis = Genesis {
        config: Default::default(),
        nonce: 0x42,
        timestamp: 0x0,
        extra_data: Bytes::from_static(b"SC"),
        gas_limit: 0xa388,
        difficulty: U256::from(0x400000000_u64),
        mix_hash: B256::ZERO,
        coinbase: Address::ZERO,
        number: Some(0),
        alloc: convert_args!(
            keys = |s: &str| s.parse().unwrap(),
            btreemap!(
                    "0x6Be02d1d3665660d22FF9624b7BE0551ee1Ac91b"
                    => GenesisAccount {
                        balance: U256::from_str_radix("4a47e3c12448f4ad000000", 16).unwrap(),
                        ..Default::default()
                    },
                    "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
                    => GenesisAccount {
                        balance: U256::from_str_radix("D3C21BCECCEDA1000000", 16).unwrap(),
                        ..Default::default()
                    },
                    "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                    => GenesisAccount {
                        balance: U256::from_str_radix("D3C21BCECCEDA1000000", 16).unwrap(),
                        ..Default::default()
                    },
                    "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
                    => GenesisAccount {
                        balance: U256::from_str_radix("D3C21BCECCEDA1000000", 16).unwrap(),
                        ..Default::default()
                    },
                    "0x90F79bf6EB2c4f870365E785982E1f101E93b906"
                    => GenesisAccount {
                        balance: U256::from_str_radix("D3C21BCECCEDA1000000", 16).unwrap(),
                        ..Default::default()
                    },
            )
        ),
        ..Default::default()
    };

    ChainSpecBuilder::default()
        .chain(Chain::from_id(TEMPO_CHAIN_ID))
        .genesis(genesis)
        .paris_activated()
        .shanghai_activated()
        .cancun_activated()
        .build()
}
