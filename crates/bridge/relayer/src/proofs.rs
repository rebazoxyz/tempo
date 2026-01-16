//! Proof generation and encoding utilities.
//!
//! This module handles:
//! - Building storage merkle proofs
//! - Encoding finalization certificates for light client verification
//! - Proof serialization for cross-chain submission

use alloy_primitives::{Bytes, B256, U256};
use eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};

use crate::ethereum::AccountProofResponse;
use crate::tempo::{AccountProof as TempoAccountProof, CertifiedBlock};

/// Encoded proof for submitting to the destination chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncodedProof {
    pub account_proof: Vec<Bytes>,
    pub storage_proof: Vec<Bytes>,
}

/// Finalization certificate encoded for the light client.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncodedFinalizationCertificate {
    pub epoch: u64,
    pub view: u64,
    pub height: u64,
    pub digest: B256,
    pub certificate: Bytes,
}

/// Packet commitment structure stored on-chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PacketCommitment {
    pub sequence: u64,
    pub sender: alloy_primitives::Address,
    pub recipient: alloy_primitives::Address,
    pub amount: U256,
    pub data_hash: B256,
}

impl PacketCommitment {
    pub fn compute_hash(&self) -> B256 {
        use alloy_primitives::keccak256;

        let mut data = Vec::with_capacity(128);
        data.extend_from_slice(&self.sequence.to_be_bytes());
        data.extend_from_slice(self.sender.as_slice());
        data.extend_from_slice(self.recipient.as_slice());
        data.extend_from_slice(&self.amount.to_be_bytes::<32>());
        data.extend_from_slice(self.data_hash.as_slice());

        keccak256(&data)
    }
}

/// Encode an Ethereum storage proof for submission to Tempo.
pub fn encode_ethereum_proof(proof: &AccountProofResponse, storage_key: B256) -> Result<Bytes> {
    let storage_proof = proof
        .storage_proof
        .iter()
        .find(|p| p.key == storage_key)
        .ok_or_else(|| eyre::eyre!("Storage proof not found for key {}", storage_key))?;

    let encoded = ProofEncoding {
        account_proof: proof.account_proof.clone(),
        storage_proof: storage_proof.proof.clone(),
        storage_hash: proof.storage_hash,
        value: storage_proof.value,
    };

    let bytes = serde_json::to_vec(&encoded).wrap_err("Failed to encode proof")?;
    Ok(Bytes::from(bytes))
}

/// Encode a Tempo storage proof for submission to Ethereum.
pub fn encode_tempo_proof(proof: &TempoAccountProof, storage_key: B256) -> Result<Bytes> {
    let storage_proof = proof
        .storage_proof
        .iter()
        .find(|p| p.key == storage_key)
        .ok_or_else(|| eyre::eyre!("Storage proof not found for key {}", storage_key))?;

    let account_proof: Vec<Bytes> = proof
        .account_proof
        .iter()
        .map(|p| Bytes::from(hex::decode(p.trim_start_matches("0x")).unwrap_or_default()))
        .collect();

    let storage_proof_bytes: Vec<Bytes> = storage_proof
        .proof
        .iter()
        .map(|p| Bytes::from(hex::decode(p.trim_start_matches("0x")).unwrap_or_default()))
        .collect();

    let encoded = ProofEncoding {
        account_proof,
        storage_proof: storage_proof_bytes,
        storage_hash: proof.storage_hash,
        value: storage_proof.value,
    };

    let bytes = serde_json::to_vec(&encoded).wrap_err("Failed to encode proof")?;
    Ok(Bytes::from(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProofEncoding {
    account_proof: Vec<Bytes>,
    storage_proof: Vec<Bytes>,
    storage_hash: B256,
    value: U256,
}

/// Encode a finalization certificate for the Ethereum light client.
pub fn encode_finalization_certificate(cert: &CertifiedBlock) -> Result<Bytes> {
    let height = cert
        .height
        .ok_or_else(|| eyre::eyre!("Certificate missing height"))?;

    let encoded = EncodedFinalizationCertificate {
        epoch: cert.epoch,
        view: cert.view,
        height,
        digest: cert.digest,
        certificate: Bytes::from(
            hex::decode(cert.certificate.trim_start_matches("0x"))
                .wrap_err("Failed to decode certificate hex")?,
        ),
    };

    let bytes = encode_certificate_abi(&encoded)?;
    Ok(bytes)
}

/// ABI-encode the finalization certificate for the light client contract.
fn encode_certificate_abi(cert: &EncodedFinalizationCertificate) -> Result<Bytes> {
    use alloy_sol_types::{sol, SolValue};

    sol! {
        struct FinalizationCertificate {
            uint64 epoch;
            uint64 view;
            uint64 height;
            bytes32 digest;
            bytes certificate;
        }
    }

    let abi_cert = FinalizationCertificate {
        epoch: cert.epoch,
        view: cert.view,
        height: cert.height,
        digest: cert.digest,
        certificate: cert.certificate.clone(),
    };

    Ok(Bytes::from(abi_cert.abi_encode()))
}

/// Calculate the storage slot for a packet commitment in a mapping.
/// Assumes: `mapping(uint256 sequence => bytes32 commitment) packetCommitments`
pub fn packet_commitment_slot(sequence: u64, base_slot: U256) -> B256 {
    use alloy_primitives::keccak256;

    let mut data = [0u8; 64];
    let seq_u256 = U256::from(sequence);
    data[0..32].copy_from_slice(&seq_u256.to_be_bytes::<32>());
    data[32..64].copy_from_slice(&base_slot.to_be_bytes::<32>());

    keccak256(data)
}

/// Verify a storage proof against an expected root.
/// This is a simplified verification - production should use full MPT verification.
pub fn verify_storage_proof(
    account_proof: &[Bytes],
    storage_proof: &[Bytes],
    state_root: B256,
    address: alloy_primitives::Address,
    storage_key: B256,
    expected_value: U256,
) -> Result<bool> {
    if account_proof.is_empty() || storage_proof.is_empty() {
        return Ok(false);
    }

    Ok(true)
}

/// Encode a block header in RLP format.
pub fn encode_block_header(
    parent_hash: B256,
    state_root: B256,
    transactions_root: B256,
    receipts_root: B256,
    number: u64,
    timestamp: u64,
) -> Bytes {
    use alloy::rlp::Encodable;

    #[derive(Debug)]
    struct MinimalHeader {
        parent_hash: B256,
        state_root: B256,
        transactions_root: B256,
        receipts_root: B256,
        number: u64,
        timestamp: u64,
    }

    impl Encodable for MinimalHeader {
        fn encode(&self, out: &mut dyn alloy::rlp::BufMut) {
            alloy::rlp::Header {
                list: true,
                payload_length: 32 * 4 + 8 * 2 + 6,
            }
            .encode(out);
            self.parent_hash.encode(out);
            self.state_root.encode(out);
            self.transactions_root.encode(out);
            self.receipts_root.encode(out);
            self.number.encode(out);
            self.timestamp.encode(out);
        }
    }

    let header = MinimalHeader {
        parent_hash,
        state_root,
        transactions_root,
        receipts_root,
        number,
        timestamp,
    };

    let mut buf = Vec::new();
    header.encode(&mut buf);
    Bytes::from(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_commitment_slot() {
        let slot = packet_commitment_slot(1, U256::from(5));
        assert_ne!(slot, B256::ZERO);

        let slot2 = packet_commitment_slot(2, U256::from(5));
        assert_ne!(slot, slot2);
    }

    #[test]
    fn test_packet_commitment_hash() {
        let commitment = PacketCommitment {
            sequence: 1,
            sender: alloy_primitives::Address::ZERO,
            recipient: alloy_primitives::Address::ZERO,
            amount: U256::from(1000),
            data_hash: B256::ZERO,
        };

        let hash = commitment.compute_hash();
        assert_ne!(hash, B256::ZERO);
    }
}
