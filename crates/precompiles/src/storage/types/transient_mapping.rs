//! Type-safe wrapper for EVM transient storage mappings (hash-based key-value transient storage).
//!
//! Transient storage is automatically cleared at the end of each transaction,
//! making it perfect for transaction-scoped data that doesn't need to persist.

use alloy::primitives::{U256, keccak256};
use std::marker::PhantomData;

use crate::{
    error::Result,
    storage::{PrecompileStorageProvider, Storable, StorableType, StorageKey, types::slot::SlotId},
};

/// Type-safe wrapper for EVM transient storage mappings.
///
/// Transient storage provides gas-efficient temporary storage that:
/// - Automatically clears at transaction end
/// - Costs significantly less gas than persistent storage
/// - Perfect for transaction-scoped data
///
/// # Type Parameters
///
/// - `K`: Key type (must implement `StorageKey`)
/// - `V`: Value type (must implement `Storable<N>`)
/// - `Base`: Zero-sized marker type identifying the base slot (implements `SlotId`)
///
/// # Storage Layout
///
/// Uses the same layout as persistent mappings:
/// - Base slot: `Base::SLOT`
/// - Actual slot for key `k`: `keccak256(k || base_slot)`
///
/// # Gas Costs (Cancun+)
///
/// - TLOAD: 100 gas (vs SLOAD: 2100-2600 gas)
/// - TSTORE: 100 gas (vs SSTORE: 2900-20000 gas)
#[derive(Debug, Clone, Copy)]
pub struct TransientMapping<K, V, Base: SlotId> {
    _phantom: PhantomData<(K, V, Base)>,
}

impl<K, V, Base: SlotId> TransientMapping<K, V, Base> {
    /// Creates a new `TransientMapping` marker.
    #[inline]
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Returns the U256 base storage slot number for this mapping.
    #[inline]
    pub const fn slot() -> U256 {
        Base::SLOT
    }

    /// Reads a value from the transient mapping at the given key.
    ///
    /// This method:
    /// 1. Computes the storage slot via keccak256(key || base_slot)
    /// 2. Uses TLOAD to read from transient storage
    /// 3. Delegates to `Storable::from_evm_words` for decoding
    ///
    /// # Example
    ///
    /// ```ignore
    /// type TxKeyMapping = TransientMapping<Address, Address, TxKeySlotId>;
    /// let key_id = TxKeyMapping::read(&mut storage, account)?;
    /// ```
    #[inline]
    pub fn read<S: PrecompileStorageProvider, const N: usize>(
        storage: &mut S,
        address: alloy::primitives::Address,
        key: K,
    ) -> Result<V>
    where
        K: StorageKey,
        V: Storable<N>,
    {
        let slot = mapping_slot(key.as_storage_bytes(), Base::SLOT);

        // For multi-slot values, read N consecutive slots
        let mut words = [U256::ZERO; N];
        for i in 0..N {
            words[i] = storage.tload(address, slot + U256::from(i))?;
        }

        V::from_evm_words(words)
    }

    /// Writes a value to the transient mapping at the given key.
    ///
    /// This method:
    /// 1. Computes the storage slot via keccak256(key || base_slot)
    /// 2. Uses TSTORE to write to transient storage
    /// 3. Delegates to `Storable::to_evm_words` for encoding
    ///
    /// # Example
    ///
    /// ```ignore
    /// type TxKeyMapping = TransientMapping<Address, Address, TxKeySlotId>;
    /// TxKeyMapping::write(&mut storage, account, key_id)?;
    /// ```
    #[inline]
    pub fn write<S: PrecompileStorageProvider, const N: usize>(
        storage: &mut S,
        address: alloy::primitives::Address,
        key: K,
        value: V,
    ) -> Result<()>
    where
        K: StorageKey,
        V: Storable<N>,
    {
        let slot = mapping_slot(key.as_storage_bytes(), Base::SLOT);
        let words = value.to_evm_words()?;

        // For multi-slot values, write N consecutive slots
        for i in 0..N {
            storage.tstore(address, slot + U256::from(i), words[i])?;
        }

        Ok(())
    }

    /// Deletes the value from the transient mapping at the given key.
    ///
    /// Note: This is typically unnecessary since transient storage
    /// automatically clears at transaction end, but provided for completeness.
    ///
    /// # Example
    ///
    /// ```ignore
    /// type TxKeyMapping = TransientMapping<Address, Address, TxKeySlotId>;
    /// TxKeyMapping::delete(&mut storage, account)?;
    /// ```
    #[inline]
    pub fn delete<S: PrecompileStorageProvider, const N: usize>(
        storage: &mut S,
        address: alloy::primitives::Address,
        key: K,
    ) -> Result<()>
    where
        K: StorageKey,
        V: Storable<N>,
    {
        let slot = mapping_slot(key.as_storage_bytes(), Base::SLOT);

        // Clear N consecutive slots
        for i in 0..N {
            storage.tstore(address, slot + U256::from(i), U256::ZERO)?;
        }

        Ok(())
    }
}

impl<K, V, Base: SlotId> Default for TransientMapping<K, V, Base> {
    fn default() -> Self {
        Self::new()
    }
}

// TransientMappings occupy a full 32-byte slot in the layout (used as a base for hashing),
// even though they don't store data in that slot directly.
impl<K, V, Base: SlotId> StorableType for TransientMapping<K, V, Base> {
    const BYTE_COUNT: usize = 32;
}

// -- HELPER FUNCTIONS (reused from mapping.rs) ---------------------------------

fn left_pad_to_32(data: &[u8]) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[32 - data.len()..].copy_from_slice(data);
    buf
}

/// Compute storage slot for a transient mapping (same as persistent mapping)
/// Uses the same hash calculation as persistent mappings for consistency.
#[inline]
fn mapping_slot<T: AsRef<[u8]>>(key: T, mapping_slot: U256) -> U256 {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&left_pad_to_32(key.as_ref()));
    buf[32..].copy_from_slice(&mapping_slot.to_be_bytes::<32>());
    U256::from_be_bytes(keccak256(buf).0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::hashmap::HashMapStorageProvider;
    use alloy::primitives::{Address, address};

    // Test SlotId implementations
    struct TestSlot0;
    impl SlotId for TestSlot0 {
        const SLOT: U256 = U256::ZERO;
    }

    struct TestSlot1;
    impl SlotId for TestSlot1 {
        const SLOT: U256 = U256::ONE;
    }

    #[test]
    fn test_transient_mapping_creation() {
        let _mapping: TransientMapping<Address, U256, TestSlot1> = TransientMapping::new();
        let _another: TransientMapping<U256, Address, TestSlot0> = TransientMapping::default();
    }

    #[test]
    fn test_transient_mapping_slot_extraction() {
        assert_eq!(
            TransientMapping::<Address, U256, TestSlot0>::slot(),
            U256::ZERO
        );
        assert_eq!(
            TransientMapping::<Address, U256, TestSlot1>::slot(),
            U256::ONE
        );
    }

    #[test]
    fn test_transient_mapping_is_zero_sized() {
        assert_eq!(
            std::mem::size_of::<TransientMapping<Address, U256, TestSlot0>>(),
            0
        );
        assert_eq!(
            std::mem::size_of::<TransientMapping<U256, Address, TestSlot1>>(),
            0
        );
    }

    #[test]
    fn test_transient_mapping_read_write() {
        let mut storage = HashMapStorageProvider::new(1);
        let contract_address = address!("1000000000000000000000000000000000000001");
        let user = Address::random();
        let value = U256::from(42);

        type TMapping = TransientMapping<Address, U256, TestSlot1>;

        // Write value
        TMapping::write(&mut storage, contract_address, user, value).unwrap();

        // Read value back
        let loaded = TMapping::read(&mut storage, contract_address, user).unwrap();
        assert_eq!(loaded, value);
    }

    #[test]
    fn test_transient_mapping_delete() {
        let mut storage = HashMapStorageProvider::new(1);
        let contract_address = address!("2000000000000000000000000000000000000002");
        let user = Address::random();
        let value = U256::from(999);

        type TMapping = TransientMapping<Address, U256, TestSlot0>;

        // Write value
        TMapping::write(&mut storage, contract_address, user, value).unwrap();

        // Delete value
        TMapping::delete(&mut storage, contract_address, user).unwrap();

        // Should read zero after delete
        let loaded = TMapping::read(&mut storage, contract_address, user).unwrap();
        assert_eq!(loaded, U256::ZERO);
    }

    #[test]
    fn test_transient_mapping_isolation() {
        let mut storage = HashMapStorageProvider::new(1);
        let contract_address = address!("3000000000000000000000000000000000000003");
        let user1 = Address::random();
        let user2 = Address::random();
        let value1 = U256::from(100);
        let value2 = U256::from(200);

        type TMapping = TransientMapping<Address, U256, TestSlot1>;

        // Write different values for different keys
        TMapping::write(&mut storage, contract_address, user1, value1).unwrap();
        TMapping::write(&mut storage, contract_address, user2, value2).unwrap();

        // Verify isolation
        assert_eq!(
            TMapping::read(&mut storage, contract_address, user1).unwrap(),
            value1
        );
        assert_eq!(
            TMapping::read(&mut storage, contract_address, user2).unwrap(),
            value2
        );
    }
}
