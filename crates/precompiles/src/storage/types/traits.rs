use alloy::primitives::{Address, B256, FixedBytes, U256};
use revm::interpreter::instructions::utility::{IntoAddress, IntoU256};

use crate::error::{Result, TempoPrecompileError};

/// Trait for types that can be stored/loaded in/from a single EVM storage slot.
///
/// This trait provides conversion between Rust types and their U256 storage
/// representation for types that fit within a single 32-byte word. More complex
/// types like dynamic arrays, long strings (>31 bytes), or custom structs that
/// span multiple slots are not covered by this trait.
///
/// # SAFETY
///
/// Implementations must ensure that:
/// - No precission loss in round-trip conversions: `from_u256(x.to_u256()) == Ok(x)`
/// - `to_u256()` always produces valid U256 values
pub trait StorageType: Sized {
    /// Convert from U256 storage representation to this type.
    ///
    /// # Errors
    ///
    /// Returns `TempoPrecompileError::Fatal` if the value cannot be decoded.
    fn from_u256(value: U256) -> Result<Self>;

    /// Convert this type to its U256 storage representation.
    fn to_u256(&self) -> U256;
}

/// Trait for types that can be used as storage mapping keys.
///
/// Keys are hashed using keccak256 along with the mapping's base slot
/// to determine the final storage location. This trait provides the
/// byte representation used in that hash.
pub trait StorageKey {
    fn as_storage_bytes(&self) -> &[u8];
}

// -- STORAGE TYPE IMPLEMENTATIONS ---------------------------------------------

impl StorageType for U256 {
    #[inline]
    fn from_u256(value: U256) -> Result<Self> {
        Ok(value)
    }

    #[inline]
    fn to_u256(&self) -> U256 {
        *self
    }
}

impl StorageType for Address {
    #[inline]
    fn from_u256(value: U256) -> Result<Self> {
        Ok(value.into_address())
    }

    #[inline]
    fn to_u256(&self) -> U256 {
        self.into_u256()
    }
}

impl StorageType for B256 {
    #[inline]
    fn from_u256(value: U256) -> Result<Self> {
        Ok(Self::from(value.to_be_bytes::<32>()))
    }

    #[inline]
    fn to_u256(&self) -> U256 {
        U256::from_be_bytes(self.0)
    }
}

impl StorageType for bool {
    #[inline]
    fn from_u256(value: U256) -> Result<Self> {
        Ok(value != U256::ZERO)
    }

    #[inline]
    fn to_u256(&self) -> U256 {
        if *self { U256::ONE } else { U256::ZERO }
    }
}

impl StorageType for u64 {
    #[inline]
    fn from_u256(value: U256) -> Result<Self> {
        Ok(value.to::<Self>())
    }

    #[inline]
    fn to_u256(&self) -> U256 {
        U256::from(*self)
    }
}

impl StorageType for u128 {
    #[inline]
    fn from_u256(value: U256) -> Result<Self> {
        Ok(value.to::<Self>())
    }

    #[inline]
    fn to_u256(&self) -> U256 {
        U256::from(*self)
    }
}

impl StorageType for i16 {
    #[inline]
    fn from_u256(value: U256) -> Result<Self> {
        // Read as u16 then cast to i16 (preserves bit pattern)
        Ok(value.to::<u16>() as Self)
    }

    #[inline]
    fn to_u256(&self) -> U256 {
        // Cast to u16 to preserve bit pattern, then extend to U256
        U256::from(*self as u16)
    }
}

/// String storage using Solidity's short string optimization.
///
/// Strings up to 31 bytes are stored inline in a single slot with the format:
/// - Bytes 0..len: UTF-8 string data
/// - Byte 31: length * 2 (LSB indicates short string encoding)
///
/// Strings longer than 31 bytes are not currently supported and will panic.
impl StorageType for String {
    fn from_u256(value: U256) -> Result<Self> {
        let bytes = value.to_be_bytes::<32>();
        let len = bytes[31] as usize / 2; // Length stored as len * 2

        if len > 31 {
            return Err(TempoPrecompileError::Fatal(
                "String too long for short string encoding".into(),
            ));
        }

        let utf8_bytes = &bytes[..len];
        Self::from_utf8(utf8_bytes.to_vec()).map_err(|e| {
            TempoPrecompileError::Fatal(format!("Invalid UTF-8 in stored string: {e}"))
        })
    }

    fn to_u256(&self) -> U256 {
        let bytes = self.as_bytes();

        if bytes.len() > 31 {
            panic!("String too long for storage slot: {} bytes", bytes.len());
        }

        let mut storage_bytes = [0u8; 32];
        storage_bytes[..bytes.len()].copy_from_slice(bytes);
        storage_bytes[31] = (bytes.len() * 2) as u8; // Store length * 2

        U256::from_be_bytes(storage_bytes)
    }
}

// -- STORAGE KEY IMPLEMENTATIONS ---------------------------------------------

impl StorageKey for Address {
    #[inline]
    fn as_storage_bytes(&self) -> &[u8] {
        self.as_slice()
    }
}

impl StorageKey for U256 {
    #[inline]
    fn as_storage_bytes(&self) -> &[u8] {
        // U256 needs to be converted to bytes; we'll use a thread-local buffer
        // This is safe because the lifetime is tied to the borrow
        thread_local! {
            static BUFFER: std::cell::RefCell<[u8; 32]> = const { std::cell::RefCell::new([0u8; 32]) };
        }

        BUFFER.with(|buf| {
            let mut buffer = buf.borrow_mut();
            *buffer = self.to_be_bytes();
            // SAFETY: The buffer lives in TLS and we're returning a reference
            // that cannot outlive this function call. The caller must use it
            // immediately before any other code can access the TLS buffer.
            unsafe { std::slice::from_raw_parts(buffer.as_ptr(), 32) }
        })
    }
}

impl StorageKey for B256 {
    #[inline]
    fn as_storage_bytes(&self) -> &[u8] {
        self.as_slice()
    }
}

impl StorageKey for FixedBytes<4> {
    #[inline]
    fn as_storage_bytes(&self) -> &[u8] {
        self.as_slice()
    }
}

impl StorageKey for u64 {
    #[inline]
    fn as_storage_bytes(&self) -> &[u8] {
        thread_local! {
            static BUFFER: std::cell::RefCell<[u8; 8]> = const { std::cell::RefCell::new([0u8; 8]) };
        }

        BUFFER.with(|buf| {
            let mut buffer = buf.borrow_mut();
            *buffer = self.to_be_bytes();
            unsafe { std::slice::from_raw_parts(buffer.as_ptr(), 8) }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    #[test]
    fn test_u256_round_trip() {
        let value = U256::from(12345u64);
        let stored = value.to_u256();
        let loaded = U256::from_u256(stored).unwrap();
        assert_eq!(value, loaded);
    }

    #[test]
    fn test_address_round_trip() {
        let addr = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let stored = addr.to_u256();
        let loaded = Address::from_u256(stored).unwrap();
        assert_eq!(addr, loaded);
    }

    #[test]
    fn test_b256_round_trip() {
        let value = B256::from([0x42u8; 32]);
        let stored = value.to_u256();
        let loaded = B256::from_u256(stored).unwrap();
        assert_eq!(value, loaded);
    }

    #[test]
    fn test_bool_conversions() {
        // true -> ONE -> true
        assert_eq!(true.to_u256(), U256::ONE);
        assert!(bool::from_u256(U256::ONE).unwrap());

        // false -> ZERO -> false
        assert_eq!(false.to_u256(), U256::ZERO);
        assert!(!bool::from_u256(U256::ZERO).unwrap());

        // Any non-zero value is true
        assert!(bool::from_u256(U256::from(42)).unwrap());
        assert!(bool::from_u256(U256::MAX).unwrap());
    }

    #[test]
    fn test_u64_round_trip() {
        let value = u64::MAX;
        let stored = value.to_u256();
        let loaded = u64::from_u256(stored).unwrap();
        assert_eq!(value, loaded);
    }

    #[test]
    fn test_u128_round_trip() {
        let value = u128::MAX;
        let stored = value.to_u256();
        let loaded = u128::from_u256(stored).unwrap();
        assert_eq!(value, loaded);
    }

    #[test]
    fn test_i16_round_trip() {
        // Positive value
        let pos = i16::MAX;
        assert_eq!(i16::from_u256(pos.to_u256()).unwrap(), pos);

        // Negative value (two's complement)
        let neg = i16::MIN;
        assert_eq!(i16::from_u256(neg.to_u256()).unwrap(), neg);

        // Zero
        let zero = 0i16;
        assert_eq!(i16::from_u256(zero.to_u256()).unwrap(), zero);
    }

    #[test]
    fn test_string_empty() {
        let s = String::new();
        let stored = s.to_u256();
        let loaded = String::from_u256(stored).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_string_short() {
        let s = "Hello, Tempo!".to_string();
        assert!(s.len() <= 31, "Test string must be <= 31 bytes");

        let stored = s.to_u256();
        let loaded = String::from_u256(stored).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_string_max_length() {
        // 31 bytes is the maximum for short string encoding
        let s = "a".repeat(31);
        assert_eq!(s.len(), 31);

        let stored = s.to_u256();
        let loaded = String::from_u256(stored).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    #[should_panic(expected = "String too long")]
    fn test_string_too_long_panics() {
        let s = "a".repeat(32); // 32 bytes > 31 byte limit
        let _stored = s.to_u256();
    }

    #[test]
    fn test_string_unicode() {
        let s = "Hello 世界 🌍".to_string();
        assert!(s.len() <= 31, "Test string too long");

        let stored = s.to_u256();
        let loaded = String::from_u256(stored).unwrap();
        assert_eq!(s, loaded);
    }

    #[test]
    fn test_string_storage_format() {
        let s = "test".to_string(); // 4 bytes
        let stored = s.to_u256();
        let bytes = stored.to_be_bytes::<32>();

        // Check first 4 bytes contain "test"
        assert_eq!(&bytes[0..4], b"test");

        // Check rest is zeros
        assert!(bytes[4..31].iter().all(|&b| b == 0));

        // Check length byte: 4 * 2 = 8
        assert_eq!(bytes[31], 8);
    }

    #[test]
    fn test_address_as_storage_bytes() {
        let addr = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let bytes = addr.as_storage_bytes();
        assert_eq!(bytes.len(), 20);
        assert_eq!(bytes, addr.as_slice());
    }

    #[test]
    fn test_u256_as_storage_bytes() {
        let value = U256::from(0x123456789abcdef_u64);
        let bytes = value.as_storage_bytes();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn test_b256_as_storage_bytes() {
        let value = B256::from([0x42u8; 32]);
        let bytes = value.as_storage_bytes();
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes, value.as_slice());
    }

    #[test]
    fn test_u64_as_storage_bytes() {
        let value = 0x123456789abcdef_u64;
        let bytes = value.as_storage_bytes();
        assert_eq!(bytes.len(), 8);
        assert_eq!(bytes, &value.to_be_bytes());
    }
}
