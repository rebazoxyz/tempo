use alloy::primitives::TxHash;
use alloy::providers::Provider;
use futures::StreamExt;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Statistics tracked by the verification service
#[derive(Debug, Clone)]
pub struct VerificationStats {
    /// Total transactions sent to verification
    pub total_sent: Arc<AtomicU64>,
    /// Transactions confirmed with receipts
    pub confirmed: Arc<AtomicU64>,
    /// Transactions still pending verification
    pub pending: Arc<AtomicU64>,
    /// Transactions that failed verification after max attempts
    pub failed: Arc<AtomicU64>,
}

impl VerificationStats {
    pub fn new() -> Self {
        Self {
            total_sent: Arc::new(AtomicU64::new(0)),
            confirmed: Arc::new(AtomicU64::new(0)),
            pending: Arc::new(AtomicU64::new(0)),
            failed: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn total_sent(&self) -> u64 {
        self.total_sent.load(Ordering::Relaxed)
    }

    pub fn confirmed(&self) -> u64 {
        self.confirmed.load(Ordering::Relaxed)
    }

    pub fn pending(&self) -> u64 {
        self.pending.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }
}

/// Unified verification service that subscribes to blocks and matches pending transactions
pub struct VerificationService<P> {
    provider: P,
    stats: VerificationStats,
    pending_rx: mpsc::UnboundedReceiver<TxHash>,
    pending: HashSet<TxHash>,
}

impl<P> VerificationService<P>
where
    P: Provider + Clone + 'static,
{
    /// Create a new verification service
    pub fn new(
        provider: P,
        stats: VerificationStats,
        pending_rx: mpsc::UnboundedReceiver<TxHash>,
    ) -> Self {
        Self { provider, stats, pending_rx, pending: HashSet::new() }
    }

    /// Run the verification service loop
    pub async fn run(mut self) {
        info!("Starting unified verification service");

        // Subscribe to new blocks
        let block_stream = match self.provider.subscribe_blocks().await {
            Ok(stream) => stream,
            Err(e) => {
                error!("Failed to subscribe to blocks: {}", e);
                return;
            }
        };

        let mut block_stream = block_stream.into_stream();

        loop {
            tokio::select! {
                // Process incoming tx hashes from all sender threads
                Some(tx_hash) = self.pending_rx.recv() => {
                    self.stats.total_sent.fetch_add(1, Ordering::Relaxed);
                    self.pending.insert(tx_hash);
                    self.stats.pending.fetch_add(1, Ordering::Relaxed);
                }

                // Process new blocks
                Some(block_header) = block_stream.next() => {
                    let block_number = block_header.number;

                    // Fetch block with transaction hashes
                    match self.provider.get_block_by_number(block_number.into()).await {
                        Ok(Some(block)) => {
                            let tx_hashes: Vec<TxHash> = block.transactions.hashes().collect();
                            debug!("Block {}: {} transactions", block_number, tx_hashes.len());

                            // Check which pending transactions are in this block
                            let mut confirmed_count = 0;
                            for tx_hash in tx_hashes {
                                if self.pending.remove(&tx_hash) {
                                    self.stats.confirmed.fetch_add(1, Ordering::Relaxed);
                                    self.stats.pending.fetch_sub(1, Ordering::Relaxed);
                                    confirmed_count += 1;
                                    debug!("Transaction {} confirmed in block {}", tx_hash, block_number);
                                }
                            }

                            if confirmed_count > 0 {
                                debug!("Block {}: {} transactions confirmed", block_number, confirmed_count);
                            }
                        }
                        Ok(None) => {
                            warn!("Block {} not found", block_number);
                        }
                        Err(e) => {
                            error!("Error fetching block {}: {}", block_number, e);
                        }
                    }
                }

                // Check if channel is closed (benchmark finished)
                else => {
                    info!("Verification channel closed, shutting down");
                    break;
                }
            }
        }

        // Final stats
        info!(
            "Verification service shutdown - Final stats: Total: {}, Confirmed: {}, Pending: {}, Failed: {}",
            self.stats.total_sent(),
            self.stats.confirmed(),
            self.stats.pending(),
            self.stats.failed()
        );
    }
}

/// Spawn the unified verification service (call this once)
/// Returns a channel that all sender threads use to send pending tx hashes
pub fn spawn_verification_service<P>(
    provider: P,
    stats: VerificationStats,
) -> (mpsc::UnboundedSender<TxHash>, tokio::task::JoinHandle<()>)
where
    P: Provider + Clone + Send + 'static,
{
    // Channel for pending tx hashes from all sender threads
    let (pending_tx, pending_rx) = mpsc::unbounded_channel();

    // Spawn verification service
    let verification_service = VerificationService::new(provider, stats, pending_rx);
    let verification_handle = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime for verification");

        rt.block_on(async move {
            verification_service.run().await;
        });
    });

    (pending_tx, verification_handle)
}
