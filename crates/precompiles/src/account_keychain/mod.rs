pub mod dispatch;

use tempo_contracts::precompiles::{AccountKeychainError, AccountKeychainEvent};
pub use tempo_contracts::precompiles::{
    IAccountKeychain,
    IAccountKeychain::{
        KeyInfo, SignatureType, TokenLimit, authorizeKeyCall, getKeyCall, getRemainingLimitCall,
        getTransactionKeyCall, revokeKeyCall, updateSpendingLimitCall,
    },
};

use crate::{ACCOUNT_KEYCHAIN_ADDRESS, error::Result, storage::PrecompileStorageProvider};
use alloy::primitives::{Address, B256, Bytes, IntoLogData, U256};
use revm::state::Bytecode;
use tempo_precompiles_macros::{Storable, contract};

/// Key information stored in the precompile
#[derive(Debug, Clone, Default, PartialEq, Eq, Storable)]
pub struct AuthorizedKey {
    pub signature_type: u8, // 0: secp256k1, 1: P256, 2: WebAuthn
    pub expiry: u64,        // Block timestamp when key expires
    pub is_active: bool,    // Whether key is active
}

/// Account Keychain contract for managing authorized keys
#[contract]
pub struct AccountKeychain {
    // keys[account][keyId] -> AuthorizedKey
    keys: Mapping<Address, Mapping<Address, AuthorizedKey>>,
    // spendingLimits[(account, keyId)][token] -> amount
    // Using a hash of account and keyId as the key to avoid triple nesting
    spending_limits: Mapping<B256, Mapping<Address, U256>>,
    // transactionKey[account] -> keyId (Address::ZERO for main key)
    // Uses transient storage that automatically clears after transaction
    transaction_key: TransientMapping<Address, Address>,
}

impl<'a, S: PrecompileStorageProvider> AccountKeychain<'a, S> {
    /// Creates an instance of the precompile.
    ///
    /// Caution: This does not initialize the account, see [`Self::initialize`].
    pub fn new(storage: &'a mut S) -> Self {
        Self::_new(ACCOUNT_KEYCHAIN_ADDRESS, storage)
    }

    /// Create a hash key for spending limits mapping from account and keyId
    fn spending_limit_key(account: Address, key_id: Address) -> B256 {
        use alloy::primitives::keccak256;
        let mut data = [0u8; 40];
        data[..20].copy_from_slice(account.as_slice());
        data[20..].copy_from_slice(key_id.as_slice());
        keccak256(data)
    }

    /// Initializes the account keychain contract.
    pub fn initialize(&mut self) -> Result<()> {
        self.storage.set_code(
            ACCOUNT_KEYCHAIN_ADDRESS,
            Bytecode::new_legacy(Bytes::from_static(&[0xef])),
        )?;

        Ok(())
    }

    /// Authorize a new key for an account
    /// This can only be called by the account itself (using main key)
    pub fn authorize_key(&mut self, msg_sender: Address, call: authorizeKeyCall) -> Result<()> {
        // Check that the transaction key for this transaction is zero (main key)
        let transaction_key = self.sload_transaction_key(msg_sender)?;

        // If transaction_key is not zero, it means a secondary key is being used
        if transaction_key != Address::ZERO {
            return Err(AccountKeychainError::unauthorized_caller().into());
        }

        // Validate inputs
        if call.keyId == Address::ZERO {
            return Err(AccountKeychainError::zero_public_key().into());
        }

        // Check if key already exists
        let existing_key = self.sload_keys(msg_sender, call.keyId)?;

        if existing_key.is_active {
            return Err(AccountKeychainError::key_already_exists().into());
        }

        // Convert SignatureType enum to u8 for storage
        let signature_type = match call.signatureType {
            SignatureType::Secp256k1 => 0,
            SignatureType::P256 => 1,
            SignatureType::WebAuthn => 2,
            _ => return Err(AccountKeychainError::invalid_signature_type().into()),
        };

        // Create and store the new key
        let new_key = AuthorizedKey {
            signature_type,
            expiry: call.expiry,
            is_active: true,
        };

        self.sstore_keys(msg_sender, call.keyId, new_key)?;

        // Set initial spending limits
        let limit_key = Self::spending_limit_key(msg_sender, call.keyId);
        for limit in call.limits {
            self.sstore_spending_limits(limit_key, limit.token, limit.amount)?;
        }

        // Emit event
        let mut public_key_bytes = [0u8; 32];
        public_key_bytes[12..].copy_from_slice(call.keyId.as_slice());
        self.storage.emit_event(
            ACCOUNT_KEYCHAIN_ADDRESS,
            AccountKeychainEvent::KeyAuthorized(IAccountKeychain::KeyAuthorized {
                account: msg_sender,
                publicKey: B256::from(public_key_bytes),
                signatureType: signature_type,
                expiry: call.expiry,
            })
            .into_log_data(),
        )?;

        Ok(())
    }

    /// Revoke an authorized key
    pub fn revoke_key(&mut self, msg_sender: Address, call: revokeKeyCall) -> Result<()> {
        let transaction_key = self.sload_transaction_key(msg_sender)?;

        if transaction_key != Address::ZERO {
            return Err(AccountKeychainError::unauthorized_caller().into());
        }

        let mut key = self.sload_keys(msg_sender, call.keyId)?;

        if !key.is_active {
            return Err(AccountKeychainError::key_inactive().into());
        }

        // Mark key as inactive
        key.is_active = false;
        self.sstore_keys(msg_sender, call.keyId, key)?;

        // Emit event
        let mut public_key_bytes = [0u8; 32];
        public_key_bytes[12..].copy_from_slice(call.keyId.as_slice());
        self.storage.emit_event(
            ACCOUNT_KEYCHAIN_ADDRESS,
            AccountKeychainEvent::KeyRevoked(IAccountKeychain::KeyRevoked {
                account: msg_sender,
                publicKey: B256::from(public_key_bytes),
            })
            .into_log_data(),
        )?;

        Ok(())
    }

    /// Update spending limit for a key-token pair
    pub fn update_spending_limit(
        &mut self,
        msg_sender: Address,
        call: updateSpendingLimitCall,
    ) -> Result<()> {
        let transaction_key = self.sload_transaction_key(msg_sender)?;

        if transaction_key != Address::ZERO {
            return Err(AccountKeychainError::unauthorized_caller().into());
        }

        // Verify key exists and is active
        let key = self.sload_keys(msg_sender, call.keyId)?;

        if !key.is_active {
            return Err(AccountKeychainError::key_inactive().into());
        }

        // Update the spending limit
        let limit_key = Self::spending_limit_key(msg_sender, call.keyId);
        self.sstore_spending_limits(limit_key, call.token, call.newLimit)?;

        // Emit event
        let mut public_key_bytes = [0u8; 32];
        public_key_bytes[12..].copy_from_slice(call.keyId.as_slice());
        self.storage.emit_event(
            ACCOUNT_KEYCHAIN_ADDRESS,
            AccountKeychainEvent::SpendingLimitUpdated(IAccountKeychain::SpendingLimitUpdated {
                account: msg_sender,
                publicKey: B256::from(public_key_bytes),
                token: call.token,
                newLimit: call.newLimit,
            })
            .into_log_data(),
        )?;

        Ok(())
    }

    /// Get key information
    pub fn get_key(&mut self, call: getKeyCall) -> Result<KeyInfo> {
        let key = self.sload_keys(call.account, call.keyId)?;

        // If the key is not active, return default (non-existent key)
        if !key.is_active {
            return Ok(KeyInfo {
                signatureType: SignatureType::Secp256k1,
                keyId: Address::ZERO,
                expiry: 0,
            });
        }

        // Convert u8 signature_type to SignatureType enum
        let signature_type = match key.signature_type {
            0 => SignatureType::Secp256k1,
            1 => SignatureType::P256,
            2 => SignatureType::WebAuthn,
            _ => SignatureType::Secp256k1, // Default fallback
        };

        Ok(KeyInfo {
            signatureType: signature_type,
            keyId: call.keyId,
            expiry: key.expiry,
        })
    }

    /// Get remaining spending limit
    pub fn get_remaining_limit(&mut self, call: getRemainingLimitCall) -> Result<U256> {
        let limit_key = Self::spending_limit_key(call.account, call.keyId);
        self.sload_spending_limits(limit_key, call.token)
    }

    /// Get the transaction key used in the current transaction
    pub fn get_transaction_key(
        &mut self,
        _call: getTransactionKeyCall,
        msg_sender: Address,
    ) -> Result<Address> {
        self.sload_transaction_key(msg_sender)
    }

    /// Internal: Set the transaction key (called during transaction validation)
    ///
    /// SECURITY CRITICAL: This must be called by the transaction validation logic
    /// BEFORE the transaction is executed, to store which key authorized the transaction.
    /// - If key_id is Address::ZERO (main key), this should store Address::ZERO
    /// - If key_id is a specific key address, this should store that key
    ///
    /// This creates a secure channel between validation and the precompile to ensure
    /// only the main key can authorize/revoke other keys.
    pub fn set_transaction_key(&mut self, account: Address, key_id: Address) -> Result<()> {
        self.sstore_transaction_key(account, key_id)?;
        Ok(())
    }

    /// Validate keychain authorization (existence, active status, expiry)
    ///
    /// This consolidates all validation checks into one method.
    /// Returns Ok(()) if the key is valid and authorized, Err otherwise.
    pub fn validate_keychain_authorization(
        &mut self,
        account: Address,
        key_id: Address,
        current_timestamp: u64,
    ) -> Result<()> {
        // If using main key (zero address), always valid
        if key_id == Address::ZERO {
            return Ok(());
        }

        let key = self.sload_keys(account, key_id)?;

        if !key.is_active {
            return Err(AccountKeychainError::key_inactive().into());
        }

        if key.expiry > 0 && current_timestamp >= key.expiry {
            return Err(AccountKeychainError::key_expired().into());
        }

        Ok(())
    }

    /// Internal: Verify and update spending for a token transfer
    pub fn verify_and_update_spending(
        &mut self,
        account: Address,
        key_id: Address,
        token: Address,
        amount: U256,
    ) -> Result<()> {
        // If using main key (zero address), no spending limits apply
        if key_id == Address::ZERO {
            return Ok(());
        }

        // Check key is valid
        let key = self.sload_keys(account, key_id)?;

        if !key.is_active {
            return Err(AccountKeychainError::key_inactive().into());
        }

        // Check and update spending limit
        let limit_key = Self::spending_limit_key(account, key_id);
        let remaining = self.sload_spending_limits(limit_key, token)?;

        if amount > remaining {
            return Err(AccountKeychainError::spending_limit_exceeded().into());
        }

        // Update remaining limit
        self.sstore_spending_limits(limit_key, token, remaining - amount)?;

        Ok(())
    }
}
