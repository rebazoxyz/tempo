//! Tempo chain client for the bridge relayer.
//!
//! Provides methods to interact with Tempo's consensus RPC for finalization
//! certificates and storage proofs.

use alloy_primitives::{Address, B256, U256};
use eyre::{Result, WrapErr};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// A block with a threshold BLS certificate (notarization or finalization).
/// Matches the structure from `tempo_node::rpc::consensus::types::CertifiedBlock`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CertifiedBlock {
    pub epoch: u64,
    pub view: u64,
    pub height: Option<u64>,
    pub digest: B256,
    /// Hex-encoded full notarization or finalization certificate.
    pub certificate: String,
}

/// Query for consensus data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Query {
    Latest,
    Height(u64),
}

/// Consensus state snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusState {
    pub finalized: Option<CertifiedBlock>,
    pub notarized: Option<CertifiedBlock>,
}

/// Consensus event from subscription.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConsensusEvent {
    Notarized {
        #[serde(flatten)]
        block: CertifiedBlock,
        seen: u64,
    },
    Finalized {
        #[serde(flatten)]
        block: CertifiedBlock,
        seen: u64,
    },
    Nullified { epoch: u64, view: u64, seen: u64 },
}

/// Storage proof from eth_getProof RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProof {
    pub key: B256,
    pub value: U256,
    pub proof: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProof {
    pub address: Address,
    pub account_proof: Vec<String>,
    pub balance: U256,
    pub code_hash: B256,
    pub nonce: U256,
    pub storage_hash: B256,
    pub storage_proof: Vec<StorageProof>,
}

/// Client for interacting with Tempo chain.
pub struct TempoClient {
    http_client: HttpClient,
    bridge_address: Address,
}

impl TempoClient {
    /// Create a new Tempo client.
    pub async fn new(rpc_url: &str, bridge_address: Address) -> Result<Self> {
        let http_client = HttpClientBuilder::default()
            .build(rpc_url)
            .wrap_err("Failed to create Tempo HTTP client")?;

        info!(rpc_url = %rpc_url, bridge = %bridge_address, "Connected to Tempo");

        Ok(Self {
            http_client,
            bridge_address,
        })
    }

    /// Get finalization certificate for a specific height.
    pub async fn get_finalization(&self, height: u64) -> Result<Option<CertifiedBlock>> {
        let query = Query::Height(height);
        let result: Option<CertifiedBlock> = self
            .http_client
            .request("consensus_getFinalization", rpc_params![query])
            .await
            .wrap_err("Failed to call consensus_getFinalization")?;

        if let Some(ref block) = result {
            debug!(
                height = height,
                epoch = block.epoch,
                view = block.view,
                digest = %block.digest,
                "Got finalization certificate"
            );
        }

        Ok(result)
    }

    /// Get the latest finalization certificate.
    pub async fn get_latest_finalization(&self) -> Result<Option<CertifiedBlock>> {
        let query = Query::Latest;
        let result: Option<CertifiedBlock> = self
            .http_client
            .request("consensus_getFinalization", rpc_params![query])
            .await
            .wrap_err("Failed to call consensus_getFinalization")?;

        Ok(result)
    }

    /// Get the current consensus state (latest finalized + notarized).
    pub async fn get_latest(&self) -> Result<ConsensusState> {
        let result: ConsensusState = self
            .http_client
            .request("consensus_getLatest", rpc_params![])
            .await
            .wrap_err("Failed to call consensus_getLatest")?;

        Ok(result)
    }

    /// Get storage proof for a specific storage slot at a given block.
    pub async fn get_storage_proof(
        &self,
        storage_keys: Vec<B256>,
        block_number: u64,
    ) -> Result<AccountProof> {
        let block_tag = format!("0x{:x}", block_number);

        let result: AccountProof = self
            .http_client
            .request(
                "eth_getProof",
                rpc_params![self.bridge_address, storage_keys, block_tag],
            )
            .await
            .wrap_err("Failed to call eth_getProof on Tempo")?;

        debug!(
            address = %self.bridge_address,
            storage_hash = %result.storage_hash,
            proof_count = result.storage_proof.len(),
            "Got storage proof from Tempo"
        );

        Ok(result)
    }

    /// Get the current block number.
    pub async fn get_block_number(&self) -> Result<u64> {
        let result: U256 = self
            .http_client
            .request("eth_blockNumber", rpc_params![])
            .await
            .wrap_err("Failed to call eth_blockNumber")?;

        Ok(result.to::<u64>())
    }

    /// Calculate the storage slot for a packet commitment.
    /// This assumes a mapping: `mapping(uint256 => bytes32) public packetCommitments`
    /// at slot `PACKET_COMMITMENTS_SLOT`.
    pub fn packet_commitment_slot(sequence: u64, base_slot: U256) -> B256 {
        use alloy_primitives::keccak256;

        let mut data = [0u8; 64];
        data[24..32].copy_from_slice(&sequence.to_be_bytes());
        data[32..64].copy_from_slice(&base_slot.to_be_bytes::<32>());

        keccak256(data)
    }

    /// Watch for PacketSent events starting from a given block.
    pub async fn get_packet_sent_events(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<PacketSentEvent>> {
        let filter = serde_json::json!({
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", to_block),
            "address": self.bridge_address,
            "topics": [PACKET_SENT_TOPIC]
        });

        let logs: Vec<serde_json::Value> = self
            .http_client
            .request("eth_getLogs", rpc_params![filter])
            .await
            .wrap_err("Failed to get PacketSent logs from Tempo")?;

        let events = logs
            .into_iter()
            .filter_map(|log| parse_packet_sent_log(log).ok())
            .collect();

        Ok(events)
    }

    pub fn bridge_address(&self) -> Address {
        self.bridge_address
    }
}

/// PacketSent event keccak256 topic.
/// keccak256("PacketSent(uint256,address,address,uint256,bytes)")
pub const PACKET_SENT_TOPIC: &str =
    "0x7c0c5c1ff8a4c8f6d4a7b8e5c3a2b1d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4";

/// Parsed PacketSent event.
#[derive(Clone, Debug)]
pub struct PacketSentEvent {
    pub sequence: u64,
    pub sender: Address,
    pub recipient: Address,
    pub amount: U256,
    pub data: Vec<u8>,
    pub block_number: u64,
    pub tx_hash: B256,
    pub log_index: u64,
}

fn parse_packet_sent_log(log: serde_json::Value) -> Result<PacketSentEvent> {
    let block_number = u64::from_str_radix(
        log["blockNumber"]
            .as_str()
            .unwrap_or("0x0")
            .trim_start_matches("0x"),
        16,
    )?;

    let tx_hash = log["transactionHash"]
        .as_str()
        .unwrap_or("0x0000000000000000000000000000000000000000000000000000000000000000")
        .parse::<B256>()?;

    let log_index = u64::from_str_radix(
        log["logIndex"]
            .as_str()
            .unwrap_or("0x0")
            .trim_start_matches("0x"),
        16,
    )?;

    let data = log["data"].as_str().unwrap_or("0x");
    let data_bytes = hex::decode(data.trim_start_matches("0x"))?;

    let topics = log["topics"].as_array();

    let sequence = if let Some(topics) = topics {
        if topics.len() > 1 {
            u64::from_str_radix(
                topics[1]
                    .as_str()
                    .unwrap_or("0x0")
                    .trim_start_matches("0x"),
                16,
            )?
        } else {
            0
        }
    } else {
        0
    };

    let sender = if let Some(topics) = topics {
        if topics.len() > 2 {
            let addr_str = topics[2].as_str().unwrap_or(
                "0x0000000000000000000000000000000000000000000000000000000000000000",
            );
            Address::from_slice(&hex::decode(&addr_str[26..])?)
        } else {
            Address::ZERO
        }
    } else {
        Address::ZERO
    };

    let recipient = if data_bytes.len() >= 32 {
        Address::from_slice(&data_bytes[12..32])
    } else {
        Address::ZERO
    };

    let amount = if data_bytes.len() >= 64 {
        U256::from_be_slice(&data_bytes[32..64])
    } else {
        U256::ZERO
    };

    Ok(PacketSentEvent {
        sequence,
        sender,
        recipient,
        amount,
        data: data_bytes,
        block_number,
        tx_hash,
        log_index,
    })
}
