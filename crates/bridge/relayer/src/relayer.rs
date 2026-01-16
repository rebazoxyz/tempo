//! Core relay logic for the Tempo ↔ Ethereum bridge.
//!
//! The relayer monitors source chain for PacketSent events, waits for finalization,
//! fetches proofs, and submits recvPacket transactions to the destination chain.

use alloy_primitives::{Address, Bytes, U256};
use eyre::{Result, WrapErr};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::ethereum::{EthPacketSentEvent, EthereumClient};
use crate::proofs::{encode_finalization_certificate, encode_ethereum_proof, encode_tempo_proof, packet_commitment_slot};
use crate::tempo::{PacketSentEvent, TempoClient};
use crate::Direction;

/// Base storage slot for packet commitments mapping.
/// This should match the contract's storage layout.
const PACKET_COMMITMENTS_SLOT: u64 = 0;

/// Number of confirmations to wait before considering a block final on Ethereum.
const ETH_CONFIRMATIONS: u64 = 12;

/// Relayer configuration.
#[derive(Clone, Debug)]
pub struct RelayerConfig {
    pub tempo_rpc: String,
    pub eth_rpc: String,
    pub tempo_bridge: Address,
    pub eth_bridge: Address,
    pub private_key: String,
    pub direction: Direction,
    pub poll_interval_secs: u64,
    pub max_retries: u32,
}

/// State tracking for the relayer.
#[derive(Debug, Default)]
struct RelayerState {
    /// Last processed block on Tempo.
    last_tempo_block: u64,
    /// Last processed block on Ethereum.
    last_eth_block: u64,
    /// Last relayed sequence Tempo -> Eth.
    last_tempo_to_eth_sequence: u64,
    /// Last relayed sequence Eth -> Tempo.
    last_eth_to_tempo_sequence: u64,
}

/// The bridge relayer.
pub struct Relayer {
    config: RelayerConfig,
    tempo_client: TempoClient,
    eth_client: EthereumClient,
    state: RelayerState,
}

impl Relayer {
    /// Create a new relayer instance.
    pub async fn new(config: RelayerConfig) -> Result<Self> {
        let tempo_client = TempoClient::new(&config.tempo_rpc, config.tempo_bridge).await?;
        let eth_client =
            EthereumClient::new(&config.eth_rpc, config.eth_bridge, &config.private_key).await?;

        let state = RelayerState::default();

        Ok(Self {
            config,
            tempo_client,
            eth_client,
            state,
        })
    }

    /// Run the relayer main loop.
    pub async fn run(mut self) -> Result<()> {
        info!("Starting relayer main loop");

        self.initialize_state().await?;

        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);

        loop {
            if let Err(e) = self.relay_iteration().await {
                error!(error = %e, "Relay iteration failed");
            }

            sleep(poll_interval).await;
        }
    }

    /// Initialize relayer state from on-chain data.
    async fn initialize_state(&mut self) -> Result<()> {
        self.state.last_tempo_block = self.tempo_client.get_block_number().await?;
        self.state.last_eth_block = self.eth_client.get_block_number().await?;

        info!(
            tempo_block = self.state.last_tempo_block,
            eth_block = self.state.last_eth_block,
            "Initialized relayer state"
        );

        Ok(())
    }

    /// Single iteration of the relay loop.
    async fn relay_iteration(&mut self) -> Result<()> {
        match self.config.direction {
            Direction::TempoToEth => {
                self.relay_tempo_to_eth().await?;
            }
            Direction::EthToTempo => {
                self.relay_eth_to_tempo().await?;
            }
            Direction::Both => {
                self.relay_tempo_to_eth().await?;
                self.relay_eth_to_tempo().await?;
            }
        }

        Ok(())
    }

    /// Relay packets from Tempo to Ethereum.
    async fn relay_tempo_to_eth(&mut self) -> Result<()> {
        let current_block = self.tempo_client.get_block_number().await?;

        if current_block <= self.state.last_tempo_block {
            return Ok(());
        }

        debug!(
            from = self.state.last_tempo_block,
            to = current_block,
            "Scanning Tempo blocks"
        );

        let events = self
            .tempo_client
            .get_packet_sent_events(self.state.last_tempo_block + 1, current_block)
            .await?;

        for event in events {
            if let Err(e) = self.process_tempo_packet(event).await {
                warn!(error = %e, "Failed to process Tempo packet");
            }
        }

        self.state.last_tempo_block = current_block;
        Ok(())
    }

    /// Process a single packet from Tempo.
    async fn process_tempo_packet(&mut self, event: PacketSentEvent) -> Result<()> {
        info!(
            sequence = event.sequence,
            sender = %event.sender,
            recipient = %event.recipient,
            amount = %event.amount,
            block = event.block_number,
            "Processing Tempo packet"
        );

        let finalization = self.wait_for_tempo_finalization(event.block_number).await?;

        let finalization_height = finalization
            .height
            .ok_or_else(|| eyre::eyre!("Finalization missing height"))?;

        let storage_slot = packet_commitment_slot(event.sequence, U256::from(PACKET_COMMITMENTS_SLOT));
        let proof = self
            .tempo_client
            .get_storage_proof(vec![storage_slot], finalization_height)
            .await?;

        let encoded_proof = encode_tempo_proof(&proof, storage_slot)?;
        let encoded_cert = encode_finalization_certificate(&finalization)?;

        self.eth_client
            .update_client(encoded_cert, Bytes::default())
            .await?;

        let tx_hash = self
            .submit_to_eth_with_retry(
                event.sequence,
                event.sender,
                event.recipient,
                event.amount,
                Bytes::from(event.data),
                encoded_proof,
                finalization_height,
            )
            .await?;

        info!(
            sequence = event.sequence,
            tx_hash = %tx_hash,
            "Successfully relayed Tempo -> Eth"
        );

        self.state.last_tempo_to_eth_sequence = event.sequence;
        Ok(())
    }

    /// Wait for a Tempo block to be finalized.
    async fn wait_for_tempo_finalization(
        &self,
        block_number: u64,
    ) -> Result<crate::tempo::CertifiedBlock> {
        info!(block = block_number, "Waiting for Tempo finalization");

        loop {
            if let Some(finalization) = self.tempo_client.get_finalization(block_number).await? {
                info!(
                    block = block_number,
                    epoch = finalization.epoch,
                    view = finalization.view,
                    "Block finalized on Tempo"
                );
                return Ok(finalization);
            }

            sleep(Duration::from_secs(1)).await;
        }
    }

    /// Submit recvPacket to Ethereum with retries.
    async fn submit_to_eth_with_retry(
        &self,
        sequence: u64,
        sender: Address,
        recipient: Address,
        amount: U256,
        data: Bytes,
        proof: Bytes,
        proof_height: u64,
    ) -> Result<alloy_primitives::B256> {
        let mut attempts = 0;

        loop {
            match self
                .eth_client
                .submit_recv_packet(
                    U256::from(sequence),
                    sender,
                    recipient,
                    amount,
                    data.clone(),
                    proof.clone(),
                    proof_height,
                )
                .await
            {
                Ok(tx_hash) => return Ok(tx_hash),
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.config.max_retries {
                        return Err(e).wrap_err("Max retries exceeded for recvPacket");
                    }
                    warn!(
                        attempt = attempts,
                        max = self.config.max_retries,
                        error = %e,
                        "recvPacket failed, retrying"
                    );
                    sleep(Duration::from_secs(2u64.pow(attempts))).await;
                }
            }
        }
    }

    /// Relay packets from Ethereum to Tempo.
    async fn relay_eth_to_tempo(&mut self) -> Result<()> {
        let current_block = self.eth_client.get_block_number().await?;

        let safe_block = current_block.saturating_sub(ETH_CONFIRMATIONS);
        if safe_block <= self.state.last_eth_block {
            return Ok(());
        }

        debug!(
            from = self.state.last_eth_block,
            to = safe_block,
            "Scanning Ethereum blocks"
        );

        let events = self
            .eth_client
            .get_packet_sent_events(self.state.last_eth_block + 1, safe_block)
            .await?;

        for event in events {
            if let Err(e) = self.process_eth_packet(event).await {
                warn!(error = %e, "Failed to process Ethereum packet");
            }
        }

        self.state.last_eth_block = safe_block;
        Ok(())
    }

    /// Process a single packet from Ethereum.
    async fn process_eth_packet(&mut self, event: EthPacketSentEvent) -> Result<()> {
        info!(
            sequence = %event.sequence,
            sender = %event.sender,
            recipient = %event.recipient,
            amount = %event.amount,
            block = event.block_number,
            "Processing Ethereum packet"
        );

        let storage_slot = packet_commitment_slot(
            event.sequence.to::<u64>(),
            U256::from(PACKET_COMMITMENTS_SLOT),
        );
        let proof = self
            .eth_client
            .get_storage_proof(vec![storage_slot], event.block_number)
            .await?;

        let encoded_proof = encode_ethereum_proof(&proof, storage_slot)?;

        let tx_hash = self
            .submit_to_tempo_with_retry(
                event.sequence.to::<u64>(),
                event.sender,
                event.recipient,
                event.amount,
                event.data,
                encoded_proof,
                event.block_number,
            )
            .await?;

        info!(
            sequence = %event.sequence,
            tx_hash = %tx_hash,
            "Successfully relayed Eth -> Tempo"
        );

        self.state.last_eth_to_tempo_sequence = event.sequence.to::<u64>();
        Ok(())
    }

    /// Submit recvPacket to Tempo with retries.
    async fn submit_to_tempo_with_retry(
        &self,
        sequence: u64,
        sender: Address,
        recipient: Address,
        amount: U256,
        data: Bytes,
        proof: Bytes,
        proof_height: u64,
    ) -> Result<alloy_primitives::B256> {
        todo!("Implement Tempo transaction submission")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_creation() {
        let config = RelayerConfig {
            tempo_rpc: "http://localhost:8545".to_string(),
            eth_rpc: "http://localhost:8546".to_string(),
            tempo_bridge: Address::ZERO,
            eth_bridge: Address::ZERO,
            private_key: "0x0000000000000000000000000000000000000000000000000000000000000001"
                .to_string(),
            direction: Direction::Both,
            poll_interval_secs: 12,
            max_retries: 3,
        };

        assert_eq!(config.poll_interval_secs, 12);
        assert_eq!(config.max_retries, 3);
    }
}
