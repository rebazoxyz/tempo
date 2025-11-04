use alloy::primitives::{Address, Bytes, U256, keccak256};
use revm::interpreter::instructions::utility::{IntoAddress, IntoU256};
use tempo_precompiles_macros::{storable_alloy_bytes, storable_alloy_ints, storable_rust_ints};

use crate::{
    error::{Result, TempoPrecompileError},
    storage::StorageOps,
};

/// Helper trait to access byte count without requiring const generic parameter.
///
/// This trait exists to allow the derive macro to query the byte size of field types
/// during layout computation, before the slot count is known.
pub trait StorableType {
    /// Number of bytes that the type requires.
    ///
    /// Only accurate for those types with a size known at compile-time (fixed-size).
    /// Otherwise, set to a full 32-byte slot.
    const BYTE_COUNT: usize;
}

/// Trait for types that can be stored/loaded from EVM storage.
///
/// This trait provides a flexible abstraction for reading and writing Rust types
/// to EVM storage. Types can occupy one or more consecutive storage slots, enabling
/// support for both simple values (Address, U256, bool) and complex multi-slot types
/// (structs, fixed arrays).
///
/// # Type Parameter
///
/// - `N`: The number of consecutive storage slots this type occupies.
///   For single-word types (Address, U256, bool), this is `1`.
///   For fixed-size arrays, this equals the number of elements.
///   For user-defined structs, this a number between `1` and the number of fields, which depends on slot packing.
///
/// # Storage Layout
///
/// For a type with `N = 3` starting at `base_slot`:
/// - Slot 0: `base_slot + 0`
/// - Slot 1: `base_slot + 1`
/// - Slot 2: `base_slot + 2`
///
/// # Safety
///
/// Implementations must ensure that:
/// - Round-trip conversions preserve data: `load(store(x)) == Ok(x)`
/// - `N` accurately reflects the number of slots used
/// - `store` and `load` access exactly `N` consecutive slots
/// - `to_evm_words` and `from_evm_words` produce/consume exactly `N` words
pub trait Storable<const N: usize>: Sized + StorableType {
    /// Load this type from storage starting at the given base slot.
    ///
    /// Reads `N` consecutive slots starting from `base_slot`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Storage read fails
    /// - Data cannot be decoded into this type
    fn load<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<Self>;

    /// Store this type to storage starting at the given base slot.
    ///
    /// Writes `N` consecutive slots starting from `base_slot`.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage write fails.
    fn store<S: StorageOps>(&self, storage: &mut S, base_slot: U256) -> Result<()>;

    /// Delete this type from storage (set all slots to zero).
    ///
    /// Sets `N` consecutive slots to zero, starting from `base_slot`.
    ///
    /// The default implementation sets each slot to zero individually.
    /// Types may override this for optimized bulk deletion.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage write fails.
    fn delete<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<()> {
        for offset in 0..N {
            storage.sstore(base_slot + U256::from(offset), U256::ZERO)?;
        }
        Ok(())
    }

    /// Encode this type to an array of U256 words.
    ///
    /// Returns exactly `N` words, where each word represents one storage slot.
    /// For single-slot types (`N = 1`), returns a single-element array.
    /// For multi-slot types, each array element corresponds to one slot's data.
    ///
    /// # Packed Storage
    ///
    /// When multiple small fields are packed into a single slot, they are
    /// positioned and combined into a single U256 word according to their
    /// byte offsets. The derive macro handles this automatically.
    fn to_evm_words(&self) -> Result<[U256; N]>;

    /// Decode this type from an array of U256 words.
    ///
    /// Accepts exactly `N` words, where each word represents one storage slot.
    /// Constructs the complete type from all provided words.
    ///
    /// # Packed Storage
    ///
    /// When multiple small fields are packed into a single slot, they are
    /// extracted from the appropriate word using bit shifts and masks.
    /// The derive macro handles this automatically.
    fn from_evm_words(words: [U256; N]) -> Result<Self>;
}

/// Trait for types that can be used as storage mapping keys.
///
/// Keys are hashed using keccak256 along with the mapping's base slot
/// to determine the final storage location. This trait provides the
/// byte representation used in that hash.
pub trait StorageKey {
    fn as_storage_bytes(&self) -> impl AsRef<[u8]>;
}

// -- STORAGE KEY IMPLEMENTATIONS ---------------------------------------------

impl StorageKey for Address {
    #[inline]
    fn as_storage_bytes(&self) -> impl AsRef<[u8]> {
        self.as_slice()
    }
}

// -- STORAGE TYPE IMPLEMENTATIONS ---------------------------------------------

storable_rust_ints!();
storable_alloy_ints!();
storable_alloy_bytes!();

impl StorableType for bool {
    const BYTE_COUNT: usize = 1;
}

impl Storable<1> for bool {
    #[inline]
    fn load<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<Self> {
        let value = storage.sload(base_slot)?;
        Ok(value != U256::ZERO)
    }

    #[inline]
    fn store<S: StorageOps>(&self, storage: &mut S, base_slot: U256) -> Result<()> {
        let value = if *self { U256::ONE } else { U256::ZERO };
        storage.sstore(base_slot, value)
    }

    #[inline]
    fn to_evm_words(&self) -> Result<[U256; 1]> {
        Ok([if *self { U256::ONE } else { U256::ZERO }])
    }

    #[inline]
    fn from_evm_words(words: [U256; 1]) -> Result<Self> {
        Ok(words[0] != U256::ZERO)
    }
}

impl StorableType for Address {
    const BYTE_COUNT: usize = 20;
}

impl Storable<1> for Address {
    #[inline]
    fn load<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<Self> {
        let value = storage.sload(base_slot)?;
        Ok(value.into_address())
    }

    #[inline]
    fn store<S: StorageOps>(&self, storage: &mut S, base_slot: U256) -> Result<()> {
        storage.sstore(base_slot, self.into_u256())
    }

    #[inline]
    fn to_evm_words(&self) -> Result<[U256; 1]> {
        Ok([self.into_u256()])
    }

    #[inline]
    fn from_evm_words(words: [U256; 1]) -> Result<Self> {
        Ok(words[0].into_address())
    }
}

impl StorableType for Bytes {
    const BYTE_COUNT: usize = 32;
}

impl Storable<1> for Bytes {
    #[inline]
    fn load<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<Self> {
        load_bytes_like(storage, base_slot, |data| Ok(Self::from(data)))
    }

    #[inline]
    fn store<S: StorageOps>(&self, storage: &mut S, base_slot: U256) -> Result<()> {
        store_bytes_like(self.as_ref(), storage, base_slot)
    }

    #[inline]
    fn delete<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<()> {
        delete_bytes_like(storage, base_slot)
    }

    #[inline]
    fn to_evm_words(&self) -> Result<[U256; 1]> {
        to_evm_words_bytes_like(self.as_ref())
    }

    #[inline]
    fn from_evm_words(words: [U256; 1]) -> Result<Self> {
        from_evm_words_bytes_like(words, |data| Ok(Self::from(data)))
    }
}

impl StorableType for String {
    const BYTE_COUNT: usize = 32;
}

impl Storable<1> for String {
    #[inline]
    fn load<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<Self> {
        load_bytes_like(storage, base_slot, |data| {
            Self::from_utf8(data).map_err(|e| {
                TempoPrecompileError::Fatal(format!("Invalid UTF-8 in stored string: {e}"))
            })
        })
    }

    #[inline]
    fn store<S: StorageOps>(&self, storage: &mut S, base_slot: U256) -> Result<()> {
        store_bytes_like(self.as_bytes(), storage, base_slot)
    }

    #[inline]
    fn delete<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<()> {
        delete_bytes_like(storage, base_slot)
    }

    #[inline]
    fn to_evm_words(&self) -> Result<[U256; 1]> {
        to_evm_words_bytes_like(self.as_bytes())
    }

    #[inline]
    fn from_evm_words(words: [U256; 1]) -> Result<Self> {
        from_evm_words_bytes_like(words, |data| {
            Self::from_utf8(data).map_err(|e| {
                TempoPrecompileError::Fatal(format!("Invalid UTF-8 in stored string: {e}"))
            })
        })
    }
}

/// Generic load implementation for string-like types (String, Bytes) using Solidity's encoding.
///
/// **Short strings (≤31 bytes)** are stored inline in a single slot:
/// - Bytes 0..len: UTF-8 string data (left-aligned)
/// - Byte 31 (LSB): length * 2 (bit 0 = 0 indicates short string)
///
/// **Long strings (≥32 bytes)** use keccak256-based storage:
/// - Base slot: stores `length * 2 + 1` (bit 0 = 1 indicates long string)
/// - Data slots: stored at `keccak256(main_slot) + i` for each 32-byte chunk
///
/// The converter function transforms raw bytes into the target type.
#[inline]
fn load_bytes_like<T, S, F>(storage: &mut S, base_slot: U256, into: F) -> Result<T>
where
    S: StorageOps,
    F: FnOnce(Vec<u8>) -> Result<T>,
{
    let base_value = storage.sload(base_slot)?;
    let is_long = is_long_string(base_value);
    let length = extract_string_length(base_value, is_long);

    if is_long {
        // Long string: read data from keccak256(base_slot) + i
        let slot_start = compute_string_data_slot(base_slot);
        let chunks = calc_chunks(length);
        let mut data = Vec::with_capacity(length);

        for i in 0..chunks {
            let slot = slot_start + U256::from(i);
            let chunk_value = storage.sload(slot)?;
            let chunk_bytes = chunk_value.to_be_bytes::<32>();

            // For the last chunk, only take the remaining bytes
            let bytes_to_take = if i == chunks - 1 {
                length - (i * 32)
            } else {
                32
            };
            data.extend_from_slice(&chunk_bytes[..bytes_to_take]);
        }

        into(data)
    } else {
        // Short string: data is inline in the main slot
        let bytes = base_value.to_be_bytes::<32>();
        into(bytes[..length].to_vec())
    }
}

/// Generic store implementation for byte-like types (String, Bytes) using Solidity's encoding.
///
/// **Short strings (≤31 bytes)** are stored inline in a single slot:
/// - Bytes 0..len: UTF-8 string data (left-aligned)
/// - Byte 31 (LSB): length * 2 (bit 0 = 0 indicates short string)
///
/// **Long strings (≥32 bytes)** use keccak256-based storage:
/// - Base slot: stores `length * 2 + 1` (bit 0 = 1 indicates long string)
/// - Data slots: stored at `keccak256(main_slot) + i` for each 32-byte chunk
#[inline]
fn store_bytes_like<S: StorageOps>(bytes: &[u8], storage: &mut S, base_slot: U256) -> Result<()> {
    let length = bytes.len();

    if length <= 31 {
        storage.sstore(base_slot, encode_short_string(bytes))
    } else {
        storage.sstore(base_slot, encode_long_string_length(length))?;

        // Store data in chunks at keccak256(base_slot) + i
        let slot_start = compute_string_data_slot(base_slot);
        let chunks = calc_chunks(length);

        for i in 0..chunks {
            let slot = slot_start + U256::from(i);
            let chunk_start = i * 32;
            let chunk_end = (chunk_start + 32).min(length);
            let chunk = &bytes[chunk_start..chunk_end];

            // Pad chunk to 32 bytes if it's the last chunk
            let mut chunk_bytes = [0u8; 32];
            chunk_bytes[..chunk.len()].copy_from_slice(chunk);

            storage.sstore(slot, U256::from_be_bytes(chunk_bytes))?;
        }

        Ok(())
    }
}

/// Generic delete implementation for byte-like types (String, Bytes) using Solidity's encoding.
///
/// Clears both the main slot and any keccak256-addressed data slots for long strings.
#[inline]
fn delete_bytes_like<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<()> {
    let base_value = storage.sload(base_slot)?;
    let is_long = is_long_string(base_value);

    if is_long {
        // Long string: need to clear data slots as well
        let length = extract_string_length(base_value, true);
        let slot_start = compute_string_data_slot(base_slot);
        let chunks = calc_chunks(length);

        // Clear all data slots
        for i in 0..chunks {
            let slot = slot_start + U256::from(i);
            storage.sstore(slot, U256::ZERO)?;
        }
    }

    // Clear the main slot
    storage.sstore(base_slot, U256::ZERO)
}

/// Returns the encoded length for long strings or the inline data for short strings.
#[inline]
fn to_evm_words_bytes_like(bytes: &[u8]) -> Result<[U256; 1]> {
    let length = bytes.len();

    if length <= 31 {
        Ok([encode_short_string(bytes)])
    } else {
        // Note: actual string data is in keccak256-addressed slots (not included here)
        Ok([encode_long_string_length(length)])
    }
}

/// The converter function transforms raw bytes into the target type.
/// Returns an error for long strings, which require storage access to reconstruct.
#[inline]
fn from_evm_words_bytes_like<T, F>(words: [U256; 1], into: F) -> Result<T>
where
    F: FnOnce(Vec<u8>) -> Result<T>,
{
    let slot_value = words[0];
    let is_long = is_long_string(slot_value);

    if is_long {
        // Long string: cannot reconstruct without storage access to keccak256-addressed data
        Err(TempoPrecompileError::Fatal(
            "Cannot reconstruct long string from single word. Use load() instead.".into(),
        ))
    } else {
        // Short string: data is inline in the word
        let length = extract_string_length(slot_value, false);
        let bytes = slot_value.to_be_bytes::<32>();
        into(bytes[..length].to_vec())
    }
}

/// Compute the storage slot where long string data begins.
///
/// For long strings (≥32 bytes), data is stored starting at `keccak256(base_slot)`.
#[inline]
fn compute_string_data_slot(base_slot: U256) -> U256 {
    U256::from_be_bytes(keccak256(base_slot.to_be_bytes::<32>()).0)
}

/// Check if a storage slot value represents a long string.
///
/// Solidity string encoding uses bit 0 of the LSB to distinguish:
/// - Bit 0 = 0: Short string (≤31 bytes)
/// - Bit 0 = 1: Long string (≥32 bytes)
#[inline]
fn is_long_string(slot_value: U256) -> bool {
    (slot_value.byte(0) & 1) != 0
}

/// Extract the string length from a storage slot value.
///
/// # Arguments
/// * `slot_value` - The value stored in the main slot
/// * `is_long` - Whether this is a long string (from `is_long_string()`)
///
/// # Returns
/// The length of the string in bytes
#[inline]
fn extract_string_length(slot_value: U256, is_long: bool) -> usize {
    if is_long {
        // Long string: slot stores (length * 2 + 1)
        // Extract length: (value - 1) / 2
        let length_times_two_plus_one: U256 = slot_value;
        let length_times_two: U256 = length_times_two_plus_one - U256::from(1);
        let length_u256: U256 = length_times_two >> 1;
        length_u256.to::<usize>()
    } else {
        // Short string: LSB stores (length * 2)
        // Extract length: LSB / 2
        let bytes = slot_value.to_be_bytes::<32>();
        (bytes[31] / 2) as usize
    }
}

/// Compute the number of 32-byte chunks needed to store a byte string.
#[inline]
fn calc_chunks(byte_length: usize) -> usize {
    byte_length.div_ceil(32)
}

/// Encode a short string (≤31 bytes) into a U256 for inline storage.
///
/// Format: bytes left-aligned, LSB contains (length * 2)
#[inline]
fn encode_short_string(bytes: &[u8]) -> U256 {
    let mut storage_bytes = [0u8; 32];
    storage_bytes[..bytes.len()].copy_from_slice(bytes);
    storage_bytes[31] = (bytes.len() * 2) as u8;
    U256::from_be_bytes(storage_bytes)
}

/// Encode the length metadata for a long string (≥32 bytes).
///
/// Returns `length * 2 + 1` where bit 0 = 1 indicates long string storage.
#[inline]
fn encode_long_string_length(byte_length: usize) -> U256 {
    U256::from(byte_length * 2 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PrecompileStorageProvider, hashmap::HashMapStorageProvider};
    use proptest::prelude::*;

    // Test helper that owns storage and implements StorageOps
    struct TestContract {
        address: Address,
        storage: HashMapStorageProvider,
    }

    impl StorageOps for TestContract {
        fn sstore(&mut self, slot: U256, value: U256) -> Result<()> {
            self.storage.sstore(self.address, slot, value)
        }

        fn sload(&mut self, slot: U256) -> Result<U256> {
            self.storage.sload(self.address, slot)
        }
    }

    /// Helper to create a test contract with fresh storage.
    fn setup_test_contract() -> TestContract {
        TestContract {
            address: Address::random(),
            storage: HashMapStorageProvider::new(1),
        }
    }

    #[test]
    fn test_address_round_trip() {
        let mut contract = setup_test_contract();
        let addr = Address::random();
        let slot = U256::from(1);

        addr.store(&mut contract, slot).unwrap();
        let loaded = Address::load(&mut contract, slot).unwrap();
        assert_eq!(addr, loaded);
    }

    #[test]
    fn test_bool_conversions() {
        let mut contract = setup_test_contract();
        let slot = U256::from(3);

        // Test true
        true.store(&mut contract, slot).unwrap();
        assert!(bool::load(&mut contract, slot).unwrap());

        // Test false
        false.store(&mut contract, slot).unwrap();
        assert!(!bool::load(&mut contract, slot).unwrap());

        // Test that any non-zero value is true
        contract.sstore(slot, U256::from(42)).unwrap();
        assert!(bool::load(&mut contract, slot).unwrap());
    }

    // -- STRING + BYTES TESTS -------------------------------------------------

    // Strategy for generating random U256 slot values that won't overflow
    fn arb_safe_slot() -> impl Strategy<Value = U256> {
        any::<[u64; 4]>().prop_map(|limbs| {
            // Ensure we don't overflow by limiting to a reasonable range
            U256::from_limbs(limbs) % (U256::MAX - U256::from(10000))
        })
    }

    // Strategy for short strings (0-31 bytes) - uses inline storage
    fn arb_short_string() -> impl Strategy<Value = String> {
        prop_oneof![
            // Empty string
            Just(String::new()),
            // ASCII strings (1-31 bytes)
            "[a-zA-Z0-9]{1,31}",
            // Unicode strings (up to 31 bytes)
            "[\u{0041}-\u{005A}\u{4E00}-\u{4E19}]{1,10}",
        ]
    }

    // Strategy for exactly 32-byte strings - boundary between inline and heap storage
    fn arb_32byte_string() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9]{32}"
    }

    // Strategy for long strings (33-100 bytes) - uses heap storage
    fn arb_long_string() -> impl Strategy<Value = String> {
        prop_oneof![
            // ASCII strings (33-100 bytes)
            "[a-zA-Z0-9]{33,100}",
            // Unicode strings (>32 bytes)
            "[\u{0041}-\u{005A}\u{4E00}-\u{4E19}]{11,30}",
        ]
    }

    // Strategy for short byte arrays (0-31 bytes) - uses inline storage
    fn arb_short_bytes() -> impl Strategy<Value = Bytes> {
        prop::collection::vec(any::<u8>(), 0..=31).prop_map(|v| Bytes::from(v))
    }

    // Strategy for exactly 32-byte arrays - boundary between inline and heap storage
    fn arb_32byte_bytes() -> impl Strategy<Value = Bytes> {
        prop::collection::vec(any::<u8>(), 32..=32).prop_map(|v| Bytes::from(v))
    }

    // Strategy for long byte arrays (33-100 bytes) - uses heap storage
    fn arb_long_bytes() -> impl Strategy<Value = Bytes> {
        prop::collection::vec(any::<u8>(), 33..=100).prop_map(|v| Bytes::from(v))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]

        #[test]
        fn test_short_strings(s in arb_short_string(), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();

            // Verify store → load roundtrip
            s.store(&mut contract, base_slot)?;
            let loaded = String::load(&mut contract, base_slot)?;
            assert_eq!(s, loaded, "Short string roundtrip failed for: {:?}", s);
        }

        #[test]
        fn test_32byte_strings(s in arb_32byte_string(), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();

            // Verify 32-byte boundary string is stored correctly
            assert_eq!(s.len(), 32, "Generated string should be exactly 32 bytes");

            // Verify store → load roundtrip
            s.store(&mut contract, base_slot)?;
            let loaded = String::load(&mut contract, base_slot)?;
            assert_eq!(s, loaded, "32-byte string roundtrip failed");
        }

        #[test]
        fn test_long_strings(s in arb_long_string(), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();

            // Verify store → load roundtrip
            s.store(&mut contract, base_slot)?;
            let loaded = String::load(&mut contract, base_slot)?;
            assert_eq!(s, loaded, "Long string roundtrip failed for length: {}", s.len());
        }

        #[test]
        fn test_short_bytes(b in arb_short_bytes(), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();

            // Verify store → load roundtrip
            b.store(&mut contract, base_slot)?;
            let loaded = Bytes::load(&mut contract, base_slot)?;
            assert_eq!(b, loaded, "Short bytes roundtrip failed for length: {}", b.len());
        }

        #[test]
        fn test_32byte_bytes(b in arb_32byte_bytes(), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();

            // Verify 32-byte boundary bytes is stored correctly
            assert_eq!(b.len(), 32, "Generated bytes should be exactly 32 bytes");

            // Verify store → load roundtrip
            b.store(&mut contract, base_slot)?;
            let loaded = Bytes::load(&mut contract, base_slot)?;
            assert_eq!(b, loaded, "32-byte bytes roundtrip failed");
        }

        #[test]
        fn test_long_bytes(b in arb_long_bytes(), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();

            // Verify store → load roundtrip
            b.store(&mut contract, base_slot)?;
            let loaded = Bytes::load(&mut contract, base_slot)?;
            assert_eq!(b, loaded, "Long bytes roundtrip failed for length: {}", b.len());
        }
    }
}
