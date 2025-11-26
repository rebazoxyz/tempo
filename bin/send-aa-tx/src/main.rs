use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, TxKind, U256},
    providers::{Provider, ProviderBuilder},
    signers::{local::PrivateKeySigner, SignerSync},
};
use alloy_eips::eip2718::Encodable2718;
use tempo_primitives::{
    transaction::{
        aa_signature::{AASignature, PrimitiveSignature},
        aa_signed::AASigned,
        account_abstraction::Call,
        TxAA,
    },
    TempoTxEnvelope,
};

const RPC_URL: &str = "http://<RPC_URL>";
const CHAIN_ID: u64 = 42427; // 0xa5bb - devnet chain ID
const BASE_FEE: u128 = 10_000_000_000; // 10 gwei

#[tokio::main]
async fn main() -> eyre::Result<()> {
    println!("🚀 Sending AA Transaction to Devnet\n");
    println!("Chain ID: {}\n", CHAIN_ID);

    // Generate a random private key for this test
    let signer = PrivateKeySigner::random();
    let sender_addr = signer.address();

    println!("Sender address: {}", sender_addr);

    // Create provider
    let wallet = EthereumWallet::from(signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(RPC_URL.parse()?);

    // Fund the sender address using the faucet
    println!("\n💰 Requesting funds from faucet...");

    // The faucet returns an array of transaction hashes (for AlphaUSD, BetaUSD, ThetaUSD)
    let faucet_txs: Vec<String> = provider
        .raw_request("tempo_fundAddress".into(), [sender_addr.to_string()])
        .await?;

    println!("✓ Faucet request successful");
    println!("  Faucet transactions:");
    for (i, tx) in faucet_txs.iter().enumerate() {
        println!("    {}: {}", i + 1, tx);
    }

    // Wait for the faucet transaction to be mined
    println!("Waiting for faucet transaction to be mined...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    // Get nonce
    let nonce = provider.get_transaction_count(sender_addr).await?;
    println!("\n📊 Current nonce: {}", nonce);

    // Create a simple AA transaction - just a transfer to a random address
    let recipient = Address::random();
    println!("Recipient: {}", recipient);

    let tx = TxAA {
        chain_id: CHAIN_ID,
        max_priority_fee_per_gas: BASE_FEE,
        max_fee_per_gas: BASE_FEE,
        gas_limit: 100_000,
        calls: vec![Call {
            to: TxKind::Call(recipient),
            value: U256::ZERO,
            input: Bytes::new(),
        }],
        nonce_key: U256::ZERO, // Protocol nonce (key 0)
        nonce,
        fee_token: None, // Will use default fee token
        fee_payer_signature: None,
        valid_before: Some(u64::MAX),
        valid_after: None,
        access_list: Default::default(),
        key_authorization: None,       // No key provisioning
        aa_authorization_list: vec![], // No EIP-7702 delegations
    };

    println!("\n✍️  Signing transaction...");

    // Sign the transaction with secp256k1
    let sig_hash = tx.signature_hash();
    let signature = signer.sign_hash_sync(&sig_hash)?;
    let aa_signature = AASignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tx, aa_signature);

    // Convert to envelope and encode
    let envelope: TempoTxEnvelope = signed_tx.into();
    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    println!("✓ Transaction signed");
    println!("  Transaction type: 0x{:02x} (AA)", encoded[0]);
    println!("  Encoded size: {} bytes", encoded.len());

    // Print the encoded transaction for debugging
    println!("  Encoded (hex): 0x{}", hex::encode(&encoded));

    // Send the transaction
    println!("\n📤 Sending transaction...");
    let pending_tx = provider.send_raw_transaction(&encoded).await?;
    let tx_hash = *pending_tx.tx_hash();

    println!("✓ Transaction sent!");
    println!("  Transaction hash: {}", tx_hash);

    // Wait for the transaction to be mined
    // Note: Standard alloy provider can't deserialize AA tx type (0x76) in receipts,
    // so we use a raw RPC call and parse the response manually
    println!("\n⏳ Waiting for confirmation...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let receipt: serde_json::Value = provider
        .raw_request(
            "eth_getTransactionReceipt".into(),
            [format!("{:#x}", tx_hash)],
        )
        .await?;

    if receipt.is_null() {
        println!("⚠️  Transaction not yet confirmed (check later)");
    } else {
        let status = receipt["status"].as_str().unwrap_or("0x0");
        let block_number = receipt["blockNumber"].as_str().unwrap_or("0x0");
        let gas_used = receipt["gasUsed"].as_str().unwrap_or("0x0");

        let block_num = u64::from_str_radix(block_number.trim_start_matches("0x"), 16).unwrap_or(0);
        let gas = u64::from_str_radix(gas_used.trim_start_matches("0x"), 16).unwrap_or(0);

        println!("✓ Transaction confirmed!");
        println!("  Block number: {}", block_num);
        println!("  Gas used: {}", gas);
        println!(
            "  Status: {}",
            if status == "0x1" { "Success" } else { "Failed" }
        );
    }

    println!("\nDone!");

    Ok(())
}
