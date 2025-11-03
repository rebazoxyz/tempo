use alloy::primitives::U256;
use std::marker::PhantomData;

use crate::{
    error::Result,
    storage::{Storable, StorageOps},
};

/// A zero-sized marker type representing an 32-bytes storage slot.
///
/// `Slot<T, SLOT>` is a compile-time abstraction that marks a field as occupying
/// a single EVM storage slot at a specific location. The slot number is encoded
/// in the type as a const generic, providing compile-time safety.
///
/// The slot is represented as `[u64; 4]` (4 limbs) to support the full U256 range.
///
/// # Examples
///
/// ```ignore
/// // Typically used via macro-generated code
/// struct MyStorage {
///     counter: Slot<U256, [0, 0, 0, 0]>,      // Slot 0
///     owner: Slot<Address, [1, 0, 0, 0]>,     // Slot 1
///     paused: Slot<bool, [10, 0, 0, 0]>,      // Slot 10
/// }
/// ```
///
/// The actual storage operations are handled by generated accessor methods
/// that read/write values using the `PrecompileStorageProvider` trait.
#[derive(Debug, Clone, Copy)]
pub struct Slot<T, const SLOT: [u64; 4]> {
    _phantom: PhantomData<T>,
}

impl<T, const SLOT: [u64; 4]> Slot<T, SLOT> {
    /// Creates a new `Slot` marker.
    ///
    /// This is typically not called directly; instead, slots are declared
    /// as struct fields and accessed via macro-generated methods.
    #[inline]
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Returns the U256 storage slot number.
    ///
    /// Converts the const generic `[u64; 4]` limbs to a U256.
    #[inline]
    pub const fn slot() -> U256 {
        U256::from_limbs(SLOT)
    }

    /// Reads a value from storage at this slot.
    ///
    /// This method delegates to the `Storable::load` implementation,
    /// which may read one or more consecutive slots depending on `N`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NamedSlot = Slot<String, { [0, 0, 0, 0] }>;
    /// let name = NamedSlot::read(&mut contract)?;
    /// ```
    #[inline]
    pub fn read<S: StorageOps, const N: usize>(storage: &mut S) -> Result<T>
    where
        T: Storable<N>,
    {
        T::load(storage, Self::slot())
    }

    /// Writes a value to storage at this slot.
    ///
    /// This method delegates to the `Storable::store` implementation,
    /// which may write one or more consecutive slots depending on `N`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NamedSlot = Slot<String, { [0, 0, 0, 0] }>;
    /// NamedSlot::write(&mut contract, "MyToken".to_string())?;
    /// ```
    #[inline]
    pub fn write<S: StorageOps, const N: usize>(storage: &mut S, value: T) -> Result<()>
    where
        T: Storable<N>,
    {
        value.store(storage, Self::slot())
    }

    /// Deletes the value at this slot (sets all slots to zero).
    ///
    /// This method delegates to the `Storable::delete` implementation,
    /// which sets `N` consecutive slots to zero.
    ///
    /// # Example
    ///
    /// ```ignore
    /// type NamedSlot = Slot<String, { [0, 0, 0, 0] }>;
    /// NamedSlot::delete(&mut contract)?;
    /// ```
    #[inline]
    pub fn delete<S: StorageOps, const N: usize>(storage: &mut S) -> Result<()>
    where
        T: Storable<N>,
    {
        T::delete(storage, Self::slot())
    }
}

impl<T, const SLOT: [u64; 4]> Default for Slot<T, SLOT> {
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
    fn test_slot_is_zero_sized() {
        assert_eq!(std::mem::size_of::<Slot<U256, { [0, 0, 0, 0] }>>(), 0);
        assert_eq!(std::mem::size_of::<Slot<Address, { [1, 0, 0, 0] }>>(), 0);
        assert_eq!(std::mem::size_of::<Slot<bool, { [10, 0, 0, 0] }>>(), 0);
    }

    #[test]
    fn test_slot_creation() {
        let _slot_u256: Slot<U256, { [0, 0, 0, 0] }> = Slot::new();
        let _slot_addr: Slot<Address, { [1, 0, 0, 0] }> = Slot::new();
        let _slot_default: Slot<bool, { [2, 0, 0, 0] }> = Slot::default();
    }

    #[test]
    fn test_slot_number_extraction() {
        assert_eq!(Slot::<U256, { [0, 0, 0, 0] }>::slot(), U256::ZERO);
        assert_eq!(Slot::<Address, { [1, 0, 0, 0] }>::slot(), U256::from(1));
        assert_eq!(Slot::<bool, { [10, 0, 0, 0] }>::slot(), U256::from(10));

        // Test with larger slot number
        assert_eq!(Slot::<U256, { [123, 0, 0, 0] }>::slot(), U256::from(123));
    }

    #[test]
    fn test_full_u256_slot() {
        // Test full U256 range support
        const MAX_SLOT: [u64; 4] = [u64::MAX, u64::MAX, u64::MAX, u64::MAX];
        type MaxSlot = Slot<U256, MAX_SLOT>;
        assert_eq!(MaxSlot::slot(), U256::MAX);
    }

    #[test]
    fn test_slot_read_write_u256() {
        let mut storage = HashMapStorageProvider::new(1);
        let addr = Address::random();
        let mut contract = TestContract {
            address: addr,
            storage: &mut storage,
        };

        type TestSlot = Slot<U256, { [42, 0, 0, 0] }>;
        let test_value = U256::from(12345u64);

        // Write using new API
        _ = TestSlot::write(&mut contract, test_value);

        // Read using new API
        let loaded = TestSlot::read(&mut contract).unwrap();
        assert_eq!(loaded, test_value);

        // Verify it actually wrote to slot 42
        let raw = contract.storage.sload(addr, U256::from(42));
        assert_eq!(raw, Ok(test_value));
    }

    #[test]
    fn test_slot_read_write_address() {
        let mut storage = HashMapStorageProvider::new(1);
        let contract_addr = Address::random();
        let mut contract = TestContract {
            address: contract_addr,
            storage: &mut storage,
        };
        let test_addr = Address::random();

        type OwnerSlot = Slot<Address, { [1, 0, 0, 0] }>;

        // Write
        _ = OwnerSlot::write(&mut contract, test_addr);

        // Read
        let loaded = OwnerSlot::read(&mut contract).unwrap();
        assert_eq!(loaded, test_addr);
    }

    #[test]
    fn test_slot_read_write_bool() {
        let mut storage = HashMapStorageProvider::new(1);
        let addr = Address::random();
        let mut contract = TestContract {
            address: addr,
            storage: &mut storage,
        };

        type PausedSlot = Slot<bool, { [8, 0, 0, 0] }>;

        // Write true
        _ = PausedSlot::write(&mut contract, true);
        assert!(PausedSlot::read(&mut contract).unwrap());

        // Write false
        _ = PausedSlot::write(&mut contract, false);
        assert!(!PausedSlot::read(&mut contract).unwrap());
    }

    #[test]
    fn test_slot_read_write_string() {
        let mut storage = HashMapStorageProvider::new(1);
        let addr = Address::random();
        let mut contract = TestContract {
            address: addr,
            storage: &mut storage,
        };

        type NamedSlot = Slot<String, { [0, 0, 0, 0] }>;

        let test_name = "TestToken";
        _ = NamedSlot::write(&mut contract, test_name.to_string());

        let loaded = NamedSlot::read(&mut contract).unwrap();
        assert_eq!(loaded, test_name);
    }

    #[test]
    fn test_slot_default_value_is_zero() {
        let mut storage = HashMapStorageProvider::new(1);
        let addr = Address::random();
        let mut contract = TestContract {
            address: addr,
            storage: &mut storage,
        };

        type UninitializedSlot = Slot<U256, { [99, 0, 0, 0] }>;

        // Reading uninitialized storage should return zero
        let value = UninitializedSlot::read(&mut contract).unwrap();
        assert_eq!(value, U256::ZERO);
    }

    #[test]
    fn test_slot_overwrite() {
        let mut storage = HashMapStorageProvider::new(1);
        let addr = Address::random();
        let mut contract = TestContract {
            address: addr,
            storage: &mut storage,
        };

        type CounterSlot = Slot<u64, { [5, 0, 0, 0] }>;

        // Write initial value
        _ = CounterSlot::write(&mut contract, 100);
        assert_eq!(CounterSlot::read(&mut contract), Ok(100));

        // Overwrite with new value
        _ = CounterSlot::write(&mut contract, 200);
        assert_eq!(CounterSlot::read(&mut contract), Ok(200));
    }
}
