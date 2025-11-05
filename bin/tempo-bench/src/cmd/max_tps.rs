use alloy::{
    eips::BlockNumberOrTag::Latest,
    network::{Ethereum, EthereumWallet, Network, TransactionBuilder, TxSignerSync},
    primitives::{Address, BlockNumber, ChainId, TxKind, U256},
    providers::{
        PendingTransactionBuilder, Provider, ProviderBuilder, RootProvider,
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
        },
    },
    sol,
    sol_types::{SolCall, SolEvent},
    transports::http::reqwest::Url,
};
use alloy_consensus::{SignableTransaction, TxLegacy, transaction::RlpEcdsaEncodableTx};
use alloy_signer_local::{MnemonicBuilder, PrivateKeySigner, coins_bip39::English};
use clap::Parser;
use core_affinity::CoreId;
use eyre::{Context, OptionExt, ensure};
use governor::{Quota, RateLimiter};
use indicatif::ProgressBar;
use rand::random;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rlimit::Resource;
use serde::Serialize;
use simple_tqdm::ParTqdm;
use std::{
    fs::File,
    io::BufWriter,
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use tempo_chainspec::spec::TEMPO_BASE_FEE;
use tempo_contracts::precompiles::{
    IRolesAuth, IStablecoinExchange::IStablecoinExchangeInstance, ITIP20, ITIP20::ITIP20Instance,
    ITIP20Factory, LINKING_USD_ADDRESS, STABLECOIN_EXCHANGE_ADDRESS, TIP20_FACTORY_ADDRESS,
};
use tempo_precompiles::{
    stablecoin_exchange::MIN_ORDER_AMOUNT,
    tip20::{ISSUER_ROLE, token_id_to_address},
};
use tokio::time::timeout;

sol! {
    interface ERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
    }
}

/// Run maximum TPS throughput benchmarking
#[derive(Parser, Debug)]
pub struct MaxTpsArgs {
    /// Target transactions per second
    #[arg(short, long)]
    tps: u64,

    /// Test duration in seconds
    #[arg(short, long, default_value = "30")]
    duration: u64,

    /// Number of accounts for pre-generation
    #[arg(short, long, default_value = "100")]
    accounts: u64,

    /// Number of workers to send transactions
    #[arg(short, long, default_value = "10")]
    workers: usize,

    /// Mnemonic for generating accounts
    #[arg(
        short,
        long,
        default_value = "test test test test test test test test test test test junk"
    )]
    mnemonic: String,

    /// Chain ID
    #[arg(long, default_value = "1337")]
    chain_id: u64,

    /// Token address used when creating TIP20 transfer calldata
    #[arg(long, default_value = "0x20c0000000000000000000000000000000000000")]
    token_address: Address,

    /// Target URLs for network connections
    #[arg(long, default_values_t = vec!["http://localhost:8545".to_string()])]
    target_urls: Vec<String>,

    /// Total network connections
    #[arg(long, default_value = "100")]
    total_connections: u64,

    /// Disable binding worker threads to specific CPU cores, letting the OS scheduler handle placement.
    #[arg(long)]
    disable_thread_pinning: bool,

    /// File descriptor limit to set
    #[arg(long)]
    fd_limit: Option<u64>,

    /// Node commit SHA for metadata
    #[arg(long)]
    node_commit_sha: Option<String>,

    /// Build profile for metadata (e.g., "release", "debug", "maxperf")
    #[arg(long)]
    build_profile: Option<String>,

    /// Benchmark mode for metadata (e.g., "max_tps", "stress_test")
    #[arg(long)]
    benchmark_mode: Option<String>,
}

impl MaxTpsArgs {
    pub async fn run(self) -> eyre::Result<()> {
        // Set file descriptor limit if provided
        if let Some(fd_limit) = self.fd_limit {
            increase_nofile_limit(fd_limit).context("Failed to increase nofile limit")?;
        }

        let target_urls: Vec<Url> = self
            .target_urls
            .iter()
            .map(|s| {
                s.parse::<Url>()
                    .wrap_err_with(|| format!("failed to parse `{s}` as URL"))
            })
            .collect::<eyre::Result<Vec<_>>>()
            .wrap_err("failed parsing input target URLs")?;

        // Generate all transactions
        let total_txs = self.tps * self.duration;
        let transactions = Arc::new(
            generate_transactions(
                total_txs,
                self.accounts,
                &self.mnemonic,
                self.chain_id,
                self.token_address,
                &target_urls[0],
            )
            .await
            .context("Failed to generate transactions")?,
        );

        // Get first block height before sending transactions
        let provider = ProviderBuilder::new().connect_http(target_urls[0].clone());
        let start_block = provider
            .get_block(Latest.into())
            .await?
            .ok_or_eyre("failed to fetch start block")?;
        let start_block_number = start_block.header.number;

        // Create shared transaction counter and monitoring
        let tx_counter = Arc::new(AtomicU64::new(0));

        // Spawn monitoring thread for TPS tracking
        let _monitor_handle = monitor_tps(tx_counter.clone());

        // Spawn workers and send transactions
        send_transactions(
            transactions,
            self.workers,
            self.total_connections,
            target_urls.clone(),
            self.tps,
            self.disable_thread_pinning,
            tx_counter,
        )
        .context("Failed to send transactions")?;

        // Wait for all sender threads to finish
        std::thread::sleep(Duration::from_secs(self.duration));
        println!("Finished sending transactions");

        let end_block = provider
            .get_block(Latest.into())
            .await?
            .ok_or_eyre("failed to fetch start block")?;
        let end_block_number = end_block.header.number;

        generate_report(
            &target_urls[0].clone(),
            start_block_number,
            end_block_number,
            &self,
        )
        .await?;

        Ok(())
    }
}

fn send_transactions(
    transactions: Arc<Vec<Vec<u8>>>,
    num_workers: usize,
    _num_connections: u64,
    target_urls: Vec<Url>,
    tps: u64,
    disable_thread_pinning: bool,
    tx_counter: Arc<AtomicU64>,
) -> eyre::Result<()> {
    // Get available cores
    let core_ids =
        core_affinity::get_core_ids().ok_or_else(|| eyre::eyre!("Failed to get core IDs"))?;
    println!("Detected {} effective cores.", core_ids.len());

    let num_sender_threads = num_workers.min(core_ids.len());
    let chunk_size = transactions.len().div_ceil(num_sender_threads);

    // Create a shared rate limiter for all threads
    let rate_limiter = Arc::new(RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(tps as u32).unwrap(),
    )));

    for thread_id in 0..num_sender_threads {
        if !disable_thread_pinning {
            let core_id = core_ids[thread_id % core_ids.len()];
            pin_thread(core_id);
        }

        // Segment transactions
        let rate_limiter = rate_limiter.clone();
        let transactions = transactions.clone();
        let target_urls = target_urls.to_vec();
        let tx_counter = tx_counter.clone();
        let start = thread_id * chunk_size;
        let end = (start + chunk_size).min(transactions.len());

        // Spawn thread and send transactions over specified duration
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to build tokio runtime");

            rt.block_on(async {
                // TODO: Send txs from multiple senders
                // Create multiple connections for this thread
                // let mut providers = Vec::new();
                // for i in 0..num_connections {
                //     println!("{i:?}");
                //     let url = &target_urls[(i as usize) % target_urls.len()];
                //     let provider = ProviderBuilder::new().connect_http(url.clone());
                //     providers.push(provider);
                // }

                let provider = ProviderBuilder::new().connect_http(target_urls[0].clone());
                for tx_bytes in transactions[start..end].iter() {
                    rate_limiter.until_ready().await;

                    match timeout(
                        Duration::from_secs(1),
                        provider.send_raw_transaction(tx_bytes),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {
                            tx_counter.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(Err(e)) => eprintln!("Failed to send transaction: {e}"),
                        Err(_) => eprintln!("Tx send timed out"),
                    }
                }
            });
        });
    }

    Ok(())
}

async fn generate_transactions(
    total_txs: u64,
    num_accounts: u64,
    mnemonic: &str,
    chain_id: u64,
    token_address: Address,
    rpc_url: &Url,
) -> eyre::Result<Vec<Vec<u8>>> {
    let (exchange, quote, base1, base2) = dex::setup(rpc_url.clone(), mnemonic).await?;

    println!("Generating {num_accounts} accounts...");
    let signers: Vec<PrivateKeySigner> = (0..num_accounts as u32)
        .into_par_iter()
        .tqdm()
        .map(|i| -> eyre::Result<PrivateKeySigner> {
            let signer = MnemonicBuilder::<English>::default()
                .phrase(mnemonic)
                .index(i)?
                .build()?;
            Ok(signer)
        })
        .collect::<eyre::Result<Vec<_>>>()?;

    let txs_per_sender = total_txs / num_accounts;
    ensure!(
        txs_per_sender > 0,
        "txs per sender is 0, increase tps or decrease senders"
    );

    // Fetch current nonces for all accounts
    let provider = ProviderBuilder::new().connect_http(rpc_url.clone());
    println!("Fetching nonces for {} accounts...", signers.len());

    let mut params = Vec::new();
    for signer in signers {
        let address = signer.address();
        let current_nonce = provider
            .get_transaction_count(address)
            .await
            .context("Failed to get transaction count")?;

        for i in 0..txs_per_sender {
            params.push((signer.clone(), current_nonce + i));
        }
    }

    let transactions: Vec<Vec<u8>> = params
        .into_par_iter()
        .tqdm()
        .map(|(signer, nonce)| match random::<u32>() % 6u32 {
            0 => dex::place(&exchange, signer, nonce, chain_id, base1),
            1 => dex::place(&exchange, signer, nonce, chain_id, base2),
            2 => dex::swap_in(&exchange, signer, nonce, chain_id, base1, quote),
            3 => dex::swap_in(&exchange, signer, nonce, chain_id, base2, quote),
            4 | 5 => tip20::transfer(signer, nonce, chain_id, token_address),
            v => unreachable!("Number {v} is outside the random range"),
        })
        .collect::<eyre::Result<Vec<_>>>()?;

    println!("Generated {} transactions", transactions.len());
    Ok(transactions)
}

mod dex {
    use super::*;

    type DexProvider = FillProvider<
        JoinFill<
            JoinFill<
                alloy::providers::Identity,
                JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
            >,
            WalletFiller<EthereumWallet>,
        >,
        RootProvider,
    >;

    pub(super) async fn setup(
        url: Url,
        mnemonic: &str,
    ) -> eyre::Result<(
        IStablecoinExchangeInstance<DexProvider>,
        Address,
        Address,
        Address,
    )> {
        println!("Sending DEX setup transactions...");

        let tx_count = ProgressBar::new(12);
        tx_count.tick();

        // Setup HTTP provider with a test wallet
        let wallet = MnemonicBuilder::from_phrase(mnemonic).build()?;
        let caller = wallet.address();
        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        let base1 = setup_test_token(provider.clone(), caller, &tx_count).await?;
        let base2 = setup_test_token(provider.clone(), caller, &tx_count).await?;

        let quote = ITIP20Instance::new(token_id_to_address(0), provider.clone());

        let exchange = tempo_contracts::precompiles::IStablecoinExchange::new(
            STABLECOIN_EXCHANGE_ADDRESS,
            provider.clone(),
        );

        let mint_amount = U256::from(1000000000u128);

        await_receipts(
            &mut vec![
                exchange.createPair(*base1.address()).send().await?,
                exchange.createPair(*base2.address()).send().await?,
                base1.mint(caller, mint_amount).send().await?,
                base2.mint(caller, mint_amount).send().await?,
                quote.mint(caller, mint_amount).send().await?,
                base1
                    .approve(STABLECOIN_EXCHANGE_ADDRESS, U256::MAX)
                    .send()
                    .await?,
                base2
                    .approve(STABLECOIN_EXCHANGE_ADDRESS, U256::MAX)
                    .send()
                    .await?,
                quote
                    .approve(STABLECOIN_EXCHANGE_ADDRESS, U256::MAX)
                    .send()
                    .await?,
            ],
            &tx_count,
        )
        .await?;

        Ok((
            exchange,
            *quote.address(),
            *base1.address(),
            *base2.address(),
        ))
    }

    pub(super) fn place<P, N>(
        exchange: &IStablecoinExchangeInstance<P, N>,
        signer: PrivateKeySigner,
        nonce: u64,
        chain_id: ChainId,
        token_address: Address,
    ) -> eyre::Result<Vec<u8>>
    where
        N: Network<
            UnsignedTx: SignableTransaction<alloy::signers::Signature> + RlpEcdsaEncodableTx,
        >,
        P: Provider<N>,
    {
        let min_order_amount = MIN_ORDER_AMOUNT;

        // Place an order at exactly the dust limit (should succeed)
        let mut tx = exchange
            .place(token_address, min_order_amount, true, 0)
            .into_transaction_request()
            .with_gas_limit(30000)
            .with_gas_price(TEMPO_BASE_FEE as u128)
            .with_chain_id(chain_id)
            .with_nonce(nonce)
            .build_unsigned()?;

        let signature = signer
            .sign_transaction_sync(&mut tx)
            .map_err(|e| eyre::eyre!("Failed to sign transaction: {e}"))?;
        let mut payload = Vec::new();
        tx.into_signed(signature).eip2718_encode(&mut payload);
        Ok(payload)
    }

    pub(super) fn swap_in<P, N>(
        exchange: &IStablecoinExchangeInstance<P, N>,
        signer: PrivateKeySigner,
        nonce: u64,
        chain_id: ChainId,
        token_in: Address,
        token_out: Address,
    ) -> eyre::Result<Vec<u8>>
    where
        N: Network<
            UnsignedTx: SignableTransaction<alloy::signers::Signature> + RlpEcdsaEncodableTx,
        >,
        P: Provider<N>,
    {
        let min_order_amount = MIN_ORDER_AMOUNT;

        // Place an order at exactly the dust limit (should succeed)
        let mut tx = exchange
            .swapExactAmountIn(token_in, token_out, min_order_amount, min_order_amount)
            .into_transaction_request()
            .with_gas_limit(30000)
            .with_gas_price(TEMPO_BASE_FEE as u128)
            .with_chain_id(chain_id)
            .with_nonce(nonce)
            .build_unsigned()?;

        let signature = signer
            .sign_transaction_sync(&mut tx)
            .map_err(|e| eyre::eyre!("Failed to sign transaction: {e}"))?;
        let mut payload = Vec::new();
        tx.into_signed(signature).eip2718_encode(&mut payload);
        Ok(payload)
    }

    /// Creates a test TIP20 token with issuer role granted to the caller
    async fn setup_test_token<P>(
        provider: P,
        caller: Address,
        tx_count: &ProgressBar,
    ) -> eyre::Result<ITIP20Instance<impl Clone + Provider>>
    where
        P: Provider + Clone,
    {
        let factory = ITIP20Factory::new(TIP20_FACTORY_ADDRESS, provider.clone());
        let receipt = factory
            .createToken(
                "Test".to_owned(),
                "TEST".to_owned(),
                "USD".to_owned(),
                LINKING_USD_ADDRESS,
                caller,
            )
            .send()
            .await?
            .get_receipt()
            .await?;
        tx_count.inc(1);
        let event = ITIP20Factory::TokenCreated::decode_log(&receipt.logs()[0].inner)?;

        let token_addr = token_id_to_address(event.tokenId.to());
        let token = ITIP20::new(token_addr, provider.clone());
        let roles = IRolesAuth::new(*token.address(), provider);

        roles
            .grantRole(*ISSUER_ROLE, caller)
            .send()
            .await?
            .get_receipt()
            .await?;
        tx_count.inc(1);

        Ok(token)
    }

    async fn await_receipts(
        pending_txs: &mut Vec<PendingTransactionBuilder<Ethereum>>,
        tx_count: &ProgressBar,
    ) -> eyre::Result<()> {
        for tx in pending_txs.drain(..) {
            let receipt = tx.get_receipt().await?;
            tx_count.inc(1);
            assert!(receipt.status());
        }

        Ok(())
    }
}

mod tip20 {
    use super::*;

    pub(super) fn transfer(
        signer: PrivateKeySigner,
        nonce: u64,
        chain_id: ChainId,
        token_address: Address,
    ) -> eyre::Result<Vec<u8>> {
        let mut tx = TxLegacy {
            chain_id: Some(chain_id),
            nonce,
            gas_price: TEMPO_BASE_FEE as u128,
            gas_limit: 30000,
            to: TxKind::Call(token_address),
            value: U256::ZERO,
            input: ERC20::transferCall {
                to: Address::random(),
                amount: U256::ONE,
            }
            .abi_encode()
            .into(),
        };

        let signature = signer
            .sign_transaction_sync(&mut tx)
            .map_err(|e| eyre::eyre!("Failed to sign transaction: {e}"))?;
        let mut payload = Vec::new();
        tx.into_signed(signature).eip2718_encode(&mut payload);
        Ok(payload)
    }
}

pub fn increase_nofile_limit(min_limit: u64) -> eyre::Result<u64> {
    let (soft, hard) = Resource::NOFILE.get()?;
    println!("[*] At startup, file descriptor limit:      soft = {soft}, hard = {hard}");

    if hard < min_limit {
        panic!(
            "[!] File descriptor hard limit is too low. Please increase it to at least {min_limit}."
        );
    }

    if soft != hard {
        Resource::NOFILE.set(hard, hard)?; // Just max things out to give us plenty of overhead.
        let (soft, hard) = Resource::NOFILE.get()?;
        println!("[+] After increasing file descriptor limit: soft = {soft}, hard = {hard}");
    }

    Ok(soft)
}

/// Pin the current thread to the given core ID if enabled.
/// Panics if the thread fails to pin.
pub fn pin_thread(core_id: CoreId) {
    if !core_affinity::set_for_current(core_id) {
        panic!(
            "[!] Failed to pin thread to core {}. Try disabling thread_pinning in your config.",
            core_id.id
        );
    }
}

#[derive(Serialize)]
struct BenchmarkedBlock {
    number: BlockNumber,
    tx_count: usize,
    gas_used: u64,
    timestamp: u64,
    latency_ms: Option<u64>,
}

#[derive(Serialize)]
struct BenchmarkMetadata {
    target_tps: u64,
    run_duration_secs: u64,
    num_accounts: u64,
    num_workers: usize,
    chain_id: u64,
    total_connections: u64,
    start_block: BlockNumber,
    end_block: BlockNumber,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_commit_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
}

#[derive(Serialize)]
struct BenchmarkReport {
    metadata: BenchmarkMetadata,
    blocks: Vec<BenchmarkedBlock>,
}

pub async fn generate_report(
    rpc_url: &Url,
    start_block: BlockNumber,
    end_block: BlockNumber,
    args: &MaxTpsArgs,
) -> eyre::Result<()> {
    let provider = ProviderBuilder::new().connect_http(rpc_url.clone());

    let mut last_block_timestamp: Option<u64> = None;

    let mut benchmarked_blocks = Vec::new();

    for number in start_block..=end_block {
        let block = provider
            .get_block(number.into())
            .await?
            .expect("we should always have this block number");
        let receipts = provider
            .get_block_receipts(number.into())
            .await?
            .expect("there should always be at least one receipt");
        let timestamp = block.header.timestamp;

        let latency_ms = last_block_timestamp.map(|last| (timestamp - last) * 1000);

        benchmarked_blocks.push(BenchmarkedBlock {
            number,
            tx_count: receipts.len(),
            gas_used: block.header.gas_used,
            timestamp: block.header.timestamp,
            latency_ms,
        });

        last_block_timestamp = Some(timestamp);
    }

    let metadata = BenchmarkMetadata {
        target_tps: args.tps,
        run_duration_secs: args.duration,
        num_accounts: args.accounts,
        num_workers: args.workers,
        chain_id: args.chain_id,
        total_connections: args.total_connections,
        start_block,
        end_block,
        node_commit_sha: args.node_commit_sha.clone(),
        build_profile: args.build_profile.clone(),
        mode: args.benchmark_mode.clone(),
    };

    let report = BenchmarkReport {
        metadata,
        blocks: benchmarked_blocks,
    };

    let file = File::create("report.json")?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, &report)?;

    println!("Report written to report.json");

    Ok(())
}

fn monitor_tps(tx_counter: Arc<AtomicU64>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_count = 0u64;
        loop {
            let current_count = tx_counter.load(Ordering::Relaxed);
            let tps = current_count - last_count;
            last_count = current_count;

            println!("TPS Sent: {tps}, Total Txs Sent: {current_count}");
            thread::sleep(Duration::from_secs(1));
        }
    })
}
