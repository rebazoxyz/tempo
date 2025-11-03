use alloy::primitives::U256;
use std::marker::PhantomData;

use crate::{
    error::Result,
    storage::{
        Storable, StorageKey, StorageOps,
        slots::{double_mapping_slot, mapping_slot},
    },
};

/// A zero-sized marker type representing a storage mapping.
///
/// `Mapping<K, V, SLOT>` is a compile-time abstraction representing Solidity's `mapping(K => V)`.
/// The base slot number is encoded in the type as a const generic, providing compile-time safety.
/// The slot is represented as `[u64; 4]` (4 limbs) to support the full U256 range.
///
/// The actual storage operations compute the storage location following Solidity's
/// hash-based slot encoding spec.
///
/// # Mapping
///
/// ```ignore
/// struct MyStorage {
///     balances: Mapping<Address, U256, [10, 0, 0, 0]>,  // Base slot 10
/// }
/// ```
///
/// # Nested Mapping
///
/// ```ignore
/// struct MyStorage {
///     allowances: Mapping<Address, Mapping<Address, U256>, [11, 0, 0, 0]>,
/// }
/// ```
/// Note: For nested mappings, only the outermost mapping needs a slot specified.
#[derive(Debug, Clone, Copy)]
pub struct Mapping<K, V, const SLOT: [u64; 4]> {
    _phantom: PhantomData<(K, V)>,
}

impl<K, V, const SLOT: [u64; 4]> Mapping<K, V, SLOT> {
    /// Creates a new `Mapping` marker.
    ///
    /// This is typically not called directly; instead, mappings are declared
    /// as struct fields and accessed via macro-generated methods.
    #[inline]
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Returns the U256 base storage slot number for this mapping.
    ///
    /// Converts the const generic `[u64; 4]` limbs to a U256.
    #[inline]
    pub const fn slot() -> U256 {
        U256::from_limbs(SLOT)
    }

    /// Reads a value from the mapping at the given key.
    ///
    /// This method:
    /// 1. Computes the storage slot via keccak256(key || base_slot)
    /// 2. Delegates to `Storable::load`, which reads `N` consecutive slots
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NamedMapping = Mapping<Address, U256, { [10, 0, 0, 0] }>;
    /// let name = NamedMapping::read(&mut contract, user_address)?;
    /// ```
    #[inline]
    pub fn read<S: StorageOps, const N: usize>(storage: &mut S, key: K) -> Result<V>
    where
        K: StorageKey,
        V: Storable<N>,
    {
        let slot = mapping_slot(key.as_storage_bytes(), Self::slot());
        V::load(storage, slot)
    }

    /// Writes a value to the mapping at the given key.
    ///
    /// This method:
    /// 1. Computes the storage slot via keccak256(key || base_slot)
    /// 2. Delegates to `Storable::store`, which writes to `N` consecutive slots
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NamedMapping = Mapping<Address, U256, { [10, 0, 0, 0] }>;
    /// NamedMapping::write(&mut contract, user_address, U256::from(100))?;
    /// ```
    #[inline]
    pub fn write<S: StorageOps, const N: usize>(storage: &mut S, key: K, value: V) -> Result<()>
    where
        K: StorageKey,
        V: Storable<N>,
    {
        let slot = mapping_slot(key.as_storage_bytes(), Self::slot());
        value.store(storage, slot)
    }

    /// Deletes the value from the mapping at the given key (sets all slots to zero).
    ///
    /// This method:
    /// 1. Computes the storage slot via keccak256(key || base_slot)
    /// 2. Delegates to `Storable::delete`, which sets `N` consecutive slots to zero
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NamedMapping = Mapping<Address, U256, { [10, 0, 0, 0] }>;
    /// NamedMapping::delete(&mut contract, user_address)?;
    /// ```
    #[inline]
    pub fn delete<S: StorageOps, const N: usize>(storage: &mut S, key: K) -> Result<()>
    where
        K: StorageKey,
        V: Storable<N>,
    {
        let slot = mapping_slot(key.as_storage_bytes(), Self::slot());
        V::delete(storage, slot)
    }
}

impl<K1, K2, V, const SLOT: [u64; 4], const DUMMY: [u64; 4]>
    Mapping<K1, Mapping<K2, V, DUMMY>, SLOT>
{
    /// Reads a value from a nested mapping at the given keys.
    ///
    /// This method:
    /// 1. Computes the storage slot using: `keccak256(k2 || keccak256(k1 || base_slot))`
    /// 2. Delegates to `Storable::load`, which may read one or more consecutive slots
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NestedMapping = Mapping<Address, Mapping<Address, U256, { [0, 0, 0, 0] }>, { [11, 0, 0, 0] }>;
    /// let nested = NestedMapping::read_nested(
    ///     &mut contract,
    ///     owner_address,
    ///     spender_address
    /// )?;
    /// ```
    #[inline]
    pub fn read_nested<S: StorageOps, const N: usize>(
        storage: &mut S,
        key1: K1,
        key2: K2,
    ) -> Result<V>
    where
        K1: StorageKey,
        K2: StorageKey,
        V: Storable<N>,
    {
        let slot = double_mapping_slot(
            key1.as_storage_bytes(),
            key2.as_storage_bytes(),
            Self::slot(),
        );
        V::load(storage, slot)
    }

    /// Writes a value to a nested mapping at the given keys.
    ///
    /// This method:
    /// 1. Computes the storage slot using: `keccak256(k2 || keccak256(k1 || base_slot))`
    /// 2. Delegates to `Storable::store`, which may write one or more consecutive slots
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NestedMapping = Mapping<Address, Mapping<Address, U256, { [0, 0, 0, 0] }>, { [11, 0, 0, 0] }>;
    /// NestedMapping::write_nested(
    ///     &mut contract,
    ///     owner_address,
    ///     spender_address,
    ///     U256::from(1000)
    /// )?;
    /// ```
    #[inline]
    pub fn write_nested<S: StorageOps, const N: usize>(
        storage: &mut S,
        key1: K1,
        key2: K2,
        value: V,
    ) -> Result<()>
    where
        K1: StorageKey,
        K2: StorageKey,
        V: Storable<N>,
    {
        let slot = double_mapping_slot(
            key1.as_storage_bytes(),
            key2.as_storage_bytes(),
            Self::slot(),
        );
        value.store(storage, slot)
    }

    /// Deletes a value from a nested mapping at the given keys (sets all slots to zero).
    ///
    /// This method:
    /// 1. Computes the storage slot using: `keccak256(k2 || keccak256(k1 || base_slot))`
    /// 2. Delegates to `Storable::delete`, which sets `N` consecutive slots to zero
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NestedMapping = Mapping<Address, Mapping<Address, U256, { [0, 0, 0, 0] }>, { [11, 0, 0, 0] }>;
    /// NestedMapping::delete_nested(
    ///     &mut contract,
    ///     owner_address,
    ///     spender_address
    /// )?;
    /// ```
    #[inline]
    pub fn delete_nested<S: StorageOps, const N: usize>(
        storage: &mut S,
        key1: K1,
        key2: K2,
    ) -> Result<()>
    where
        K1: StorageKey,
        K2: StorageKey,
        V: Storable<N>,
    {
        let slot = double_mapping_slot(
            key1.as_storage_bytes(),
            key2.as_storage_bytes(),
            Self::slot(),
        );
        V::delete(storage, slot)
    }
}

impl<K, V, const SLOT: [u64; 4]> Default for Mapping<K, V, SLOT> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PrecompileStorageProvider, hashmap::HashMapStorageProvider};
    use alloy::primitives::Address;

    // Test helper that implements StorageOps
    struct TestContract<'a, S> {
        address: Address,
        storage: &'a mut S,
    }

    impl<'a, S: PrecompileStorageProvider> StorageOps for TestContract<'a, S> {
        fn sstore(&mut self, slot: U256, value: U256) -> Result<()> {
            self.storage.sstore(self.address, slot, value)
        }

        fn sload(&mut self, slot: U256) -> Result<U256> {
            self.storage.sload(self.address, slot)
        }
    }

    #[test]
    fn test_mapping_is_zero_sized() {
        assert_eq!(
            std::mem::size_of::<Mapping<Address, U256, { [10, 0, 0, 0] }>>(),
            0
        );
        assert_eq!(
            std::mem::size_of::<Mapping<U256, Address, { [11, 0, 0, 0] }>>(),
            0
        );
        // Nested mapping (only outer mapping has slot)
        type NestedMapping = Mapping<Address, U256, { [12, 0, 0, 0] }>;
        assert_eq!(std::mem::size_of::<NestedMapping>(), 0);
    }

    #[test]
    fn test_mapping_creation() {
        let _simple: Mapping<Address, U256, { [10, 0, 0, 0] }> = Mapping::new();
        let _another: Mapping<U256, bool, { [11, 0, 0, 0] }> = Mapping::default();
    }

    #[test]
    fn test_mapping_slot_extraction() {
        assert_eq!(
            Mapping::<Address, U256, { [10, 0, 0, 0] }>::slot(),
            U256::from(10)
        );
        assert_eq!(
            Mapping::<U256, Address, { [11, 0, 0, 0] }>::slot(),
            U256::from(11)
        );

        // Test with larger slot number
        assert_eq!(
            Mapping::<Address, U256, { [100, 0, 0, 0] }>::slot(),
            U256::from(100)
        );
    }

    #[test]
    fn test_mapping_full_u256_slot() {
        // Test full U256 range support
        const MAX_SLOT: [u64; 4] = [u64::MAX, u64::MAX, u64::MAX, u64::MAX];
        type MaxMapping = Mapping<Address, U256, MAX_SLOT>;
        assert_eq!(MaxMapping::slot(), U256::MAX);
    }

    #[test]
    fn test_mapping_read_write_balances() {
        let mut storage = HashMapStorageProvider::new(1);
        let token_addr = Address::random();
        let mut contract = TestContract {
            address: token_addr,
            storage: &mut storage,
        };
        let user1 = Address::random();
        let user2 = Address::random();

        type NamedMapping = Mapping<Address, U256, { [10, 0, 0, 0] }>;

        let balance1 = U256::from(1000u64);
        let balance2 = U256::from(2000u64);

        // Write balances
        _ = NamedMapping::write(&mut contract, user1, balance1);
        _ = NamedMapping::write(&mut contract, user2, balance2);

        // Read balances
        let loaded1 = NamedMapping::read(&mut contract, user1).unwrap();
        let loaded2 = NamedMapping::read(&mut contract, user2).unwrap();

        assert_eq!(loaded1, balance1);
        assert_eq!(loaded2, balance2);
    }

    #[test]
    fn test_mapping_read_default_is_zero() {
        let mut storage = HashMapStorageProvider::new(1);
        let token_addr = Address::random();
        let mut contract = TestContract {
            address: token_addr,
            storage: &mut storage,
        };
        let user = Address::random();

        type NamedMapping = Mapping<Address, U256, { [10, 0, 0, 0] }>;

        // Reading uninitialized mapping slot should return zero
        let balance = NamedMapping::read(&mut contract, user).unwrap();
        assert_eq!(balance, U256::ZERO);
    }

    #[test]
    fn test_mapping_overwrite() {
        let mut storage = HashMapStorageProvider::new(1);
        let token_addr = Address::random();
        let mut contract = TestContract {
            address: token_addr,
            storage: &mut storage,
        };
        let user = Address::random();

        type NamedMapping = Mapping<Address, U256, { [10, 0, 0, 0] }>;

        // Write initial balance
        _ = NamedMapping::write(&mut contract, user, U256::from(100));
        assert_eq!(NamedMapping::read(&mut contract, user), Ok(U256::from(100)));

        // Overwrite with new balance
        _ = NamedMapping::write(&mut contract, user, U256::from(200));
        assert_eq!(NamedMapping::read(&mut contract, user), Ok(U256::from(200)));
    }

    #[test]
    fn test_nested_mapping_read_write_allowances() {
        let mut storage = HashMapStorageProvider::new(1);
        let token_addr = Address::random();
        let mut contract = TestContract {
            address: token_addr,
            storage: &mut storage,
        };
        let owner = Address::random();
        let spender1 = Address::random();
        let spender2 = Address::random();

        // Nested mapping: outer slot is 11, inner slot is dummy (unused)
        type NestedMapping =
            Mapping<Address, Mapping<Address, U256, { [0, 0, 0, 0] }>, { [11, 0, 0, 0] }>;

        let allowance1 = U256::from(500u64);
        let allowance2 = U256::from(1500u64);

        // Write allowances using nested API
        _ = NestedMapping::write_nested(&mut contract, owner, spender1, allowance1);
        _ = NestedMapping::write_nested(&mut contract, owner, spender2, allowance2);

        // Read allowances using nested API
        let loaded1 = NestedMapping::read_nested(&mut contract, owner, spender1).unwrap();
        let loaded2 = NestedMapping::read_nested(&mut contract, owner, spender2).unwrap();

        assert_eq!(loaded1, allowance1);
        assert_eq!(loaded2, allowance2);
    }

    #[test]
    fn test_nested_mapping_default_is_zero() {
        let mut storage = HashMapStorageProvider::new(1);
        let token_addr = Address::random();
        let mut contract = TestContract {
            address: token_addr,
            storage: &mut storage,
        };
        let owner = Address::random();
        let spender = Address::random();

        type NestedMapping =
            Mapping<Address, Mapping<Address, U256, { [0, 0, 0, 0] }>, { [11, 0, 0, 0] }>;

        // Reading uninitialized nested mapping should return zero
        let allowance = NestedMapping::read_nested(&mut contract, owner, spender).unwrap();
        assert_eq!(allowance, U256::ZERO);
    }

    #[test]
    fn test_nested_mapping_independence() {
        let mut storage = HashMapStorageProvider::new(1);
        let token_addr = Address::random();
        let mut contract = TestContract {
            address: token_addr,
            storage: &mut storage,
        };
        let owner1 = Address::random();
        let owner2 = Address::random();
        let spender = Address::random();

        type NestedMapping =
            Mapping<Address, Mapping<Address, U256, { [0, 0, 0, 0] }>, { [11, 0, 0, 0] }>;

        // Set allowance for owner1 -> spender
        _ = NestedMapping::write_nested(&mut contract, owner1, spender, U256::from(100));

        // Verify owner2 -> spender is still zero (independent slot)
        let allowance2 = NestedMapping::read_nested(&mut contract, owner2, spender).unwrap();
        assert_eq!(allowance2, U256::ZERO);

        // Verify owner1 -> spender is unchanged
        let allowance1 = NestedMapping::read_nested(&mut contract, owner1, spender).unwrap();
        assert_eq!(allowance1, U256::from(100));
    }

    #[test]
    fn test_mapping_with_different_key_types() {
        let mut storage = HashMapStorageProvider::new(1);
        let contract_addr = Address::random();
        let mut contract = TestContract {
            address: contract_addr,
            storage: &mut storage,
        };

        // Mapping with U256 key
        type NoncesMapping = Mapping<Address, U256, { [12, 0, 0, 0] }>;
        let user = Address::random();
        let nonce = U256::from(42);

        _ = NoncesMapping::write(&mut contract, user, nonce);
        let loaded_nonce = NoncesMapping::read(&mut contract, user).unwrap();
        assert_eq!(loaded_nonce, nonce);

        // Mapping with bool value
        type FlagsMapping = Mapping<Address, bool, { [13, 0, 0, 0] }>;
        _ = FlagsMapping::write(&mut contract, user, true);
        let loaded_flag = FlagsMapping::read(&mut contract, user).unwrap();
        assert!(loaded_flag);
    }
}
