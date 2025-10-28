//! Core types for 2D nonce implementation

use alloy_primitives::Address;

/// 192-bit nonce key (24 bytes)
pub type U192 = [u8; 24];

/// Unique identifier for a sender + nonce_key combination
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub struct SenderKey {
    pub sender: Address,
    pub nonce_key: U192,
}

impl SenderKey {
    /// Create a new sender key
    pub fn new(sender: Address, nonce_key: U192) -> Self {
        Self { sender, nonce_key }
    }

    /// Create from u64 for convenience
    pub fn from_u64(sender: Address, key: u64) -> Self {
        let mut nonce_key = [0u8; 24];
        nonce_key[16..24].copy_from_slice(&key.to_be_bytes());
        Self { sender, nonce_key }
    }
}
