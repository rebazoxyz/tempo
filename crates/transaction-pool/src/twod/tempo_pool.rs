//! Minimal wrapper combining vanilla and 2D pools with simple shared ordering

use std::{iter::Peekable, sync::Arc};

use reth_transaction_pool::ValidPoolTransaction;

use crate::transaction::TempoPooledTransaction;

/// Simple merge iterator that picks highest fee transaction from either pool
///
/// At each step, this iterator:
/// 1. Peeks at the next transaction from both pools
/// 2. Compares their effective tips per gas
/// 3. Returns the transaction with the higher tip
/// 4. Continues until both pools are exhausted
pub struct MergeByTip<I1, I2>
where
    I1: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
    I2: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
{
    vanilla: Peekable<I1>,
    twod: Peekable<I2>,
}

impl<I1, I2> MergeByTip<I1, I2>
where
    I1: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
    I2: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
{
    pub fn new(vanilla: I1, twod: I2) -> Self {
        Self {
            vanilla: vanilla.peekable(),
            twod: twod.peekable(),
        }
    }
}

impl<I1, I2> Iterator for MergeByTip<I1, I2>
where
    I1: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
    I2: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
{
    type Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.vanilla.peek(), self.twod.peek()) {
            (Some(v), Some(t)) => {
                // Compare tips and take from higher
                // Use 0 as base fee for simplicity in POC
                let v_tip = v.effective_tip_per_gas(0).unwrap_or(0);
                let t_tip = t.effective_tip_per_gas(0).unwrap_or(0);

                if v_tip >= t_tip {
                    self.vanilla.next()
                } else {
                    self.twod.next()
                }
            }
            (Some(_), None) => self.vanilla.next(),
            (None, Some(_)) => self.twod.next(),
            (None, None) => None,
        }
    }
}

/// Helper function to create merged iterator from two pools
///
/// Usage:
/// ```
/// let merged = merge_pools(
///     vanilla_pool.best_transactions(),
///     twod_pool.best_transactions(),
/// );
/// for tx in merged {
///     execute(tx);
/// }
/// ```
pub fn merge_pools<I1, I2>(vanilla_iter: I1, twod_iter: I2) -> MergeByTip<I1, I2>
where
    I1: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
    I2: Iterator<Item = Arc<ValidPoolTransaction<TempoPooledTransaction>>>,
{
    MergeByTip::new(vanilla_iter, twod_iter)
}
