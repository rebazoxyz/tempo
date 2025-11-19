//! Tests for transient storage functionality in AccountKeychain

use alloy::primitives::{Address, address};
use tempo_precompiles::{
    account_keychain::AccountKeychain, storage::hashmap::HashMapStorageProvider,
};

#[test]
fn test_transaction_key_transient_storage() {
    let mut storage = HashMapStorageProvider::new(1);

    let account1 = address!("1000000000000000000000000000000000000001");
    let account2 = address!("2000000000000000000000000000000000000002");
    let key_id1 = address!("3000000000000000000000000000000000000003");
    let key_id2 = address!("4000000000000000000000000000000000000004");

    // Create AccountKeychain instance
    let mut keychain = AccountKeychain::new(&mut storage);

    // Set transaction key for account1
    keychain.set_transaction_key(account1, key_id1).unwrap();

    // Verify it was set correctly
    let retrieved_key = keychain
        .get_transaction_key(
            tempo_contracts::precompiles::IAccountKeychain::getTransactionKeyCall {},
            account1,
        )
        .unwrap();
    assert_eq!(retrieved_key, key_id1);

    // Set a different key for account2
    keychain.set_transaction_key(account2, key_id2).unwrap();

    // Verify both keys are independent
    let key1 = keychain
        .get_transaction_key(
            tempo_contracts::precompiles::IAccountKeychain::getTransactionKeyCall {},
            account1,
        )
        .unwrap();
    let key2 = keychain
        .get_transaction_key(
            tempo_contracts::precompiles::IAccountKeychain::getTransactionKeyCall {},
            account2,
        )
        .unwrap();

    assert_eq!(key1, key_id1);
    assert_eq!(key2, key_id2);

    // Update account1's key
    let new_key_id = address!("5000000000000000000000000000000000000005");
    keychain.set_transaction_key(account1, new_key_id).unwrap();

    // Verify the update
    let updated_key = keychain
        .get_transaction_key(
            tempo_contracts::precompiles::IAccountKeychain::getTransactionKeyCall {},
            account1,
        )
        .unwrap();
    assert_eq!(updated_key, new_key_id);

    // Account2's key should remain unchanged
    let key2_again = keychain
        .get_transaction_key(
            tempo_contracts::precompiles::IAccountKeychain::getTransactionKeyCall {},
            account2,
        )
        .unwrap();
    assert_eq!(key2_again, key_id2);
}

#[test]
fn test_transaction_key_zero_address() {
    let mut storage = HashMapStorageProvider::new(1);
    let account = address!("6000000000000000000000000000000000000006");

    let mut keychain = AccountKeychain::new(&mut storage);

    // Set to zero address (main key)
    keychain
        .set_transaction_key(account, Address::ZERO)
        .unwrap();

    let key = keychain
        .get_transaction_key(
            tempo_contracts::precompiles::IAccountKeychain::getTransactionKeyCall {},
            account,
        )
        .unwrap();
    assert_eq!(key, Address::ZERO);
}

#[test]
fn test_authorization_requires_main_key() {
    let mut storage = HashMapStorageProvider::new(1);
    let account = address!("7000000000000000000000000000000000000007");
    let key_id = address!("8000000000000000000000000000000000000008");

    let mut keychain = AccountKeychain::new(&mut storage);
    keychain.initialize().unwrap();

    // Set transaction key to ZERO (main key) - should allow authorization
    keychain
        .set_transaction_key(account, Address::ZERO)
        .unwrap();

    // Try to authorize a new key - should succeed with main key
    let result = keychain.authorize_key(
        account,
        tempo_contracts::precompiles::IAccountKeychain::authorizeKeyCall {
            keyId: key_id,
            signatureType: tempo_contracts::precompiles::IAccountKeychain::SignatureType::Secp256k1,
            expiry: 0,
            limits: vec![],
        },
    );
    assert!(result.is_ok());

    // Now set transaction key to the secondary key
    keychain.set_transaction_key(account, key_id).unwrap();

    // Try to authorize another key - should fail with secondary key
    let another_key = address!("9000000000000000000000000000000000000009");
    let result = keychain.authorize_key(
        account,
        tempo_contracts::precompiles::IAccountKeychain::authorizeKeyCall {
            keyId: another_key,
            signatureType: tempo_contracts::precompiles::IAccountKeychain::SignatureType::Secp256k1,
            expiry: 0,
            limits: vec![],
        },
    );
    assert!(result.is_err());
}

#[test]
fn test_transient_storage_isolation() {
    // This test verifies that transient storage is properly isolated per address
    let mut storage = HashMapStorageProvider::new(1);

    let account = address!("a000000000000000000000000000000000000010");
    let key_id = address!("b000000000000000000000000000000000000011");

    let mut keychain1 = AccountKeychain::new(&mut storage);
    keychain1.set_transaction_key(account, key_id).unwrap();

    // Create a second keychain instance with the same storage
    // In a real scenario, this would be a different transaction
    let mut keychain2 = AccountKeychain::new(&mut storage);

    // The key should still be accessible (simulating within same transaction)
    let retrieved = keychain2
        .get_transaction_key(
            tempo_contracts::precompiles::IAccountKeychain::getTransactionKeyCall {},
            account,
        )
        .unwrap();
    assert_eq!(retrieved, key_id);

    // Note: In production, transient storage automatically clears between transactions
    // This is handled by the EVM, not our code
}
