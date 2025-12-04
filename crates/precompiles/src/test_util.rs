//! Test utilities for precompile dispatch testing

use crate::{
    PATH_USD_ADDRESS, Precompile, Result,
    storage::{StorageContext, hashmap::HashMapStorageProvider},
    tip20::{self, ITIP20, TIP20Token},
    tip20_factory::{self, TIP20Factory},
};
use alloy::{
    primitives::{Address, B256, Bytes, U256},
    sol_types::SolError,
};
use revm::precompile::PrecompileError;
use tempo_contracts::precompiles::{TIP20_FACTORY_ADDRESS, UnknownFunctionSelector};

/// Checks that all selectors in an interface have dispatch handlers.
///
/// Calls each selector with dummy parameters and checks for "Unknown function selector" errors.
/// Returns unsupported selectors as `(selector_bytes, function_name)` tuples.
pub fn check_selector_coverage<P: Precompile>(
    precompile: &mut P,
    selectors: &[[u8; 4]],
    interface_name: &str,
    name_lookup: impl Fn([u8; 4]) -> Option<&'static str>,
) -> Vec<([u8; 4], &'static str)> {
    let mut unsupported_selectors = Vec::new();

    for selector in selectors.iter() {
        let mut calldata = selector.to_vec();
        // Add some dummy data for functions that require parameters
        calldata.extend_from_slice(&[0u8; 32]);

        let result = precompile.call(&Bytes::from(calldata), Address::ZERO);

        // Check if we got "Unknown function selector" error (old format)
        let is_unsupported_old = matches!(&result,
            Err(PrecompileError::Other(msg)) if msg.contains("Unknown function selector")
        );

        // Check if we got "Unknown function selector" error (new format - ABI-encoded)
        let is_unsupported_new = if let Ok(output) = &result {
            output.reverted && UnknownFunctionSelector::abi_decode(&output.bytes).is_ok()
        } else {
            false
        };

        if (is_unsupported_old || is_unsupported_new)
            && let Some(name) = name_lookup(*selector)
        {
            unsupported_selectors.push((*selector, name));
        }
    }

    // Print unsupported selectors for visibility
    if !unsupported_selectors.is_empty() {
        eprintln!("Unsupported {interface_name} selectors:");
        for (selector, name) in &unsupported_selectors {
            eprintln!("  - {name} ({selector:?})");
        }
    }

    unsupported_selectors
}

/// Asserts that multiple selector coverage checks all pass (no unsupported selectors).
///
/// Takes an iterator of unsupported selector results and panics if any are found.
pub fn assert_full_coverage(results: impl IntoIterator<Item = Vec<([u8; 4], &'static str)>>) {
    let all_unsupported: Vec<_> = results
        .into_iter()
        .flat_map(|r| r.into_iter())
        .map(|(_, name)| name)
        .collect();

    assert!(
        all_unsupported.is_empty(),
        "Found {} unsupported selectors: {:?}",
        all_unsupported.len(),
        all_unsupported
    );
}

/// Helper to create a test storage provider with a random address
pub fn setup_storage() -> (HashMapStorageProvider, Address) {
    (HashMapStorageProvider::new(1), Address::random())
}

/// Builder for TIP20 token setup in tests.
///
/// Handles PathUSD initialization, factory creation, role grants, minting,
/// approvals, and reward configuration in a single chainable API.
#[derive(Default)]
#[cfg(any(test, feature = "test-utils"))]
pub struct TIP20Builder {
    name: String,
    symbol: String,
    currency: String,
    quote_token: Option<Address>,
    admin: Address,
    fee_recipient: Address,
    roles: Vec<(Address, B256)>,
    mints: Vec<(Address, U256)>,
    approvals: Vec<(Address, Address, U256)>,
    reward_opt_ins: Vec<Address>,
    reward_streams: Vec<(U256, u32)>,
}

#[cfg(any(test, feature = "test-utils"))]
impl TIP20Builder {
    /// Create a new token builder with required fields.
    ///
    /// Defaults to `currency: "USD"`, `quote_token: PathUSD`, `fee_recipient: Address::ZERO`
    pub fn new(name: &str, symbol: &str, admin: Address) -> Self {
        Self {
            name: name.to_string(),
            symbol: symbol.to_string(),
            currency: "USD".to_string(),
            admin,
            ..Default::default()
        }
    }

    /// Set the token currency (default: "USD").
    pub fn currency(mut self, currency: &str) -> Self {
        self.currency = currency.to_string();
        self
    }

    /// Set a custom quote token (default: PathUSD).
    pub fn quote_token(mut self, token: Address) -> Self {
        self.quote_token = Some(token);
        self
    }

    /// Set the fee recipient address.
    pub fn fee_recipient(mut self, recipient: Address) -> Self {
        self.fee_recipient = recipient;
        self
    }

    /// Grant ISSUER_ROLE to an account.
    pub fn with_issuer(self, account: Address) -> Self {
        use crate::tip20::ISSUER_ROLE;
        self.with_role(account, *ISSUER_ROLE)
    }

    /// Grant an arbitrary role to an account.
    pub fn with_role(mut self, account: Address, role: B256) -> Self {
        self.roles.push((account, role));
        self
    }

    /// Mint tokens to an address after creation.
    ///
    /// Note: Requires ISSUER_ROLE on admin (use `with_issuer(admin)`).
    pub fn with_mint(mut self, to: Address, amount: U256) -> Self {
        self.mints.push((to, amount));
        self
    }

    /// Set an approval from owner to spender.
    pub fn with_approval(mut self, owner: Address, spender: Address, amount: U256) -> Self {
        self.approvals.push((owner, spender, amount));
        self
    }

    /// Opt a user into rewards (sets reward recipient to themselves).
    pub fn with_reward_opt_in(mut self, user: Address) -> Self {
        self.reward_opt_ins.push(user);
        self
    }

    /// Start a reward stream (requires tokens minted to admin first).
    pub fn with_reward_stream(mut self, amount: U256, duration_secs: u32) -> Self {
        self.reward_streams.push((amount, duration_secs));
        self
    }

    /// Initialize PathUSD (token 0) if needed and return it.
    pub fn path_usd(admin: Address) -> Result<TIP20Token> {
        if is_initialized(PATH_USD_ADDRESS) {
            return Ok(TIP20Token::from_address(PATH_USD_ADDRESS));
        }

        // In Allegretto, PathUSD is token 0 created via factory with quoteToken=0
        if StorageContext.spec().is_allegretto() {
            let mut factory = Self::factory()?;
            factory.create_token(
                admin,
                tip20_factory::ITIP20Factory::createTokenCall {
                    name: "PathUSD".to_string(),
                    symbol: "PUSD".to_string(),
                    currency: "USD".to_string(),
                    quoteToken: Address::ZERO,
                    admin,
                },
            )?;
        } else {
            // Pre-Allegretto: direct initialization
            TIP20Token::from_address(PATH_USD_ADDRESS).initialize(
                "PathUSD",
                "PUSD",
                "USD",
                Address::ZERO,
                admin,
                Address::ZERO,
            )?;
        }

        Ok(TIP20Token::from_address(PATH_USD_ADDRESS))
    }

    /// Initialize the TIP20 factory if needed.
    pub fn factory() -> Result<TIP20Factory> {
        let mut factory = TIP20Factory::new();
        if !is_initialized(TIP20_FACTORY_ADDRESS) {
            factory.initialize()?;
        }
        Ok(factory)
    }

    /// Build the token, returning just the TIP20Token.
    pub fn build(self) -> Result<TIP20Token> {
        self.build_with_id().map(|(_, token)| token)
    }

    /// Build the token, returning both token_id and TIP20Token.
    pub fn build_with_id(self) -> Result<(u64, TIP20Token)> {
        // Initialize factory and pathUSD if needed
        let mut factory = Self::factory()?;
        let _ = Self::path_usd(self.admin)?;

        // Create token via factory
        let quote = self.quote_token.unwrap_or(PATH_USD_ADDRESS);
        let token_address = factory.create_token(
            self.admin,
            tip20_factory::ITIP20Factory::createTokenCall {
                name: self.name,
                symbol: self.symbol,
                currency: self.currency,
                quoteToken: quote,
                admin: self.admin,
            },
        )?;

        let token_id = tip20::address_to_token_id_unchecked(token_address);
        let mut token = TIP20Token::new(token_id);

        // Apply roles
        for (account, role) in self.roles {
            token.grant_role_internal(account, role)?;
        }

        // Apply mints
        for (to, amount) in self.mints {
            token.mint(self.admin, ITIP20::mintCall { to, amount })?;
        }

        // Apply approvals
        for (owner, spender, amount) in self.approvals {
            token.approve(owner, ITIP20::approveCall { spender, amount })?;
        }

        // Apply reward opt-ins
        for user in self.reward_opt_ins {
            token.set_reward_recipient(user, ITIP20::setRewardRecipientCall { recipient: user })?;
        }

        // Start reward streams
        for (amount, secs) in self.reward_streams {
            token.start_reward(self.admin, ITIP20::startRewardCall { amount, secs })?;
        }

        Ok((token_id, token))
    }
}

/// Checks if a contract at the given address has bytecode deployed.
#[cfg(any(test, feature = "test-utils"))]
fn is_initialized(address: Address) -> bool {
    crate::storage::StorageContext.has_bytecode(address)
}

/// Test helper function for constructing EVM words from hex string literals.
///
/// Takes an array of hex strings (with or without "0x" prefix), concatenates
/// them left-to-right, left-pads with zeros to 32 bytes, and returns a U256.
///
/// # Example
/// ```ignore
/// let word = gen_word_from(&[
///     "0x2a",                                        // 1 byte
///     "0x1111111111111111111111111111111111111111",  // 20 bytes
///     "0x01",                                        // 1 byte
/// ]);
/// // Produces: [10 zeros] [0x2a] [20 bytes of 0x11] [0x01]
/// ```
pub fn gen_word_from(values: &[&str]) -> U256 {
    let mut bytes = Vec::new();

    for value in values {
        let hex_str = value.strip_prefix("0x").unwrap_or(value);

        // Parse hex string to bytes
        assert!(
            hex_str.len() % 2 == 0,
            "Hex string '{value}' has odd length"
        );

        for i in (0..hex_str.len()).step_by(2) {
            let byte_str = &hex_str[i..i + 2];
            let byte = u8::from_str_radix(byte_str, 16)
                .unwrap_or_else(|e| panic!("Invalid hex in '{value}': {e}"));
            bytes.push(byte);
        }
    }

    assert!(
        bytes.len() <= 32,
        "Total bytes ({}) exceed 32-byte slot limit",
        bytes.len()
    );

    // Left-pad with zeros to 32 bytes
    let mut slot_bytes = [0u8; 32];
    let start_idx = 32 - bytes.len();
    slot_bytes[start_idx..].copy_from_slice(&bytes);

    U256::from_be_bytes(slot_bytes)
}
