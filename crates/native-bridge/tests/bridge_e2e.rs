//! End-to-end tests for the bridge sidecar.
//!
//! This test:
//! 1. Starts an Anvil instance (Ethereum with Prague hardfork for EIP-2537)
//! 2. Starts a Tempo node (in-process via TestNodeBuilder)
//! 3. Deploys the REAL MessageBridge contract to both
//! 4. Sends messages and verifies event subscription works

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, Bytes};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::signers::local::MnemonicBuilder;
use alloy::sol;
use alloy::sol_types::SolEvent;
use alloy_primitives::B256;
use futures::StreamExt;
use reth_ethereum::tasks::TaskManager;
use reth_node_builder::{NodeBuilder, NodeConfig};
use reth_node_core::args::RpcServerArgs;
use reth_rpc_builder::RpcModuleSelection;
use tempo_chainspec::spec::TempoChainSpec;
use tempo_node::node::TempoNode;
use tokio::time::timeout;

/// Standard test mnemonic (has balance in genesis).
const TEST_MNEMONIC: &str = "test test test test test test test test test test test junk";

// MessageBridge contract interface
sol! {
    #[derive(Debug)]
    event MessageSent(
        address indexed sender,
        bytes32 indexed messageHash,
        uint64 indexed destinationChainId
    );

    #[derive(Debug)]
    function send(bytes32 messageHash, uint64 destinationChainId) external;
}

/// Encode MessageBridge constructor arguments.
/// constructor(address _owner, uint64 _initialEpoch, bytes memory _initialPublicKey)
fn encode_message_bridge_constructor(owner: Address, epoch: u64, public_key: &[u8]) -> Vec<u8> {
    // ABI encode: (address, uint64, bytes)
    // address is padded to 32 bytes
    // uint64 is padded to 32 bytes  
    // bytes is encoded as: offset (32) + length (32) + data (padded to 32)
    let mut encoded = Vec::new();

    // owner (address, 32 bytes)
    encoded.extend_from_slice(&[0u8; 12]); // 12 bytes padding
    encoded.extend_from_slice(owner.as_slice());

    // epoch (uint64, 32 bytes)
    encoded.extend_from_slice(&[0u8; 24]); // 24 bytes padding
    encoded.extend_from_slice(&epoch.to_be_bytes());

    // bytes offset (points to byte 96 = 0x60)
    encoded.extend_from_slice(&[0u8; 31]);
    encoded.push(0x60);

    // bytes length
    let len = public_key.len();
    encoded.extend_from_slice(&[0u8; 24]);
    encoded.extend_from_slice(&(len as u64).to_be_bytes());

    // bytes data (padded to 32 bytes)
    encoded.extend_from_slice(public_key);
    let padding = (32 - (len % 32)) % 32;
    encoded.extend_from_slice(&vec![0u8; padding]);

    encoded
}

/// Anvil instance wrapper with automatic cleanup.
struct AnvilInstance {
    child: Child,
    rpc_url: String,
    ws_url: String,
}

impl AnvilInstance {
    async fn start() -> eyre::Result<Self> {
        let port = portpicker::pick_unused_port().expect("no free port");

        let child = Command::new("anvil")
            .args([
                "--port",
                &port.to_string(),
                "--chain-id",
                "1",
                "--block-time",
                "1",
                "--hardfork",
                "prague", // Required for EIP-2537 BLS precompiles
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let rpc_url = format!("http://127.0.0.1:{port}");
        let ws_url = format!("ws://127.0.0.1:{port}");

        // Wait for anvil to be ready
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Verify it's running
        let provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);
        let block = provider.get_block_number().await?;
        tracing::info!(port, block, "anvil started");

        Ok(Self {
            child,
            rpc_url,
            ws_url,
        })
    }
}

impl Drop for AnvilInstance {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Real MessageBridge bytecode.
/// Uses EIP-2537 BLS12-381 precompiles (available on Prague+ hardfork and Tempo).
/// From: crates/native-bridge/contracts/out/MessageBridge.sol/MessageBridge.json
const MESSAGE_BRIDGE_BYTECODE: &str = include_str!("../contracts/out/MessageBridge.sol/MessageBridge.bytecode.hex");

/// G1 generator point (uncompressed, 128 bytes) for test deployment.
/// This is a valid BLS12-381 G1 point that can be used as the initial public key.
/// Format: 64 bytes x-coordinate + 64 bytes y-coordinate (each with 16 bytes zero padding)
const G1_GENERATOR: &str = "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1";

/// Deploy the real MessageBridge contract (for Anvil with Prague hardfork).
async fn deploy_message_bridge_anvil(rpc_url: &str) -> eyre::Result<Address> {
    use alloy::network::TransactionBuilder;
    use alloy::providers::ProviderBuilder;
    use alloy::signers::local::PrivateKeySigner;

    // Anvil's default funded account
    let signer: PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
            .parse()
            .unwrap();
    let owner = signer.address();

    let provider = ProviderBuilder::new()
        .wallet(alloy::network::EthereumWallet::from(signer))
        .connect_http(rpc_url.parse()?);

    // Contract bytecode
    let bytecode = hex::decode(MESSAGE_BRIDGE_BYTECODE.trim())?;

    // Constructor arguments: owner, initial epoch, initial public key (G1 generator)
    let initial_epoch = 1u64;
    let initial_public_key = hex::decode(G1_GENERATOR)?;

    // Encode constructor args and append to bytecode
    let constructor_args = encode_message_bridge_constructor(owner, initial_epoch, &initial_public_key);
    let deploy_code: Vec<u8> = bytecode.into_iter().chain(constructor_args).collect();

    let tx = alloy::rpc::types::TransactionRequest::default()
        .with_deploy_code(Bytes::from(deploy_code));

    let pending = provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;

    let address = receipt
        .contract_address
        .ok_or_else(|| eyre::eyre!("no contract address in receipt"))?;

    tracing::info!(%address, %owner, epoch = initial_epoch, "deployed MessageBridge on Anvil");
    Ok(address)
}

/// Deploy the real MessageBridge contract on Tempo.
async fn deploy_message_bridge_tempo(rpc_url: &str) -> eyre::Result<Address> {
    use alloy::network::TransactionBuilder;
    use alloy::providers::ProviderBuilder;

    // Use the funded mnemonic wallet
    let wallet = MnemonicBuilder::from_phrase(TEST_MNEMONIC).build()?;
    let owner = wallet.address();

    let provider = ProviderBuilder::new()
        .wallet(alloy::network::EthereumWallet::from(wallet))
        .connect_http(rpc_url.parse()?);

    // Contract bytecode
    let bytecode = hex::decode(MESSAGE_BRIDGE_BYTECODE.trim())?;

    // Constructor arguments: owner, initial epoch, initial public key (G1 generator)
    let initial_epoch = 1u64;
    let initial_public_key = hex::decode(G1_GENERATOR)?;

    // Encode constructor args and append to bytecode
    let constructor_args = encode_message_bridge_constructor(owner, initial_epoch, &initial_public_key);
    let deploy_code: Vec<u8> = bytecode.into_iter().chain(constructor_args).collect();

    let tx = alloy::rpc::types::TransactionRequest::default()
        .with_deploy_code(Bytes::from(deploy_code));

    let pending = provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;

    let address = receipt
        .contract_address
        .ok_or_else(|| eyre::eyre!("no contract address in receipt"))?;

    tracing::info!(%address, %owner, epoch = initial_epoch, "deployed MessageBridge on Tempo");
    Ok(address)
}

#[tokio::test]
async fn test_anvil_event_subscription() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("bridge_e2e=debug,tempo_native_bridge=debug")
        .try_init()
        .ok();

    // Start Anvil
    let anvil = AnvilInstance::start().await?;
    tracing::info!(rpc = %anvil.rpc_url, ws = %anvil.ws_url, "anvil running");

    // Deploy mock bridge
    let bridge_address = deploy_message_bridge_anvil(&anvil.rpc_url).await?;

    // Connect via WebSocket and subscribe to events
    let ws_provider = ProviderBuilder::new().connect(&anvil.ws_url).await?;

    let filter = Filter::new()
        .address(bridge_address)
        .event_signature(MessageSent::SIGNATURE_HASH);

    let sub = ws_provider.subscribe_logs(&filter).await?;
    let mut stream = sub.into_stream();

    tracing::info!("subscribed to MessageSent events");

    // Send a message using HTTP provider
    let signer: alloy::signers::local::PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
            .parse()
            .unwrap();

    let http_provider = ProviderBuilder::new()
        .wallet(alloy::network::EthereumWallet::from(signer))
        .connect_http(anvil.rpc_url.parse()?);

    // Send a transaction that emits MessageSent
    let message_hash = B256::repeat_byte(0x42);
    let dest_chain_id = 12345u64;

    let call = sendCall {
        messageHash: message_hash,
        destinationChainId: dest_chain_id,
    };

    let tx = alloy::rpc::types::TransactionRequest::default()
        .to(bridge_address)
        .input(alloy::sol_types::SolCall::abi_encode(&call).into());

    let pending = http_provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;
    tracing::info!(tx_hash = %receipt.transaction_hash, "sent message");

    // Wait for the event
    let event = timeout(Duration::from_secs(10), stream.next())
        .await?
        .ok_or_else(|| eyre::eyre!("no event received"))?;

    tracing::info!(?event, "received event");

    // Verify event data
    let topics = event.topics();
    assert!(topics.len() >= 4, "expected 4 topics");

    let received_sender = Address::from_slice(&topics[1].as_slice()[12..]);
    let received_hash = B256::from(topics[2]);
    let received_dest = u64::from_be_bytes(topics[3].as_slice()[24..].try_into()?);

    tracing::info!(
        sender = %received_sender,
        hash = %received_hash,
        dest = received_dest,
        "parsed event"
    );

    assert_eq!(received_hash, message_hash);
    assert_eq!(received_dest, dest_chain_id);

    tracing::info!("test passed!");
    Ok(())
}

#[tokio::test]
async fn test_anvil_polling_fallback() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("bridge_e2e=debug")
        .try_init()
        .ok();

    // Start Anvil
    let anvil = AnvilInstance::start().await?;

    // Deploy mock bridge
    let bridge_address = deploy_message_bridge_anvil(&anvil.rpc_url).await?;

    // Use HTTP provider (polling mode)
    let provider = ProviderBuilder::new().connect_http(anvil.rpc_url.parse()?);

    let start_block = provider.get_block_number().await?;

    // Send a message
    let signer: alloy::signers::local::PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
            .parse()
            .unwrap();

    let tx_provider = ProviderBuilder::new()
        .wallet(alloy::network::EthereumWallet::from(signer))
        .connect_http(anvil.rpc_url.parse()?);

    let message_hash = B256::repeat_byte(0x11);

    let call = sendCall {
        messageHash: message_hash,
        destinationChainId: 1u64,
    };

    let tx = alloy::rpc::types::TransactionRequest::default()
        .to(bridge_address)
        .input(alloy::sol_types::SolCall::abi_encode(&call).into());

    let pending = tx_provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;
    tracing::info!(tx_hash = %receipt.transaction_hash, "sent message");

    // Wait a bit for block to be mined
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Poll for logs
    let end_block = provider.get_block_number().await?;

    let filter = Filter::new()
        .address(bridge_address)
        .event_signature(MessageSent::SIGNATURE_HASH)
        .from_block(start_block)
        .to_block(end_block);

    let logs = provider.get_logs(&filter).await?;
    tracing::info!(count = logs.len(), "fetched logs");

    assert!(!logs.is_empty(), "expected at least one log");

    let log = &logs[0];
    let received_hash = B256::from(log.topics()[2]);
    assert_eq!(received_hash, message_hash);

    tracing::info!("polling test passed!");
    Ok(())
}

/// Test genesis JSON for Tempo node tests.
const TEST_GENESIS: &str = include_str!("../../node/tests/assets/test-genesis.json");

/// Start an in-process Tempo node for testing.
async fn start_tempo_node() -> eyre::Result<(String, String, TaskManager)> {
    let genesis: serde_json::Value = serde_json::from_str(TEST_GENESIS)?;
    let chain_spec = TempoChainSpec::from_genesis(serde_json::from_value(genesis)?);
    let validator = chain_spec.inner.genesis.coinbase;

    let tasks = TaskManager::current();

    let mut node_config = NodeConfig::new(Arc::new(chain_spec))
        .with_unused_ports()
        .dev()
        .with_rpc(
            RpcServerArgs::default()
                .with_unused_ports()
                .with_http()
                .with_ws()
                .with_http_api(RpcModuleSelection::All)
                .with_ws_api(RpcModuleSelection::All),
        );
    node_config.dev.block_time = Some(Duration::from_millis(500));

    let node_handle = NodeBuilder::new(node_config)
        .testing_node(tasks.executor())
        .node(TempoNode::default())
        .launch_with_debug_capabilities()
        .map_debug_payload_attributes(move |mut attributes| {
            attributes.suggested_fee_recipient = validator;
            attributes
        })
        .await?;

    let http_url = node_handle
        .node
        .rpc_server_handle()
        .http_url()
        .ok_or_else(|| eyre::eyre!("no HTTP URL"))?;

    let ws_url = node_handle
        .node
        .rpc_server_handle()
        .ws_url()
        .ok_or_else(|| eyre::eyre!("no WS URL"))?;

    tracing::info!(%http_url, %ws_url, "tempo node started");

    // Keep the node handle alive by leaking it (task manager keeps it running)
    std::mem::forget(node_handle);

    Ok((http_url, ws_url, tasks))
}

#[tokio::test]
async fn test_tempo_event_subscription() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("bridge_e2e=debug,tempo_native_bridge=debug,tempo=info")
        .try_init()
        .ok();

    // Start Tempo node
    let (http_url, ws_url, _tasks) = start_tempo_node().await?;
    tracing::info!(http = %http_url, ws = %ws_url, "tempo node running");

    // Deploy mock bridge contract (use Tempo deployer with funded wallet)
    let bridge_address = deploy_message_bridge_tempo(&http_url).await?;

    // Connect via WebSocket and subscribe to events
    let ws_provider = ProviderBuilder::new().connect(&ws_url).await?;

    let filter = Filter::new()
        .address(bridge_address)
        .event_signature(MessageSent::SIGNATURE_HASH);

    let sub = ws_provider.subscribe_logs(&filter).await?;
    let mut stream = sub.into_stream();

    tracing::info!("subscribed to MessageSent events on Tempo");

    // Send a message using HTTP provider with funded wallet
    let wallet = MnemonicBuilder::from_phrase(TEST_MNEMONIC).build()?;

    let http_provider = ProviderBuilder::new()
        .wallet(alloy::network::EthereumWallet::from(wallet))
        .connect_http(http_url.parse()?);

    // Send a transaction that emits MessageSent
    let message_hash = B256::repeat_byte(0x99);
    let dest_chain_id = 1u64; // Ethereum mainnet

    let call = sendCall {
        messageHash: message_hash,
        destinationChainId: dest_chain_id,
    };

    let tx = alloy::rpc::types::TransactionRequest::default()
        .to(bridge_address)
        .input(alloy::sol_types::SolCall::abi_encode(&call).into());

    let pending = http_provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;
    tracing::info!(tx_hash = %receipt.transaction_hash, "sent message on Tempo");

    // Wait for the event
    let event = timeout(Duration::from_secs(10), stream.next())
        .await?
        .ok_or_else(|| eyre::eyre!("no event received from Tempo"))?;

    tracing::info!(?event, "received event from Tempo");

    // Verify event data
    let topics = event.topics();
    assert!(topics.len() >= 4, "expected 4 topics");

    let received_hash = B256::from(topics[2]);
    let received_dest = u64::from_be_bytes(topics[3].as_slice()[24..].try_into()?);

    tracing::info!(
        hash = %received_hash,
        dest = received_dest,
        "parsed Tempo event"
    );

    assert_eq!(received_hash, message_hash);
    assert_eq!(received_dest, dest_chain_id);

    tracing::info!("Tempo event subscription test passed!");
    Ok(())
}

#[tokio::test]
async fn test_tempo_polling_fallback() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("bridge_e2e=debug")
        .try_init()
        .ok();

    // Start Tempo node
    let (http_url, _ws_url, _tasks) = start_tempo_node().await?;

    // Deploy mock bridge (use Tempo deployer with funded wallet)
    let bridge_address = deploy_message_bridge_tempo(&http_url).await?;

    // Use HTTP provider (polling mode)
    let provider = ProviderBuilder::new().connect_http(http_url.parse()?);

    let start_block = provider.get_block_number().await?;

    // Send a message with funded wallet
    let wallet = MnemonicBuilder::from_phrase(TEST_MNEMONIC).build()?;

    let tx_provider = ProviderBuilder::new()
        .wallet(alloy::network::EthereumWallet::from(wallet))
        .connect_http(http_url.parse()?);

    let message_hash = B256::repeat_byte(0x77);

    let call = sendCall {
        messageHash: message_hash,
        destinationChainId: 1u64,
    };

    let tx = alloy::rpc::types::TransactionRequest::default()
        .to(bridge_address)
        .input(alloy::sol_types::SolCall::abi_encode(&call).into());

    let pending = tx_provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;
    tracing::info!(tx_hash = %receipt.transaction_hash, "sent message on Tempo");

    // Wait for block to be mined
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Poll for logs
    let end_block = provider.get_block_number().await?;

    let filter = Filter::new()
        .address(bridge_address)
        .event_signature(MessageSent::SIGNATURE_HASH)
        .from_block(start_block)
        .to_block(end_block);

    let logs = provider.get_logs(&filter).await?;
    tracing::info!(count = logs.len(), "fetched logs from Tempo");

    assert!(!logs.is_empty(), "expected at least one log");

    let log = &logs[0];
    let received_hash = B256::from(log.topics()[2]);
    assert_eq!(received_hash, message_hash);

    tracing::info!("Tempo polling test passed!");
    Ok(())
}
