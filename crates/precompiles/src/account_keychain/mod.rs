pub mod dispatch;

use tempo_contracts::precompiles::{AccountKeychainError, AccountKeychainEvent};
pub use tempo_contracts::precompiles::{
    IAccountKeychain,
    IAccountKeychain::{
        CurrencyLimit, KeyInfo, SignatureType, TokenLimit, authorizeKeyCall, getKeyCall,
        getRemainingCurrencyLimitCall, getRemainingLimitCall, getTransactionKeyCall, revokeKeyCall,
        updateCurrencyLimitCall, updateSpendingLimitCall,
    },
};

use crate::{
    ACCOUNT_KEYCHAIN_ADDRESS,
    error::Result,
    storage::{Handler, Mapping},
    tip20::TIP20Token,
};
use alloy::primitives::{Address, B256, U256};
use tempo_precompiles_macros::{Storable, contract};

/// Key information stored in the precompile
///
/// Storage layout (packed into single slot, right-aligned):
/// - byte 0: signature_type (u8)
/// - bytes 1-8: expiry (u64, little-endian)
/// - byte 9: enforce_limits (bool) - master switch
/// - byte 10: enforce_token_limits (bool) - whether token limits exist
/// - byte 11: enforce_currency_limits (bool) - whether currency limits exist
/// - byte 12: is_revoked (bool)
#[derive(Debug, Clone, Default, PartialEq, Eq, Storable)]
pub struct AuthorizedKey {
    /// Signature type: 0 = secp256k1, 1 = P256, 2 = WebAuthn
    pub signature_type: u8,
    /// Block timestamp when key expires
    pub expiry: u64,
    /// Master switch: whether to enforce spending limits for this key.
    /// If false, this key has unlimited spending (passthrough).
    pub enforce_limits: bool,
    /// Whether per-token spending limits exist for this key.
    /// Only checked if enforce_limits is true.
    pub enforce_token_limits: bool,
    /// Whether per-currency spending limits exist for this key.
    /// Only checked if enforce_limits is true.
    pub enforce_currency_limits: bool,
    /// Whether this key has been revoked. Once revoked, a key cannot be re-authorized
    /// with the same key_id. This prevents replay attacks.
    pub is_revoked: bool,
}

// TODO(rusowsky): remove this and create a read-only wrapper that is callable from read-only ctx with db access
impl AuthorizedKey {
    /// Decode AuthorizedKey from a storage slot value
    ///
    /// This is useful for read-only contexts (like pool validation) that don't have
    /// access to PrecompileStorageProvider but need to decode the packed struct.
    pub fn decode_from_slot(slot_value: U256) -> Self {
        use crate::storage::{LayoutCtx, Storable, packing::PackedSlot};

        // NOTE: fine to expect, as `StorageOps` on `PackedSlot` are infallible
        Self::load(&PackedSlot(slot_value), U256::ZERO, LayoutCtx::FULL)
            .expect("unable to decode AuthorizedKey from slot")
    }
}

/// Account Keychain contract for managing authorized keys
#[contract(addr = ACCOUNT_KEYCHAIN_ADDRESS)]
pub struct AccountKeychain {
    // keys[account][keyId] -> AuthorizedKey
    keys: Mapping<Address, Mapping<Address, AuthorizedKey>>,
    // spendingLimits[(account, keyId)][token] -> amount
    // Using a hash of account and keyId as the key to avoid triple nesting
    spending_limits: Mapping<B256, Mapping<Address, U256>>,
    // currencyLimits[(account, keyId)][currency_hash] -> amount
    // Using a hash of account and keyId as the outer key to avoid triple nesting
    // Using a hash of the currency string as the inner key (keccak256(currency))
    // Stores per-currency spending limits that apply across all tokens with that currency
    currency_limits: Mapping<B256, Mapping<B256, U256>>,

    // WARNING(rusowsky): transient storage slots must always be placed at the very end until the `contract`
    // macro is refactored and has 2 independent layouts (persistent and transient).
    // If new (persistent) storage fields need to be added to the precompile, they must go above this one.
    transaction_key: Address,
    // The transaction origin (tx.origin) - the EOA that signed the transaction.
    // Used to ensure spending limits only apply when msg_sender == tx_origin.
    tx_origin: Address,
}

impl AccountKeychain {
    /// Create a hash key for spending limits mapping from account and keyId
    fn spending_limit_key(account: Address, key_id: Address) -> B256 {
        use alloy::primitives::keccak256;
        let mut data = [0u8; 40];
        data[..20].copy_from_slice(account.as_slice());
        data[20..].copy_from_slice(key_id.as_slice());
        keccak256(data)
    }

    /// Create a hash key for currency from currency string
    fn currency_key(currency: &str) -> B256 {
        use alloy::primitives::keccak256;
        keccak256(currency.as_bytes())
    }

    /// Initializes the account keychain contract.
    pub fn initialize(&mut self) -> Result<()> {
        self.__initialize()
    }

    /// Authorize a new key for an account
    /// This can only be called by the account itself (using main key)
    pub fn authorize_key(&mut self, msg_sender: Address, call: authorizeKeyCall) -> Result<()> {
        // Check that the transaction key for this transaction is zero (main key)
        let transaction_key = self.transaction_key.t_read()?;

        // If transaction_key is not zero, it means a secondary key is being used
        if transaction_key != Address::ZERO {
            return Err(AccountKeychainError::unauthorized_caller().into());
        }

        // Validate inputs
        if call.keyId == Address::ZERO {
            return Err(AccountKeychainError::zero_public_key().into());
        }

        // Check if key already exists (key exists if expiry > 0)
        let existing_key = self.keys[msg_sender][call.keyId].read()?;
        if existing_key.expiry > 0 {
            return Err(AccountKeychainError::key_already_exists().into());
        }

        // Check if this key was previously revoked - prevents replay attacks
        if existing_key.is_revoked {
            return Err(AccountKeychainError::key_already_revoked().into());
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
            enforce_limits: call.enforceLimits,
            enforce_token_limits: !call.tokenLimits.is_empty(),
            enforce_currency_limits: !call.currencyLimits.is_empty(),
            is_revoked: false,
        };

        self.keys[msg_sender][call.keyId].write(new_key)?;

        // Set initial spending limits
        let limit_key = Self::spending_limit_key(msg_sender, call.keyId);

        // Set token limits (only if enforceLimits is true and tokenLimits is not empty)
        if call.enforceLimits {
            for limit in call.tokenLimits {
                self.spending_limits[limit_key][limit.token].write(limit.amount)?;
            }

            // Set currency limits
            for limit in call.currencyLimits {
                let currency_key = Self::currency_key(&limit.currency);
                self.currency_limits[limit_key][currency_key].write(limit.amount)?;
            }
        }

        // Emit event
        self.emit_event(AccountKeychainEvent::KeyAuthorized(
            IAccountKeychain::KeyAuthorized {
                account: msg_sender,
                publicKey: call.keyId,
                signatureType: signature_type,
                expiry: call.expiry,
            },
        ))
    }

    /// Revoke an authorized key
    ///
    /// This marks the key as revoked by setting is_revoked to true and expiry to 0.
    /// Once revoked, a key_id can never be re-authorized for this account, preventing
    /// replay attacks where old KeyAuthorization signatures could be reused.
    pub fn revoke_key(&mut self, msg_sender: Address, call: revokeKeyCall) -> Result<()> {
        let transaction_key = self.transaction_key.t_read()?;

        if transaction_key != Address::ZERO {
            return Err(AccountKeychainError::unauthorized_caller().into());
        }

        let key = self.keys[msg_sender][call.keyId].read()?;

        // Key exists if expiry > 0
        if key.expiry == 0 {
            return Err(AccountKeychainError::key_not_found().into());
        }

        // Mark the key as revoked - this prevents replay attacks by ensuring
        // the same key_id can never be re-authorized for this account.
        // We keep is_revoked=true but clear other fields.
        let revoked_key = AuthorizedKey {
            signature_type: 0,
            expiry: 0,
            enforce_limits: false,
            enforce_token_limits: false,
            enforce_currency_limits: false,
            is_revoked: true,
        };
        self.keys[msg_sender][call.keyId].write(revoked_key)?;

        // Note: We don't clear spending limits here - they become inaccessible

        // Emit event
        self.emit_event(AccountKeychainEvent::KeyRevoked(
            IAccountKeychain::KeyRevoked {
                account: msg_sender,
                publicKey: call.keyId,
            },
        ))
    }

    /// Update spending limit for a key-token pair
    ///
    /// This can be used to add limits to an unlimited key (converting it to limited)
    /// or to update existing limits.
    pub fn update_spending_limit(
        &mut self,
        msg_sender: Address,
        call: updateSpendingLimitCall,
    ) -> Result<()> {
        let transaction_key = self.transaction_key.t_read()?;

        if transaction_key != Address::ZERO {
            return Err(AccountKeychainError::unauthorized_caller().into());
        }

        // Verify key exists, hasn't been revoked, and hasn't expired
        let mut key = self.load_active_key(msg_sender, call.keyId)?;

        let current_timestamp = self.storage.timestamp().saturating_to::<u64>();
        if current_timestamp >= key.expiry {
            return Err(AccountKeychainError::key_expired().into());
        }

        // If this key had no limits enforced, enable limits now
        if !key.enforce_limits {
            key.enforce_limits = true;
            key.enforce_token_limits = true;
            self.keys[msg_sender][call.keyId].write(key)?;
        } else if !key.enforce_token_limits {
            // If limits were enforced but no token limits existed, add them
            key.enforce_token_limits = true;
            self.keys[msg_sender][call.keyId].write(key)?;
        }

        // Update the spending limit
        let limit_key = Self::spending_limit_key(msg_sender, call.keyId);
        self.spending_limits[limit_key][call.token].write(call.newLimit)?;

        // Emit event
        self.emit_event(AccountKeychainEvent::SpendingLimitUpdated(
            IAccountKeychain::SpendingLimitUpdated {
                account: msg_sender,
                publicKey: call.keyId,
                token: call.token,
                newLimit: call.newLimit,
            },
        ))
    }

    /// Update currency spending limit for a key
    ///
    /// This can be used to add currency limits to an unlimited key (converting it to limited)
    /// or to update existing currency limits.
    pub fn update_currency_limit(
        &mut self,
        msg_sender: Address,
        call: updateCurrencyLimitCall,
    ) -> Result<()> {
        let transaction_key = self.transaction_key.t_read()?;

        if transaction_key != Address::ZERO {
            return Err(AccountKeychainError::unauthorized_caller().into());
        }

        // Verify key exists, hasn't been revoked, and hasn't expired
        let mut key = self.load_active_key(msg_sender, call.keyId)?;

        let current_timestamp = self.storage.timestamp().saturating_to::<u64>();
        if current_timestamp >= key.expiry {
            return Err(AccountKeychainError::key_expired().into());
        }

        // If this key had no limits enforced, enable limits now
        if !key.enforce_limits {
            key.enforce_limits = true;
            key.enforce_currency_limits = true;
            self.keys[msg_sender][call.keyId].write(key)?;
        } else if !key.enforce_currency_limits {
            // If limits were enforced but no currency limits existed, add them
            key.enforce_currency_limits = true;
            self.keys[msg_sender][call.keyId].write(key)?;
        }

        // Update the currency limit
        let limit_key = Self::spending_limit_key(msg_sender, call.keyId);
        let currency_key = Self::currency_key(&call.currency);
        self.currency_limits[limit_key][currency_key].write(call.newLimit)?;

        // Emit event
        self.emit_event(AccountKeychainEvent::CurrencyLimitUpdated(
            IAccountKeychain::CurrencyLimitUpdated {
                account: msg_sender,
                publicKey: call.keyId,
                currency: call.currency,
                newLimit: call.newLimit,
            },
        ))
    }

    /// Get key information
    pub fn get_key(&self, call: getKeyCall) -> Result<KeyInfo> {
        let key = self.keys[call.account][call.keyId].read()?;

        // Key doesn't exist if expiry == 0, or key has been revoked
        if key.expiry == 0 || key.is_revoked {
            return Ok(KeyInfo {
                signatureType: SignatureType::Secp256k1,
                keyId: Address::ZERO,
                expiry: 0,
                enforceLimits: false,
                hasTokenLimits: false,
                hasCurrencyLimits: false,
                isRevoked: key.is_revoked,
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
            enforceLimits: key.enforce_limits,
            hasTokenLimits: key.enforce_token_limits,
            hasCurrencyLimits: key.enforce_currency_limits,
            isRevoked: key.is_revoked,
        })
    }

    /// Get remaining spending limit
    pub fn get_remaining_limit(&self, call: getRemainingLimitCall) -> Result<U256> {
        let limit_key = Self::spending_limit_key(call.account, call.keyId);
        self.spending_limits[limit_key][call.token].read()
    }

    /// Get remaining currency spending limit
    pub fn get_remaining_currency_limit(
        &self,
        call: getRemainingCurrencyLimitCall,
    ) -> Result<U256> {
        let limit_key = Self::spending_limit_key(call.account, call.keyId);
        let currency_key = Self::currency_key(&call.currency.to_string());
        self.currency_limits[limit_key][currency_key].read()
    }

    /// Get the transaction key used in the current transaction
    pub fn get_transaction_key(
        &self,
        _call: getTransactionKeyCall,
        _msg_sender: Address,
    ) -> Result<Address> {
        self.transaction_key.t_read()
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
    /// Uses transient storage, so the key is automatically cleared after the transaction.
    pub fn set_transaction_key(&mut self, key_id: Address) -> Result<()> {
        self.transaction_key.t_write(key_id)
    }

    /// Sets the transaction origin (tx.origin) for the current transaction.
    ///
    /// Called by the handler before transaction execution.
    /// Uses transient storage, so it's automatically cleared after the transaction.
    pub fn set_tx_origin(&mut self, origin: Address) -> Result<()> {
        self.tx_origin.t_write(origin)
    }

    /// Load and validate a key exists and is not revoked.
    ///
    /// Returns the key if valid, or an error if:
    /// - Key doesn't exist (expiry == 0)
    /// - Key has been revoked
    ///
    /// Note: This does NOT check expiry against current timestamp.
    /// Callers should check expiry separately if needed.
    fn load_active_key(&self, account: Address, key_id: Address) -> Result<AuthorizedKey> {
        let key = self.keys[account][key_id].read()?;

        if key.is_revoked {
            return Err(AccountKeychainError::key_already_revoked().into());
        }

        if key.expiry == 0 {
            return Err(AccountKeychainError::key_not_found().into());
        }

        Ok(key)
    }

    /// Validate keychain authorization (existence, revocation, and expiry)
    ///
    /// This consolidates all validation checks into one method.
    /// Returns Ok(()) if the key is valid and authorized, Err otherwise.
    pub fn validate_keychain_authorization(
        &self,
        account: Address,
        key_id: Address,
        current_timestamp: u64,
    ) -> Result<()> {
        let key = self.load_active_key(account, key_id)?;

        if current_timestamp >= key.expiry {
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

        // Check key is valid (exists and not revoked)
        let key = self.load_active_key(account, key_id)?;

        // If enforce_limits is false, this key has unlimited spending (passthrough)
        if !key.enforce_limits {
            return Ok(());
        }

        let limit_key = Self::spending_limit_key(account, key_id);

        // Check token limit (only if enforceTokenLimits is true)
        if key.enforce_token_limits {
            let token_remaining = self.spending_limits[limit_key][token].read()?;
            if amount > token_remaining {
                return Err(AccountKeychainError::spending_limit_exceeded().into());
            }
            // Update token limit
            self.spending_limits[limit_key][token].write(token_remaining - amount)?;
        }

        // Check currency limit (only if enforceCurrencyLimits is true)
        if key.enforce_currency_limits {
            let token_contract = TIP20Token::from_address(token)?;
            let currency = token_contract.currency()?;
            let currency_key = Self::currency_key(&currency);
            let currency_remaining = self.currency_limits[limit_key][currency_key].read()?;

            // Check if amount exceeds remaining limit
            if amount > currency_remaining {
                return Err(AccountKeychainError::spending_limit_exceeded().into());
            }

            // Update currency limit
            self.currency_limits[limit_key][currency_key].write(currency_remaining - amount)?;
        }

        Ok(())
    }

    /// Authorize a token transfer with access key spending limits
    ///
    /// This method checks if the transaction is using an access key, and if so,
    /// verifies and updates the spending limits for that key.
    /// Should be called before executing a transfer.
    ///
    /// # Arguments
    /// * `account` - The account performing the transfer
    /// * `token` - The token being transferred
    /// * `amount` - The amount being transferred
    ///
    /// # Returns
    /// Ok(()) if authorized (either using main key or access key with sufficient limits)
    /// Err if unauthorized or spending limit exceeded
    pub fn authorize_transfer(
        &mut self,
        account: Address,
        token: Address,
        amount: U256,
    ) -> Result<()> {
        // Get the transaction key for this account
        let transaction_key = self.transaction_key.t_read()?;

        // If using main key (Address::ZERO), no spending limits apply
        if transaction_key == Address::ZERO {
            return Ok(());
        }

        // Only apply spending limits if the caller is the tx origin.
        let tx_origin = self.tx_origin.t_read()?;
        if account != tx_origin {
            return Ok(());
        }

        // Verify and update spending limits for this access key
        self.verify_and_update_spending(account, transaction_key, token, amount)
    }

    /// Authorize a token approval with access key spending limits
    ///
    /// This method checks if the transaction is using an access key, and if so,
    /// verifies and updates the spending limits for that key.
    /// Should be called before executing an approval.
    ///
    /// # Arguments
    /// * `account` - The account performing the approval
    /// * `token` - The token being approved
    /// * `old_approval` - The current approval amount
    /// * `new_approval` - The new approval amount being set
    ///
    /// # Returns
    /// Ok(()) if authorized (either using main key or access key with sufficient limits)
    /// Err if unauthorized or spending limit exceeded
    pub fn authorize_approve(
        &mut self,
        account: Address,
        token: Address,
        old_approval: U256,
        new_approval: U256,
    ) -> Result<()> {
        // Get the transaction key for this account
        let transaction_key = self.transaction_key.t_read()?;

        // If using main key (Address::ZERO), no spending limits apply
        if transaction_key == Address::ZERO {
            return Ok(());
        }

        // Only apply spending limits if the caller is the tx origin.
        let tx_origin = self.tx_origin.t_read()?;
        if account != tx_origin {
            return Ok(());
        }

        // Calculate the increase in approval (only deduct if increasing)
        // If old approval is 100 and new approval is 120, deduct 20 from spending limit
        // If old approval is 100 and new approval is 80, deduct 0 (decreasing approval is free)
        let approval_increase = new_approval.saturating_sub(old_approval);

        // Only check spending limits if there's an increase in approval
        if approval_increase.is_zero() {
            return Ok(());
        }

        // Verify and update spending limits for this access key
        self.verify_and_update_spending(account, transaction_key, token, approval_increase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::TempoPrecompileError,
        storage::{StorageCtx, hashmap::HashMapStorageProvider},
    };
    use alloy::primitives::{Address, U256};
    use tempo_contracts::precompiles::IAccountKeychain::SignatureType;

    // Helper function to assert unauthorized error
    fn assert_unauthorized_error(error: TempoPrecompileError) {
        match error {
            TempoPrecompileError::AccountKeychainError(e) => {
                assert!(
                    matches!(e, AccountKeychainError::UnauthorizedCaller(_)),
                    "Expected UnauthorizedCaller error, got: {e:?}"
                );
            }
            _ => panic!("Expected AccountKeychainError, got: {error:?}"),
        }
    }

    #[test]
    fn test_transaction_key_transient_storage() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let access_key_addr = Address::random();
        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();

            // Test 1: Initially transaction key should be zero
            let initial_key = keychain.transaction_key.t_read()?;
            assert_eq!(
                initial_key,
                Address::ZERO,
                "Initial transaction key should be zero"
            );

            // Test 2: Set transaction key to an access key address
            keychain.set_transaction_key(access_key_addr)?;

            // Test 3: Verify it was stored
            let loaded_key = keychain.transaction_key.t_read()?;
            assert_eq!(loaded_key, access_key_addr, "Transaction key should be set");

            // Test 4: Verify getTransactionKey works
            let get_tx_key_call = getTransactionKeyCall {};
            let result = keychain.get_transaction_key(get_tx_key_call, Address::ZERO)?;
            assert_eq!(
                result, access_key_addr,
                "getTransactionKey should return the set key"
            );

            // Test 5: Clear transaction key
            keychain.set_transaction_key(Address::ZERO)?;
            let cleared_key = keychain.transaction_key.t_read()?;
            assert_eq!(
                cleared_key,
                Address::ZERO,
                "Transaction key should be cleared"
            );

            Ok(())
        })
    }

    #[test]
    fn test_admin_operations_blocked_with_access_key() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let msg_sender = Address::random();
        let existing_key = Address::random();
        let access_key = Address::random();
        let token = Address::random();
        let other = Address::random();
        StorageCtx::enter(&mut storage, || {
            // Initialize the keychain
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            // First, authorize a key with main key (transaction_key = 0) to set up the test
            keychain.set_transaction_key(Address::ZERO)?;
            let setup_call = authorizeKeyCall {
                keyId: existing_key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![],
                currencyLimits: vec![],
            };
            keychain.authorize_key(msg_sender, setup_call)?;

            // Now set transaction key to non-zero (simulating access key usage)
            keychain.set_transaction_key(access_key)?;

            // Test 1: authorize_key should fail with access key
            let auth_call = authorizeKeyCall {
                keyId: other,
                signatureType: SignatureType::P256,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![],
                currencyLimits: vec![],
            };
            let auth_result = keychain.authorize_key(msg_sender, auth_call);
            assert!(
                auth_result.is_err(),
                "authorize_key should fail when using access key"
            );
            assert_unauthorized_error(auth_result.unwrap_err());

            // Test 2: revoke_key should fail with access key
            let revoke_call = revokeKeyCall {
                keyId: existing_key,
            };
            let revoke_result = keychain.revoke_key(msg_sender, revoke_call);
            assert!(
                revoke_result.is_err(),
                "revoke_key should fail when using access key"
            );
            assert_unauthorized_error(revoke_result.unwrap_err());

            // Test 3: update_spending_limit should fail with access key
            let update_call = updateSpendingLimitCall {
                keyId: existing_key,
                token,
                newLimit: U256::from(1000),
            };
            let update_result = keychain.update_spending_limit(msg_sender, update_call);
            assert!(
                update_result.is_err(),
                "update_spending_limit should fail when using access key"
            );
            assert_unauthorized_error(update_result.unwrap_err());

            Ok(())
        })
    }

    #[test]
    fn test_replay_protection_revoked_key_cannot_be_reauthorized() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let account = Address::random();
        let key_id = Address::random();
        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            // Use main key for all operations
            keychain.set_transaction_key(Address::ZERO)?;

            // Step 1: Authorize a key
            let auth_call = authorizeKeyCall {
                keyId: key_id,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: false,
                tokenLimits: vec![],
                currencyLimits: vec![],
            };
            keychain.authorize_key(account, auth_call.clone())?;

            // Verify key exists
            let key_info = keychain.get_key(getKeyCall {
                account,
                keyId: key_id,
            })?;
            assert_eq!(key_info.expiry, u64::MAX);
            assert!(!key_info.isRevoked);

            // Step 2: Revoke the key
            let revoke_call = revokeKeyCall { keyId: key_id };
            keychain.revoke_key(account, revoke_call)?;

            // Verify key is revoked
            let key_info = keychain.get_key(getKeyCall {
                account,
                keyId: key_id,
            })?;
            assert_eq!(key_info.expiry, 0);
            assert!(key_info.isRevoked);

            // Step 3: Try to re-authorize the same key (replay attack)
            // This should fail because the key was revoked
            let replay_result = keychain.authorize_key(account, auth_call);
            assert!(
                replay_result.is_err(),
                "Re-authorizing a revoked key should fail"
            );

            // Verify it's the correct error
            match replay_result.unwrap_err() {
                TempoPrecompileError::AccountKeychainError(e) => {
                    assert!(
                        matches!(e, AccountKeychainError::KeyAlreadyRevoked(_)),
                        "Expected KeyAlreadyRevoked error, got: {e:?}"
                    );
                }
                e => panic!("Expected AccountKeychainError, got: {e:?}"),
            }
            Ok(())
        })
    }

    #[test]
    fn test_different_key_id_can_be_authorized_after_revocation() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let account = Address::random();
        let key_id_1 = Address::random();
        let key_id_2 = Address::random();
        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            // Use main key for all operations
            keychain.set_transaction_key(Address::ZERO)?;

            // Authorize key 1
            let auth_call_1 = authorizeKeyCall {
                keyId: key_id_1,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: false,
                tokenLimits: vec![],
                currencyLimits: vec![],
            };
            keychain.authorize_key(account, auth_call_1)?;

            // Revoke key 1
            keychain.revoke_key(account, revokeKeyCall { keyId: key_id_1 })?;

            // Authorizing a different key (key 2) should still work
            let auth_call_2 = authorizeKeyCall {
                keyId: key_id_2,
                signatureType: SignatureType::P256,
                expiry: 1000,
                enforceLimits: true,
                tokenLimits: vec![],
                currencyLimits: vec![],
            };
            keychain.authorize_key(account, auth_call_2)?;

            // Verify key 2 is authorized
            let key_info = keychain.get_key(getKeyCall {
                account,
                keyId: key_id_2,
            })?;
            assert_eq!(key_info.expiry, 1000);
            assert!(!key_info.isRevoked);

            Ok(())
        })
    }

    #[test]
    fn test_authorize_approve() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let access_key = Address::random();
        let token = Address::random();
        let contract = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            // authorize access key with 100 token spending limit
            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            let auth_call = authorizeKeyCall {
                keyId: access_key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![TokenLimit {
                    token,
                    amount: U256::from(100),
                }],
                currencyLimits: vec![],
            };
            keychain.authorize_key(eoa, auth_call)?;

            let initial_limit = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: access_key,
                token,
            })?;
            assert_eq!(initial_limit, U256::from(100));

            // Switch to access key for remaining tests
            keychain.set_transaction_key(access_key)?;

            // Increase approval by 30, which deducts from the limit
            keychain.authorize_approve(eoa, token, U256::ZERO, U256::from(30))?;

            let limit_after = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: access_key,
                token,
            })?;
            assert_eq!(limit_after, U256::from(70));

            // Decrease approval to 20, does not affect limit
            keychain.authorize_approve(eoa, token, U256::from(30), U256::from(20))?;

            let limit_unchanged = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: access_key,
                token,
            })?;
            assert_eq!(limit_unchanged, U256::from(70));

            // Increase from 20 to 50, reducing the limit by 30
            keychain.authorize_approve(eoa, token, U256::from(20), U256::from(50))?;

            let limit_after_increase = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: access_key,
                token,
            })?;
            assert_eq!(limit_after_increase, U256::from(40));

            // Assert that spending limits only applied when account is tx origin
            keychain.authorize_approve(contract, token, U256::ZERO, U256::from(1000))?;

            let limit_after_contract = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: access_key,
                token,
            })?;
            assert_eq!(limit_after_contract, U256::from(40)); // unchanged

            // Assert that exceeding remaining limit fails
            let exceed_result = keychain.authorize_approve(eoa, token, U256::ZERO, U256::from(50));
            assert!(matches!(
                exceed_result,
                Err(TempoPrecompileError::AccountKeychainError(
                    AccountKeychainError::SpendingLimitExceeded(_)
                ))
            ));

            // Assert that the main key bypasses spending limits, does not affect existing limits
            keychain.set_transaction_key(Address::ZERO)?;
            keychain.authorize_approve(eoa, token, U256::ZERO, U256::from(1000))?;

            let limit_main_key = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: access_key,
                token,
            })?;
            assert_eq!(limit_main_key, U256::from(40));

            Ok(())
        })
    }

    /// Test that spending limits are only enforced when msg_sender == tx_origin.
    ///
    /// This test verifies the fix for the bug where spending limits were incorrectly
    /// applied to contract-initiated transfers. The scenario:
    ///
    /// 1. EOA Alice uses an access key with spending limits
    /// 2. Alice calls a contract that transfers tokens
    /// 3. The contract's transfer should NOT be subject to Alice's spending limits
    ///    (the contract is transferring its own tokens, not Alice's)
    #[test]
    fn test_spending_limits_only_apply_to_tx_origin() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa_alice = Address::random(); // The EOA that signs the transaction
        let access_key = Address::random(); // Alice's access key with spending limits
        let contract_address = Address::random(); // A contract that Alice calls
        let token = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            // Setup: Alice authorizes an access key with a spending limit of 100 tokens
            keychain.set_transaction_key(Address::ZERO)?; // Use main key for setup
            keychain.set_tx_origin(eoa_alice)?;

            let auth_call = authorizeKeyCall {
                keyId: access_key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![TokenLimit {
                    token,
                    amount: U256::from(100),
                }],
                currencyLimits: vec![],
            };
            keychain.authorize_key(eoa_alice, auth_call)?;

            // Verify spending limit is set
            let limit = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa_alice,
                keyId: access_key,
                token,
            })?;
            assert_eq!(
                limit,
                U256::from(100),
                "Initial spending limit should be 100"
            );

            // Now simulate a transaction where Alice uses her access key
            keychain.set_transaction_key(access_key)?;
            keychain.set_tx_origin(eoa_alice)?;

            // Test 1: When msg_sender == tx_origin (Alice directly transfers)
            // Spending limit SHOULD be enforced
            keychain.authorize_transfer(eoa_alice, token, U256::from(30))?;

            let limit_after = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa_alice,
                keyId: access_key,
                token,
            })?;
            assert_eq!(
                limit_after,
                U256::from(70),
                "Spending limit should be reduced to 70 after Alice's direct transfer"
            );

            // Test 2: When msg_sender != tx_origin (contract transfers its own tokens)
            // Spending limit should NOT be enforced - the contract isn't spending Alice's tokens
            keychain.authorize_transfer(contract_address, token, U256::from(1000))?;

            let limit_unchanged = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa_alice,
                keyId: access_key,
                token,
            })?;
            assert_eq!(
                limit_unchanged,
                U256::from(70),
                "Spending limit should remain 70 - contract transfer doesn't affect Alice's limit"
            );

            // Test 3: Alice can still spend her remaining limit
            keychain.authorize_transfer(eoa_alice, token, U256::from(70))?;

            let limit_depleted = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa_alice,
                keyId: access_key,
                token,
            })?;
            assert_eq!(
                limit_depleted,
                U256::ZERO,
                "Spending limit should be depleted after Alice spends remaining 70"
            );

            // Test 4: Alice cannot exceed her spending limit
            let exceed_result = keychain.authorize_transfer(eoa_alice, token, U256::from(1));
            assert!(
                exceed_result.is_err(),
                "Should fail when Alice tries to exceed spending limit"
            );

            // Test 5: But contracts can still transfer (they're not subject to Alice's limits)
            let contract_result =
                keychain.authorize_transfer(contract_address, token, U256::from(999999));
            assert!(
                contract_result.is_ok(),
                "Contract should still be able to transfer even though Alice's limit is depleted"
            );

            Ok(())
        })
    }

    /// Test that keys with enforce_limits=false have unlimited spending (passthrough).
    ///
    /// This test verifies that when a key is authorized with enforce_limits=false,
    /// it acts as a passthrough key with unlimited spending, regardless of whether
    /// token or currency limits are provided.
    #[test]
    fn test_enforce_limits_false_unlimited_spending() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let unlimited_key = Address::random();
        let token = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            // Setup: Authorize a key with enforce_limits=false but pass token limits
            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            let auth_call = authorizeKeyCall {
                keyId: unlimited_key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: false, // Master switch is OFF
                tokenLimits: vec![TokenLimit {
                    token,
                    amount: U256::from(100), // Provide limits, but should be ignored
                }],
                currencyLimits: vec![],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Verify the key info shows enforceLimits=false
            let key_info = keychain.get_key(getKeyCall {
                account: eoa,
                keyId: unlimited_key,
            })?;
            assert!(!key_info.enforceLimits, "Key should have enforceLimits=false");

            // Verify limits were not stored (should be 0)
            let stored_limit = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: unlimited_key,
                token,
            })?;
            assert_eq!(stored_limit, U256::ZERO, "Limits should not be stored");

            // Switch to using the unlimited key
            keychain.set_transaction_key(unlimited_key)?;

            // Test: The key should be able to spend unlimited amounts
            // Even though we "provided" a 100 token limit, it should be ignored

            // Transfer 1000 tokens (way more than the "limit")
            keychain.authorize_transfer(eoa, token, U256::from(1000))?;

            // Verify limit is still 0 (nothing was deducted)
            let limit_after = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: unlimited_key,
                token,
            })?;
            assert_eq!(
                limit_after,
                U256::ZERO,
                "No limit should be deducted for passthrough key"
            );

            // Transfer another 10000 tokens (unlimited spending)
            keychain.authorize_transfer(eoa, token, U256::from(10000))?;

            // Still should succeed with no limit tracking
            let final_limit = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: unlimited_key,
                token,
            })?;
            assert_eq!(
                final_limit,
                U256::ZERO,
                "Passthrough key should never track limits"
            );

            Ok(())
        })
    }

    /// Test that updating spending limits on an unlimited key enables enforce_limits.
    ///
    /// This test verifies that when you call updateSpendingLimit on a key that has
    /// enforce_limits=false, it automatically enables the enforce_limits flag and
    /// starts enforcing limits.
    #[test]
    fn test_update_limit_enables_enforce_limits() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let key = Address::random();
        let token = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            // Setup: Authorize a key with enforce_limits=false (unlimited)
            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            let auth_call = authorizeKeyCall {
                keyId: key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: false, // Unlimited key
                tokenLimits: vec![],
                currencyLimits: vec![],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Verify initially unlimited
            let key_info = keychain.get_key(getKeyCall {
                account: eoa,
                keyId: key,
            })?;
            assert!(!key_info.enforceLimits, "Key should start as unlimited");

            // Update spending limit (should enable enforce_limits)
            let update_call = updateSpendingLimitCall {
                keyId: key,
                token,
                newLimit: U256::from(500),
            };
            keychain.update_spending_limit(eoa, update_call)?;

            // Verify enforce_limits is now enabled
            let key_info_after = keychain.get_key(getKeyCall {
                account: eoa,
                keyId: key,
            })?;
            assert!(
                key_info_after.enforceLimits,
                "Updating limit should enable enforceLimits"
            );
            assert!(
                key_info_after.hasTokenLimits,
                "hasTokenLimits should be true"
            );

            // Verify the limit was set
            let limit = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: key,
                token,
            })?;
            assert_eq!(limit, U256::from(500), "Limit should be set to 500");

            // Now limits should be enforced
            keychain.set_transaction_key(key)?;

            // Spend within limit should work
            keychain.authorize_transfer(eoa, token, U256::from(200))?;

            let remaining = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: key,
                token,
            })?;
            assert_eq!(remaining, U256::from(300), "Limit should be deducted");

            // Exceeding limit should fail
            let exceed_result = keychain.authorize_transfer(eoa, token, U256::from(400));
            assert!(
                exceed_result.is_err(),
                "Should fail when exceeding spending limit"
            );

            Ok(())
        })
    }

    /// Test authorizing a key with currency limits and verifying they are stored.
    #[test]
    fn test_authorize_key_with_currency_limits() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let key = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            // Authorize key with currency limits
            let auth_call = authorizeKeyCall {
                keyId: key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![],
                currencyLimits: vec![
                    CurrencyLimit {
                        limit: U256::from(1000),
                        currency: "USD".to_string(),
                    },
                    CurrencyLimit {
                        limit: U256::from(500),
                        currency: "EUR".to_string(),
                    },
                ],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Verify key info
            let key_info = keychain.get_key(getKeyCall {
                account: eoa,
                keyId: key,
            })?;
            assert!(key_info.enforceLimits, "enforceLimits should be true");
            assert!(
                key_info.hasCurrencyLimits,
                "hasCurrencyLimits should be true"
            );
            assert!(
                !key_info.hasTokenLimits,
                "hasTokenLimits should be false"
            );

            // Verify USD limit
            let usd_limit = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(usd_limit, U256::from(1000), "USD limit should be 1000");

            // Verify EUR limit
            let eur_limit = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "EUR".to_string(),
                },
            )?;
            assert_eq!(eur_limit, U256::from(500), "EUR limit should be 500");

            Ok(())
        })
    }

    /// Integration test: Currency limits are enforced across multiple tokens with the same currency.
    ///
    /// This is the key feature: if you have a USD limit of 1000, it applies to ALL tokens
    /// that return "USD" as their currency, not just a single token.
    ///
    /// This test uses REAL TIP20 tokens to test the full enforcement logic.
    #[test]
    fn test_currency_limits_enforced_across_multiple_tokens() -> eyre::Result<()> {
        use crate::test_util::TIP20Setup;
        use crate::storage::ContractStorage;

        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let key = Address::random();
        let admin = Address::random();

        StorageCtx::enter(&mut storage, || {
            // Create two TIP20 tokens, both with USD currency
            let usdc = TIP20Setup::create("USD Coin", "USDC", admin)
                .currency("USD")
                .with_issuer(admin)
                .with_mint(eoa, U256::from(10000))
                .apply()?;
            let usdc_address = usdc.address();

            let usdt = TIP20Setup::create("Tether", "USDT", admin)
                .currency("USD")
                .with_issuer(admin)
                .with_mint(eoa, U256::from(10000))
                .apply()?;
            let usdt_address = usdt.address();

            // Verify both tokens have USD currency
            assert_eq!(usdc.currency()?, "USD");
            assert_eq!(usdt.currency()?, "USD");

            // Initialize account keychain
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            // Authorize key with USD currency limit of 1000
            // This should apply to BOTH USDC and USDT since both are USD
            let auth_call = authorizeKeyCall {
                keyId: key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![],
                currencyLimits: vec![CurrencyLimit {
                    limit: U256::from(1000),
                    currency: "USD".to_string(),
                }],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Switch to using the access key
            keychain.set_transaction_key(key)?;

            // Test 1: Transfer 400 USDC - should succeed and deduct from USD limit
            keychain.verify_and_update_spending(eoa, key, usdc_address, U256::from(400))?;

            let usd_remaining = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(
                usd_remaining,
                U256::from(600),
                "USD limit should be reduced to 600 after USDC transfer"
            );

            // Test 2: Transfer 300 USDT - should succeed and deduct from SAME USD limit
            keychain.verify_and_update_spending(eoa, key, usdt_address, U256::from(300))?;

            let usd_after_usdt = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(
                usd_after_usdt,
                U256::from(300),
                "USD limit should be reduced to 300 after USDT transfer (shared limit)"
            );

            // Test 3: Try to transfer 400 USDC - should FAIL (only 300 USD remaining)
            let exceed_result =
                keychain.verify_and_update_spending(eoa, key, usdc_address, U256::from(400));
            assert!(
                exceed_result.is_err(),
                "Should fail when exceeding shared USD currency limit"
            );
            assert!(
                matches!(
                    exceed_result.unwrap_err(),
                    TempoPrecompileError::AccountKeychainError(
                        AccountKeychainError::SpendingLimitExceeded(_)
                    )
                ),
                "Should return SpendingLimitExceeded error"
            );

            // Test 4: Transfer remaining 300 USDT - should succeed
            keychain.verify_and_update_spending(eoa, key, usdt_address, U256::from(300))?;

            let usd_depleted = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(
                usd_depleted,
                U256::ZERO,
                "USD limit should be fully depleted"
            );

            // Test 5: Any further transfers should fail
            let final_result =
                keychain.verify_and_update_spending(eoa, key, usdc_address, U256::from(1));
            assert!(
                final_result.is_err(),
                "Should fail when USD limit is depleted"
            );

            Ok(())
        })
    }

    /// Test updating currency limits on an existing key.
    #[test]
    fn test_update_currency_limit() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let key = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            // Authorize key with initial currency limits
            let auth_call = authorizeKeyCall {
                keyId: key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![],
                currencyLimits: vec![CurrencyLimit {
                    limit: U256::from(500),
                    currency: "USD".to_string(),
                }],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Verify initial limit
            let initial = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(initial, U256::from(500));

            // Update USD limit to 2000
            let update_call = updateCurrencyLimitCall {
                keyId: key,
                currency: "USD".to_string(),
                newLimit: U256::from(2000),
            };
            keychain.update_currency_limit(eoa, update_call)?;

            // Verify updated limit
            let updated = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(updated, U256::from(2000));

            Ok(())
        })
    }

    /// Test adding currency limits to an unlimited key via updateCurrencyLimit.
    #[test]
    fn test_update_currency_limit_enables_enforce_limits() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let key = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            // Authorize unlimited key
            let auth_call = authorizeKeyCall {
                keyId: key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: false, // Unlimited
                tokenLimits: vec![],
                currencyLimits: vec![],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Verify initially unlimited
            let key_info = keychain.get_key(getKeyCall {
                account: eoa,
                keyId: key,
            })?;
            assert!(!key_info.enforceLimits);
            assert!(!key_info.hasCurrencyLimits);

            // Add currency limit (should enable enforce_limits)
            let update_call = updateCurrencyLimitCall {
                keyId: key,
                currency: "EUR".to_string(),
                newLimit: U256::from(750),
            };
            keychain.update_currency_limit(eoa, update_call)?;

            // Verify enforce_limits is now enabled
            let key_info_after = keychain.get_key(getKeyCall {
                account: eoa,
                keyId: key,
            })?;
            assert!(key_info_after.enforceLimits, "enforceLimits should be true");
            assert!(
                key_info_after.hasCurrencyLimits,
                "hasCurrencyLimits should be true"
            );

            // Verify the limit was set
            let limit = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "EUR".to_string(),
                },
            )?;
            assert_eq!(limit, U256::from(750));

            Ok(())
        })
    }

    /// Test that both token and currency limits can be set on the same key.
    #[test]
    fn test_combined_token_and_currency_limits() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let key = Address::random();
        let usdc_token = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            keychain.set_transaction_key(Address::ZERO)?;
            keychain.set_tx_origin(eoa)?;

            // Authorize key with BOTH token limits and currency limits
            let auth_call = authorizeKeyCall {
                keyId: key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![TokenLimit {
                    token: usdc_token,
                    amount: U256::from(100),
                }],
                currencyLimits: vec![CurrencyLimit {
                    limit: U256::from(1000),
                    currency: "USD".to_string(),
                }],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Verify key info shows both limit types
            let key_info = keychain.get_key(getKeyCall {
                account: eoa,
                keyId: key,
            })?;
            assert!(key_info.enforceLimits);
            assert!(key_info.hasTokenLimits);
            assert!(key_info.hasCurrencyLimits);

            // Verify token limit
            let token_limit = keychain.get_remaining_limit(getRemainingLimitCall {
                account: eoa,
                keyId: key,
                token: usdc_token,
            })?;
            assert_eq!(token_limit, U256::from(100));

            // Verify currency limit
            let currency_limit = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(currency_limit, U256::from(1000));

            Ok(())
        })
    }

    /// Test that missing currency limits (zero value) are treated as unlimited.
    #[test]
    fn test_missing_currency_limit_is_unlimited() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);

        let eoa = Address::random();
        let key = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut keychain = AccountKeychain::new();
            keychain.initialize()?;

            keychain.set_transaction_key(Address::ZERO)?;

            // Authorize key with only USD limit, no EUR limit
            let auth_call = authorizeKeyCall {
                keyId: key,
                signatureType: SignatureType::Secp256k1,
                expiry: u64::MAX,
                enforceLimits: true,
                tokenLimits: vec![],
                currencyLimits: vec![CurrencyLimit {
                    limit: U256::from(1000),
                    currency: "USD".to_string(),
                }],
            };
            keychain.authorize_key(eoa, auth_call)?;

            // Check USD limit exists
            let usd_limit = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "USD".to_string(),
                },
            )?;
            assert_eq!(usd_limit, U256::from(1000));

            // Check EUR limit (not set) returns 0, which means unlimited
            let eur_limit = keychain.get_remaining_currency_limit(
                getRemainingCurrencyLimitCall {
                    account: eoa,
                    keyId: key,
                    currency: "EUR".to_string(),
                },
            )?;
            assert_eq!(
                eur_limit,
                U256::ZERO,
                "Missing currency limit should be 0 (unlimited)"
            );

            Ok(())
        })
    }
}
