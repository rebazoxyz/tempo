//! Ethereum chain client for the bridge relayer.
//!
//! Provides methods to watch for bridge events on Ethereum and submit
//! relay transactions.

use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, B256, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::{Filter, Log},
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolEvent,
};
use eyre::{Result, WrapErr};
use std::str::FromStr;
use tracing::{debug, info, warn};

sol! {
    #[derive(Debug)]
    event PacketSent(
        uint256 indexed sequence,
        address indexed sender,
        address recipient,
        uint256 amount,
        bytes data
    );

    #[derive(Debug)]
    event PacketReceived(
        uint256 indexed sequence,
        address indexed sender,
        address recipient,
        uint256 amount
    );

    #[derive(Debug)]
    function recvPacket(
        uint256 sequence,
        address sender,
        address recipient,
        uint256 amount,
        bytes calldata data,
        bytes calldata proof,
        uint64 proofHeight
    ) external;

    #[derive(Debug)]
    function updateClient(
        bytes calldata finalizationCertificate,
        bytes calldata header
    ) external;

    #[derive(Debug)]
    function getNextSequenceRecv() external view returns (uint256);

    #[derive(Debug)]
    function getLatestHeight() external view returns (uint64);
}

/// Parsed PacketSent event from Ethereum.
#[derive(Clone, Debug)]
pub struct EthPacketSentEvent {
    pub sequence: U256,
    pub sender: Address,
    pub recipient: Address,
    pub amount: U256,
    pub data: Bytes,
    pub block_number: u64,
    pub tx_hash: B256,
    pub log_index: u64,
}

/// Storage proof from eth_getProof.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProofItem {
    pub key: B256,
    pub value: U256,
    pub proof: Vec<Bytes>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProofResponse {
    pub address: Address,
    pub account_proof: Vec<Bytes>,
    pub balance: U256,
    pub code_hash: B256,
    pub nonce: U256,
    pub storage_hash: B256,
    pub storage_proof: Vec<StorageProofItem>,
}

/// Client for interacting with Ethereum chain.
pub struct EthereumClient {
    provider: alloy::providers::RootProvider,
    wallet: EthereumWallet,
    bridge_address: Address,
    signer_address: Address,
}

impl EthereumClient {
    /// Create a new Ethereum client.
    pub async fn new(
        rpc_url: &str,
        bridge_address: Address,
        private_key: &str,
    ) -> Result<Self> {
        let signer: PrivateKeySigner = private_key
            .trim_start_matches("0x")
            .parse()
            .wrap_err("Failed to parse private key")?;
        let signer_address = signer.address();
        let wallet = EthereumWallet::from(signer);

        let provider = ProviderBuilder::new()
            .on_builtin(rpc_url)
            .await
            .wrap_err("Failed to create Ethereum provider")?;

        info!(
            rpc_url = %rpc_url,
            bridge = %bridge_address,
            relayer = %signer_address,
            "Connected to Ethereum"
        );

        Ok(Self {
            provider,
            wallet,
            bridge_address,
            signer_address,
        })
    }

    /// Get the current block number.
    pub async fn get_block_number(&self) -> Result<u64> {
        let block_number = self
            .provider
            .get_block_number()
            .await
            .wrap_err("Failed to get Ethereum block number")?;

        Ok(block_number)
    }

    /// Get PacketSent events from a block range.
    pub async fn get_packet_sent_events(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<EthPacketSentEvent>> {
        let filter = Filter::new()
            .address(self.bridge_address)
            .event_signature(PacketSent::SIGNATURE_HASH)
            .from_block(from_block)
            .to_block(to_block);

        let logs = self
            .provider
            .get_logs(&filter)
            .await
            .wrap_err("Failed to get PacketSent logs from Ethereum")?;

        let events = logs
            .into_iter()
            .filter_map(|log| self.parse_packet_sent_log(log).ok())
            .collect();

        Ok(events)
    }

    fn parse_packet_sent_log(&self, log: Log) -> Result<EthPacketSentEvent> {
        let decoded = PacketSent::decode_log(&log.inner, true)
            .wrap_err("Failed to decode PacketSent event")?;

        Ok(EthPacketSentEvent {
            sequence: decoded.sequence,
            sender: decoded.sender,
            recipient: decoded.recipient,
            amount: decoded.amount,
            data: decoded.data,
            block_number: log.block_number.unwrap_or(0),
            tx_hash: log.transaction_hash.unwrap_or(B256::ZERO),
            log_index: log.log_index.unwrap_or(0),
        })
    }

    /// Get storage proof for a slot at a block.
    pub async fn get_storage_proof(
        &self,
        storage_keys: Vec<B256>,
        block_number: u64,
    ) -> Result<AccountProofResponse> {
        let proof = self
            .provider
            .get_proof(self.bridge_address, storage_keys)
            .block_id(block_number.into())
            .await
            .wrap_err("Failed to get storage proof from Ethereum")?;

        Ok(AccountProofResponse {
            address: proof.address,
            account_proof: proof.account_proof,
            balance: proof.balance,
            code_hash: proof.code_hash,
            nonce: U256::from(proof.nonce),
            storage_hash: proof.storage_hash,
            storage_proof: proof
                .storage_proof
                .into_iter()
                .map(|p| StorageProofItem {
                    key: p.key.as_b256(),
                    value: p.value,
                    proof: p.proof,
                })
                .collect(),
        })
    }

    /// Get block header by number.
    pub async fn get_block_header(&self, block_number: u64) -> Result<Option<Bytes>> {
        let block = self
            .provider
            .get_block_by_number(block_number.into())
            .await
            .wrap_err("Failed to get block header")?;

        if let Some(block) = block {
            let header = block.header;
            let rlp = alloy::rlp::encode(&header);
            Ok(Some(Bytes::from(rlp)))
        } else {
            Ok(None)
        }
    }

    /// Submit a recvPacket transaction to the bridge.
    pub async fn submit_recv_packet(
        &self,
        sequence: U256,
        sender: Address,
        recipient: Address,
        amount: U256,
        data: Bytes,
        proof: Bytes,
        proof_height: u64,
    ) -> Result<B256> {
        let call = recvPacketCall {
            sequence,
            sender,
            recipient,
            amount,
            data,
            proof,
            proofHeight: proof_height,
        };

        let tx = alloy::network::TransactionRequest::default()
            .to(self.bridge_address)
            .input(call.abi_encode().into());

        let pending = self
            .provider
            .send_transaction(tx)
            .await
            .wrap_err("Failed to send recvPacket transaction")?;

        let tx_hash = *pending.tx_hash();
        info!(tx_hash = %tx_hash, sequence = %sequence, "Submitted recvPacket");

        Ok(tx_hash)
    }

    /// Submit a light client update.
    pub async fn update_client(
        &self,
        finalization_certificate: Bytes,
        header: Bytes,
    ) -> Result<B256> {
        let call = updateClientCall {
            finalizationCertificate: finalization_certificate,
            header,
        };

        let tx = alloy::network::TransactionRequest::default()
            .to(self.bridge_address)
            .input(call.abi_encode().into());

        let pending = self
            .provider
            .send_transaction(tx)
            .await
            .wrap_err("Failed to send updateClient transaction")?;

        let tx_hash = *pending.tx_hash();
        info!(tx_hash = %tx_hash, "Submitted light client update");

        Ok(tx_hash)
    }

    /// Get the next expected sequence number on the receiver.
    pub async fn get_next_sequence_recv(&self) -> Result<U256> {
        let call = getNextSequenceRecvCall {};

        let result = self
            .provider
            .call(
                alloy::network::TransactionRequest::default()
                    .to(self.bridge_address)
                    .input(call.abi_encode().into()),
            )
            .await
            .wrap_err("Failed to call getNextSequenceRecv")?;

        let sequence = U256::from_be_slice(&result);
        Ok(sequence)
    }

    /// Get the latest height known to the light client.
    pub async fn get_latest_light_client_height(&self) -> Result<u64> {
        let call = getLatestHeightCall {};

        let result = self
            .provider
            .call(
                alloy::network::TransactionRequest::default()
                    .to(self.bridge_address)
                    .input(call.abi_encode().into()),
            )
            .await
            .wrap_err("Failed to call getLatestHeight")?;

        let height = u64::from_be_bytes(result[24..32].try_into()?);
        Ok(height)
    }

    pub fn bridge_address(&self) -> Address {
        self.bridge_address
    }

    pub fn signer_address(&self) -> Address {
        self.signer_address
    }
}
