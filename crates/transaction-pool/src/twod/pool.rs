//! Minimal 2D Nonce Transaction Pool for POC

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use alloy_primitives::TxHash;
use reth_transaction_pool::{
    ValidPoolTransaction,
    error::PoolError,
    identifier::{SenderId, TransactionId},
};

use crate::{
    transaction::TempoPooledTransaction,
    twod::types::{SenderKey, U192},
};

/// 2D Nonce Transaction Pool - Minimal POC Implementation
pub struct TwoDimensionalPool {
    /// Pending transactions ready for execution
    pending: HashMap<SenderKey, BTreeMap<u64, Arc<ValidPoolTransaction<TempoPooledTransaction>>>>,

    /// Queued transactions with nonce gaps
    queued: HashMap<SenderKey, BTreeMap<u64, Arc<ValidPoolTransaction<TempoPooledTransaction>>>>,

    /// Ordering for best transaction selection (sorted by tip, then by TransactionId)
    ordering: BTreeMap<(u128, TransactionId), Arc<ValidPoolTransaction<TempoPooledTransaction>>>,

    /// On-chain nonce state
    nonce_state: HashMap<SenderKey, u64>,

    /// Transaction lookup
    by_hash: HashMap<TxHash, Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
}

impl TwoDimensionalPool {
    /// Create a new empty pool
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            queued: HashMap::new(),
            ordering: BTreeMap::new(),
            nonce_state: HashMap::new(),
            by_hash: HashMap::new(),
        }
    }

    /// Helper to create a SenderId from an Address for POC
    fn sender_id_from_address(addr: alloy_primitives::Address) -> SenderId {
        // Simple POC: use first 8 bytes of address as u64
        let bytes = addr.as_slice();
        let mut id_bytes = [0u8; 8];
        id_bytes.copy_from_slice(&bytes[0..8]);
        SenderId::from(u64::from_le_bytes(id_bytes))
    }

    /// Add a transaction with 2D nonce
    pub fn add_transaction(
        &mut self,
        tx: ValidPoolTransaction<TempoPooledTransaction>,
        nonce_key: U192,
        sequence: u64,
    ) -> Result<TxHash, PoolError> {
        let sender = tx.sender();
        let tx_hash = *tx.hash();
        let sender_key = SenderKey::new(sender, nonce_key);

        // Check if already exists
        if self.by_hash.contains_key(&tx_hash) {
            return Err(PoolError::other(tx_hash, "Transaction already imported"));
        }

        let on_chain_seq = self.get_on_chain_sequence(sender_key);

        // Validate sequence
        if sequence < on_chain_seq {
            return Err(PoolError::other(tx_hash, "Transaction sequence outdated"));
        }

        let tx_arc = Arc::new(tx);
        self.by_hash.insert(tx_hash, tx_arc.clone());

        if sequence == on_chain_seq {
            // Ready for execution
            self.add_to_pending(sender_key, sequence, tx_arc.clone());

            // Add to ordering - use effective tip per gas (or 0 if not available) and transaction ID
            let tip = tx_arc.effective_tip_per_gas(0).unwrap_or(0);
            let sender_id = Self::sender_id_from_address(tx_arc.sender());
            let id = TransactionId::new(sender_id, tx_arc.nonce());
            self.ordering.insert((tip, id), tx_arc);

            // Try to promote queued transactions
            self.promote_queued_chain(sender_key, sequence + 1);
        } else {
            // Has gap - queue it
            self.add_to_queued(sender_key, sequence, tx_arc);
        }

        Ok(tx_hash)
    }

    /// Get best transactions iterator
    pub fn best_transactions(
        &self,
    ) -> impl Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>> + '_ {
        self.ordering.values().rev().cloned()
    }

    /// Handle state change from new block
    pub fn on_canonical_state_change(&mut self, updates: Vec<(SenderKey, u64)>) {
        for (sender_key, new_sequence) in updates {
            self.nonce_state.insert(sender_key, new_sequence);
            self.remove_confirmed_transactions(sender_key, new_sequence);
            self.promote_queued_chain(sender_key, new_sequence);
        }
    }

    /// Set on-chain sequence for initialization
    pub fn set_on_chain_sequence(&mut self, sender_key: SenderKey, sequence: u64) {
        self.nonce_state.insert(sender_key, sequence);
    }

    // === Private helpers ===

    fn get_on_chain_sequence(&self, sender_key: SenderKey) -> u64 {
        self.nonce_state.get(&sender_key).copied().unwrap_or(0)
    }

    fn add_to_pending(
        &mut self,
        sender_key: SenderKey,
        sequence: u64,
        tx: Arc<ValidPoolTransaction<TempoPooledTransaction>>,
    ) {
        self.pending
            .entry(sender_key)
            .or_default()
            .insert(sequence, tx);
    }

    fn add_to_queued(
        &mut self,
        sender_key: SenderKey,
        sequence: u64,
        tx: Arc<ValidPoolTransaction<TempoPooledTransaction>>,
    ) {
        self.queued
            .entry(sender_key)
            .or_default()
            .insert(sequence, tx);
    }

    fn promote_queued_chain(&mut self, sender_key: SenderKey, start_sequence: u64) {
        let mut promotions = Vec::new();
        let mut next_seq = start_sequence;

        if let Some(queued_txs) = self.queued.get_mut(&sender_key) {
            while let Some(tx) = queued_txs.remove(&next_seq) {
                promotions.push((next_seq, tx));
                next_seq += 1;
            }

            if queued_txs.is_empty() {
                self.queued.remove(&sender_key);
            }
        }

        for (seq, tx) in promotions {
            self.add_to_pending(sender_key, seq, tx.clone());

            let tip = tx.effective_tip_per_gas(0).unwrap_or(0);
            let sender_id = Self::sender_id_from_address(tx.sender());
            let id = TransactionId::new(sender_id, tx.nonce());
            self.ordering.insert((tip, id), tx);
        }
    }

    fn remove_confirmed_transactions(&mut self, sender_key: SenderKey, confirmed_seq: u64) {
        if let Some(pending_txs) = self.pending.get_mut(&sender_key) {
            let to_remove: Vec<u64> = pending_txs
                .range(..confirmed_seq)
                .map(|(seq, _)| *seq)
                .collect();

            for seq in to_remove {
                if let Some(tx) = pending_txs.remove(&seq) {
                    let tip = tx.effective_tip_per_gas(0).unwrap_or(0);
                    let sender_id = Self::sender_id_from_address(tx.sender());
                    let id = TransactionId::new(sender_id, tx.nonce());
                    self.ordering.remove(&(tip, id));
                    self.by_hash.remove(tx.hash());
                }
            }

            if pending_txs.is_empty() {
                self.pending.remove(&sender_key);
            }
        }
    }
}

impl Default for TwoDimensionalPool {
    fn default() -> Self {
        Self::new()
    }
}
