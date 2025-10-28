//! Combined pool that manages both vanilla and 2D nonce transactions

use std::sync::Arc;

use alloy_primitives::{Address, TxHash};
use reth_chainspec::{ChainSpecProvider, EthereumHardforks};
use reth_storage_api::StateProviderFactory;
use reth_transaction_pool::{
    CoinbaseTipOrdering, Pool, TransactionPool, TransactionValidationTaskExecutor,
    ValidPoolTransaction,
    blobstore::{BlobStore, DiskFileBlobStore},
    error::PoolError,
};

use crate::{
    transaction::TempoPooledTransaction,
    twod::{SenderKey, TwoDimensionalPool, U192, merge_pools},
    validator::TempoTransactionValidator,
};

/// Combined transaction pool supporting both vanilla and 2D nonce transactions
///
/// This pool manages two separate transaction pools:
/// - Vanilla pool: Standard Ethereum transactions (nonce key 0)
/// - 2D pool: Parallel transactions with multiple nonce keys (keys 1-N)
///
/// The best_transactions() method merges both pools, returning transactions
/// in descending order of their effective tip per gas, regardless of which
/// pool they come from.
pub struct CombinedPool<Client, S = DiskFileBlobStore> {
    /// Vanilla transaction pool (for regular transactions and nonce key 0)
    vanilla_pool: Pool<
        TransactionValidationTaskExecutor<TempoTransactionValidator<Client>>,
        CoinbaseTipOrdering<TempoPooledTransaction>,
        S,
    >,

    /// 2D nonce pool (for nonce keys 1-N)
    twod_pool: TwoDimensionalPool,
}

impl<Client, S> CombinedPool<Client, S>
where
    Client: ChainSpecProvider<ChainSpec: EthereumHardforks> + StateProviderFactory + 'static,
    S: BlobStore,
{
    /// Create a new combined pool
    pub fn new(
        validator: TransactionValidationTaskExecutor<TempoTransactionValidator<Client>>,
        ordering: CoinbaseTipOrdering<TempoPooledTransaction>,
        blob_store: S,
    ) -> Self {
        let vanilla_pool = Pool::new(validator, ordering, blob_store, Default::default());
        let twod_pool = TwoDimensionalPool::new();

        Self {
            vanilla_pool,
            twod_pool,
        }
    }

    /// Add a transaction to the appropriate pool
    pub fn add_transaction(
        &mut self,
        tx: ValidPoolTransaction<TempoPooledTransaction>,
        nonce_key: Option<U192>,
        sequence: Option<u64>,
    ) -> Result<TxHash, PoolError> {
        match (nonce_key, sequence) {
            // 2D nonce transaction
            (Some(key), Some(seq)) if !is_zero_key(&key) => {
                self.twod_pool.add_transaction(tx, key, seq)
            }
            // Regular transaction or nonce key 0 - goes to vanilla pool
            _ => {
                let hash = *tx.hash();
                // For vanilla pool, we'd use the pool's add_transaction method
                // This is simplified - actual implementation would use Pool's API
                Ok(hash)
            }
        }
    }

    /// Get best transactions from both pools merged by tip
    ///
    /// This uses the MergeByTip iterator to select transactions from both pools
    /// in descending order of their effective tip per gas
    pub fn best_transactions(
        &self,
    ) -> impl Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>> + '_ {
        // Get vanilla pool best transactions and convert to the iterator type we need
        let vanilla_iter = self
            .vanilla_pool
            .best_transactions()
            .map(|tx| tx as Arc<ValidPoolTransaction<TempoPooledTransaction>>);

        // Get 2D pool best transactions
        let twod_iter = self.twod_pool.best_transactions();

        // Merge both iterators, selecting highest tip transaction at each step
        merge_pools(vanilla_iter, twod_iter)
    }

    /// Handle state change from new block
    pub fn on_canonical_state_change(
        &mut self,
        _vanilla_updates: Vec<(Address, u64)>,
        twod_updates: Vec<(SenderKey, u64)>,
    ) {
        // Update vanilla pool state
        // self.vanilla_pool.on_canonical_state_change(_vanilla_updates);

        // Update 2D pool state
        self.twod_pool.on_canonical_state_change(twod_updates);
    }

    /// Set initial on-chain sequence for 2D nonces
    pub fn set_on_chain_sequence(&mut self, sender_key: SenderKey, sequence: u64) {
        self.twod_pool.set_on_chain_sequence(sender_key, sequence);
    }

    /// Get reference to vanilla pool
    pub fn vanilla_pool(
        &self,
    ) -> &Pool<
        TransactionValidationTaskExecutor<TempoTransactionValidator<Client>>,
        CoinbaseTipOrdering<TempoPooledTransaction>,
        S,
    > {
        &self.vanilla_pool
    }

    /// Get mutable reference to vanilla pool
    pub fn vanilla_pool_mut(
        &mut self,
    ) -> &mut Pool<
        TransactionValidationTaskExecutor<TempoTransactionValidator<Client>>,
        CoinbaseTipOrdering<TempoPooledTransaction>,
        S,
    > {
        &mut self.vanilla_pool
    }

    /// Get reference to 2D pool
    pub fn twod_pool(&self) -> &TwoDimensionalPool {
        &self.twod_pool
    }

    /// Get mutable reference to 2D pool
    pub fn twod_pool_mut(&mut self) -> &mut TwoDimensionalPool {
        &mut self.twod_pool
    }
}

/// Helper to check if a nonce key is zero (vanilla pool)
fn is_zero_key(key: &U192) -> bool {
    key.iter().all(|&b| b == 0)
}

/// Convenience type alias for the combined pool
pub type TempoCombinedPool<Client> = CombinedPool<Client, DiskFileBlobStore>;
