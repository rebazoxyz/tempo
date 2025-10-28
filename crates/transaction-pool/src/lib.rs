//! Tempo transaction pool implementation with 2D nonce support.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use reth_transaction_pool::{
    CoinbaseTipOrdering, Pool, TransactionValidationTaskExecutor, blobstore::DiskFileBlobStore,
};

use crate::{transaction::TempoPooledTransaction, validator::TempoTransactionValidator};

pub mod transaction;
pub mod validator;

// 2D nonce support modules
pub mod twod;

// Re-export main types for 2D nonce support
pub use twod::{
    CombinedPool, MergeByTip, SenderKey, TempoCombinedPool, TwoDimensionalPool, U192, merge_pools,
};

// Original pool type
pub type TempoTransactionPool<Client, S = DiskFileBlobStore> = Pool<
    TransactionValidationTaskExecutor<TempoTransactionValidator<Client>>,
    CoinbaseTipOrdering<TempoPooledTransaction>,
    S,
>;
