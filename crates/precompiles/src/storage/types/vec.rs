//! Dynamic array (`Vec<T>`) implementation for the `Storable` trait.
//!
//! # Storage Layout
//!
//! Vec uses Solidity-compatible dynamic array storage:
//! - **Base slot**: Stores the array length (number of elements)
//! - **Data slots**: Start at `keccak256(base_slot)`, elements packed efficiently

use alloy::primitives::U256;

use crate::{
    error::{Result, TempoPrecompileError},
    storage::{
        Storable, StorableType, StorageOps,
        packing::{calc_packed_slot_count, extract_packed_value, insert_packed_value},
    },
};

/// Calculate the starting slot for dynamic array data.
///
/// For Solidity compatibility, dynamic array data is stored at `keccak256(base_slot)`.
#[inline]
fn calc_data_slot(base_slot: U256) -> U256 {
    U256::from_be_bytes(alloy::primitives::keccak256(base_slot.to_be_bytes::<32>()).0)
}

impl<T: StorableType> StorableType for Vec<T> {
    /// Vec base slot is always 32 bytes (stores length).
    const BYTE_COUNT: usize = 32;
}

impl<T> Storable<1> for Vec<T>
where
    T: Storable<1> + StorableType,
{
    const SLOT_COUNT: usize = 1;

    fn load<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<Self> {
        // Read length from base slot
        let length_value = storage.sload(base_slot)?;
        let length = length_value.to::<usize>();

        if length == 0 {
            return Ok(Vec::new());
        }

        let data_start = calc_data_slot(base_slot);

        // Determine if elements should be packed
        let byte_count = T::BYTE_COUNT;
        if byte_count < 32 && 32 % byte_count == 0 {
            // Elements can be packed multiple per slot
            load_packed_elements(storage, data_start, length, byte_count)
        } else {
            // Elements use full slots (either 32 bytes or multi-slot)
            load_unpacked_elements(storage, data_start, length)
        }
    }

    fn store<S: StorageOps>(&self, storage: &mut S, base_slot: U256) -> Result<()> {
        // Write length to base slot
        storage.sstore(base_slot, U256::from(self.len()))?;

        if self.is_empty() {
            return Ok(());
        }

        let data_start = calc_data_slot(base_slot);

        // Determine if elements should be packed
        let byte_count = T::BYTE_COUNT;
        if byte_count < 32 && 32 % byte_count == 0 {
            // Pack multiple elements per slot
            store_packed_elements(self, storage, data_start, byte_count)
        } else {
            // Each element uses full slots
            store_unpacked_elements(self, storage, data_start)
        }
    }

    fn delete<S: StorageOps>(storage: &mut S, base_slot: U256) -> Result<()> {
        // Read length from base slot to determine how many slots to clear
        let length_value = storage.sload(base_slot)?;
        let length = length_value.to::<usize>();

        // Clear base slot (length)
        storage.sstore(base_slot, U256::ZERO)?;

        if length == 0 {
            return Ok(());
        }

        let data_start = calc_data_slot(base_slot);
        let byte_count = T::BYTE_COUNT;

        if byte_count < 32 && 32 % byte_count == 0 {
            // Clear packed element slots
            let slot_count = calc_packed_slot_count(length, byte_count);
            for slot_idx in 0..slot_count {
                storage.sstore(data_start + U256::from(slot_idx), U256::ZERO)?;
            }
        } else {
            // Clear unpacked element slots
            for elem_idx in 0..length {
                let elem_slot = data_start + U256::from(elem_idx);
                T::delete(storage, elem_slot)?;
            }
        }

        Ok(())
    }

    fn to_evm_words(&self) -> Result<[U256; 1]> {
        // Vec base slot representation: just the length
        Ok([U256::from(self.len())])
    }

    fn from_evm_words(_words: [U256; 1]) -> Result<Self> {
        Err(TempoPrecompileError::Fatal(
            "Cannot reconstruct `Vec` from base slot alone. Use `load()` with storage access."
                .into(),
        ))
    }
}

/// Load packed elements from storage.
///
/// Used when `T::BYTE_COUNT < 32` and evenly divides 32, allowing multiple elements per slot.
fn load_packed_elements<T, S>(
    storage: &mut S,
    data_start: U256,
    length: usize,
    byte_count: usize,
) -> Result<Vec<T>>
where
    T: Storable<1> + StorableType,
    S: StorageOps,
{
    let elements_per_slot = 32 / byte_count;
    let slot_count = calc_packed_slot_count(length, byte_count);

    let mut result = Vec::with_capacity(length);
    let mut current_offset = 0;

    for slot_idx in 0..slot_count {
        let slot_addr = data_start + U256::from(slot_idx);
        let slot_value = storage.sload(slot_addr)?;

        // How many elements in this slot?
        let elements_in_this_slot = if slot_idx == slot_count - 1 {
            // Last slot might be partially filled
            length - (slot_idx * elements_per_slot)
        } else {
            elements_per_slot
        };

        // Extract each element from this slot
        for _ in 0..elements_in_this_slot {
            let elem = extract_packed_value::<T>(slot_value, current_offset, byte_count)?;
            result.push(elem);

            // Move to next element position
            current_offset += byte_count;
            if current_offset >= 32 {
                current_offset = 0;
            }
        }

        // Reset offset for next slot
        current_offset = 0;
    }

    Ok(result)
}

/// Store packed elements to storage.
///
/// Packs multiple small elements into each 32-byte slot using bit manipulation.
fn store_packed_elements<T, S>(
    elements: &[T],
    storage: &mut S,
    data_start: U256,
    byte_count: usize,
) -> Result<()>
where
    T: Storable<1> + StorableType,
    S: StorageOps,
{
    let elements_per_slot = 32 / byte_count;
    let slot_count = calc_packed_slot_count(elements.len(), byte_count);

    for slot_idx in 0..slot_count {
        let slot_addr = data_start + U256::from(slot_idx);
        let start_elem = slot_idx * elements_per_slot;
        let end_elem = (start_elem + elements_per_slot).min(elements.len());

        // Build the slot value by packing multiple elements
        let mut slot_value = U256::ZERO;
        let mut current_offset = 0;

        for elem in &elements[start_elem..end_elem] {
            slot_value = insert_packed_value(slot_value, elem, current_offset, byte_count)?;
            current_offset += byte_count;
        }

        storage.sstore(slot_addr, slot_value)?;
    }

    Ok(())
}

/// Load unpacked elements from storage.
///
/// Used when elements don't pack efficiently (32 bytes or multi-slot types).
/// Each element occupies `T::SLOT_COUNT` consecutive slots.
fn load_unpacked_elements<T, S>(storage: &mut S, data_start: U256, length: usize) -> Result<Vec<T>>
where
    T: Storable<1>,
    S: StorageOps,
{
    let mut result = Vec::with_capacity(length);

    for elem_idx in 0..length {
        let elem_slot = data_start + U256::from(elem_idx);
        let elem = T::load(storage, elem_slot)?;
        result.push(elem);
    }

    Ok(result)
}

/// Store unpacked elements to storage.
///
/// Each element uses its full `T::SLOT_COUNT` consecutive slots.
fn store_unpacked_elements<T, S>(elements: &[T], storage: &mut S, data_start: U256) -> Result<()>
where
    T: Storable<1>,
    S: StorageOps,
{
    for (elem_idx, elem) in elements.iter().enumerate() {
        let elem_slot = data_start + U256::from(elem_idx);
        elem.store(storage, elem_slot)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PrecompileStorageProvider, StorageOps, hashmap::HashMapStorageProvider};
    use alloy::primitives::Address;
    use proptest::prelude::*;
    use tempo_precompiles_macros::Storable;

    // -- TEST HELPERS -------------------------------------------------------------

    /// Test helper that owns storage and implements StorageOps.
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

    /// Helper to extract and verify a packed value from a specific slot at a given offset.
    fn verify_packed_element<T>(
        contract: &mut TestContract,
        slot_addr: U256,
        expected: T,
        offset: usize,
        byte_count: usize,
        elem_name: &str,
    ) where
        T: Storable<1> + StorableType + PartialEq + std::fmt::Debug,
    {
        let slot_value = contract.sload(slot_addr).unwrap();
        let actual = extract_packed_value::<T>(slot_value, offset, byte_count).unwrap();
        assert_eq!(
            actual, expected,
            "{} at offset {} in slot {:?} mismatch",
            elem_name, offset, slot_addr
        );
    }

    // Strategy for generating random U256 slot values that won't overflow
    fn arb_safe_slot() -> impl Strategy<Value = U256> {
        any::<[u64; 4]>().prop_map(|limbs| {
            // Ensure we don't overflow by limiting to a reasonable range
            U256::from_limbs(limbs) % (U256::MAX - U256::from(10000))
        })
    }

    // Helper: Generate a single-slot struct for testing
    #[derive(Debug, Clone, PartialEq, Eq, Storable)]
    struct TestStruct {
        a: u128, // 16 bytes (slot 0)
        b: u128, // 16 bytes (slot 0)
    }

    #[test]
    fn test_vec_u8_roundtrip() {
        let mut contract = setup_test_contract();
        let base_slot = U256::ZERO;

        let data = vec![1u8, 2, 3, 4, 5];
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<u8> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Vec<u8> roundtrip failed");
    }

    #[test]
    fn test_vec_u16_roundtrip() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(100);

        let data = vec![100u16, 200, 300, 400, 500];
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<u16> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Vec<u16> roundtrip failed");
    }

    #[test]
    fn test_vec_u256_roundtrip() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(200);

        let data = vec![U256::from(12345), U256::from(67890), U256::from(111111)];
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<U256> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Vec<U256> roundtrip failed");
    }

    #[test]
    fn test_vec_address_roundtrip() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(300);

        let data = vec![
            Address::repeat_byte(0x11),
            Address::repeat_byte(0x22),
            Address::repeat_byte(0x33),
        ];
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<Address> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Vec<Address> roundtrip failed");
    }

    #[test]
    fn test_vec_empty() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(400);

        let data: Vec<u8> = vec![];
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<u8> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Empty vec roundtrip failed");
        assert!(loaded.is_empty(), "Loaded vec should be empty");
    }

    #[test]
    fn test_vec_delete() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(500);

        // Store data
        let data = vec![1u8, 2, 3, 4, 5];
        data.store(&mut contract, base_slot).unwrap();

        // Verify stored
        let loaded: Vec<u8> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Vec not stored correctly before delete");

        // Delete (static method)
        Vec::<u8>::delete(&mut contract, base_slot).unwrap();

        // Verify empty
        let loaded_after: Vec<u8> = Storable::load(&mut contract, base_slot).unwrap();
        assert!(loaded_after.is_empty(), "Vec not empty after delete");

        // Verify all data slots are cleared
        let data_start = calc_data_slot(base_slot);
        let byte_count = u8::BYTE_COUNT;
        let slot_count = calc_packed_slot_count(data.len(), byte_count);

        for i in 0..slot_count {
            let slot_value = contract.sload(data_start + U256::from(i)).unwrap();
            assert_eq!(
                slot_value,
                U256::ZERO,
                "Data slot {} not cleared after delete",
                i
            );
        }
    }

    #[test]
    fn test_vec_boundary_32_elements() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(600);

        // Exactly 32 u8 elements fit in one slot
        let data: Vec<u8> = (0..32).collect();
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<u8> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(
            loaded, data,
            "Vec with exactly 32 u8 elements failed roundtrip"
        );
    }

    #[test]
    fn test_vec_boundary_33_elements() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(700);

        // 33 u8 elements require 2 slots
        let data: Vec<u8> = (0..33).collect();
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<u8> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(
            loaded, data,
            "Vec with 33 u8 elements (2 slots) failed roundtrip"
        );
    }

    #[test]
    fn test_vec_nested() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(800);

        // Nested Vec<Vec<u8>>
        let data = vec![vec![1u8, 2, 3], vec![4, 5], vec![6, 7, 8, 9]];
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<Vec<u8>> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Nested Vec<Vec<u8>> roundtrip failed");
    }

    #[test]
    fn test_vec_struct_roundtrip() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(900);

        // Create Vec<TestStruct> with single-slot structs
        let data = vec![
            TestStruct { a: 12345, b: 11111 },
            TestStruct { a: 67890, b: 22222 },
            TestStruct { a: 11111, b: 33333 },
        ];
        data.store(&mut contract, base_slot).unwrap();

        let loaded: Vec<TestStruct> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data, "Vec<TestStruct> roundtrip failed");
    }

    #[test]
    fn test_vec_struct_delete() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(1000);

        // Store single-slot structs
        let data = vec![TestStruct { a: 999, b: 10 }, TestStruct { a: 888, b: 20 }];
        data.store(&mut contract, base_slot).unwrap();

        // Verify stored
        let loaded: Vec<TestStruct> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(
            loaded, data,
            "Vec<TestStruct> not stored correctly before delete"
        );

        // Delete
        Vec::<TestStruct>::delete(&mut contract, base_slot).unwrap();

        // Verify empty
        let loaded_after: Vec<TestStruct> = Storable::load(&mut contract, base_slot).unwrap();
        assert!(
            loaded_after.is_empty(),
            "Vec<TestStruct> not empty after delete"
        );

        // Verify all data slots are cleared
        // TestStruct is 32 bytes (1 slot), so each element uses one slot
        let data_start = calc_data_slot(base_slot);
        for elem_idx in 0..data.len() {
            let elem_slot = data_start + U256::from(elem_idx);
            let slot_value = contract.sload(elem_slot).unwrap();
            assert_eq!(
                slot_value,
                U256::ZERO,
                "Struct slot {} not cleared after delete",
                elem_idx
            );
        }
    }

    // -- SLOT-LEVEL VALIDATION TESTS ----------------------------------------------

    #[test]
    fn test_vec_u8_explicit_slot_packing() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2000);

        // Store exactly 5 u8 elements (should fit in 1 slot with 27 unused bytes)
        let data = vec![10u8, 20, 30, 40, 50];
        data.store(&mut contract, base_slot).unwrap();

        // Verify length stored in base slot
        let length_value = contract.sload(base_slot).unwrap();
        assert_eq!(length_value, U256::from(5), "Length not stored correctly");

        // Verify each element is at the correct offset in data slot 0
        let data_start = calc_data_slot(base_slot);
        let byte_count = u8::BYTE_COUNT; // 1 byte per u8

        verify_packed_element(&mut contract, data_start, 10u8, 0, byte_count, "elem[0]");
        verify_packed_element(&mut contract, data_start, 20u8, 1, byte_count, "elem[1]");
        verify_packed_element(&mut contract, data_start, 30u8, 2, byte_count, "elem[2]");
        verify_packed_element(&mut contract, data_start, 40u8, 3, byte_count, "elem[3]");
        verify_packed_element(&mut contract, data_start, 50u8, 4, byte_count, "elem[4]");

        // Verify unused bytes in the slot are zero
        let slot_value = contract.sload(data_start).unwrap();
        let slot_bytes = slot_value.to_be_bytes::<32>();
        for i in 5..32 {
            assert_eq!(
                slot_bytes[i], 0,
                "Unused byte at offset {} should be zero",
                i
            );
        }
    }

    #[test]
    fn test_vec_u16_slot_boundary() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2100);

        // Test 1: Exactly 16 u16 elements (fills exactly 1 slot: 16 * 2 bytes = 32 bytes)
        let data_exact: Vec<u16> = (0..16).map(|i| i * 100).collect();
        data_exact.store(&mut contract, base_slot).unwrap();

        let data_start = calc_data_slot(base_slot);
        let byte_count = u16::BYTE_COUNT; // 2 bytes per u16

        // Verify all 16 elements are in slot 0
        for (i, &expected) in data_exact.iter().enumerate() {
            verify_packed_element(
                &mut contract,
                data_start,
                expected,
                i * byte_count,
                byte_count,
                &format!("elem[{}]", i),
            );
        }

        // Test 2: 17 u16 elements (requires 2 slots)
        let data_overflow: Vec<u16> = (0..17).map(|i| i * 100).collect();
        data_overflow.store(&mut contract, base_slot).unwrap();

        // Verify first 16 are in slot 0
        for i in 0..16 {
            verify_packed_element(
                &mut contract,
                data_start,
                (i * 100) as u16,
                i * byte_count,
                byte_count,
                &format!("slot0_elem[{}]", i),
            );
        }

        // Verify 17th element is in slot 1 at offset 0
        let slot1_addr = data_start + U256::from(1);
        verify_packed_element(
            &mut contract,
            slot1_addr,
            1600u16,
            0,
            byte_count,
            "slot1_elem[0]",
        );

        // Verify remaining bytes in slot 1 are zero (30 unused bytes)
        let slot1_value = contract.sload(slot1_addr).unwrap();
        let slot1_bytes = slot1_value.to_be_bytes::<32>();
        for i in 2..32 {
            assert_eq!(
                slot1_bytes[i], 0,
                "Unused byte at offset {} in slot 1 should be zero",
                i
            );
        }
    }

    #[test]
    fn test_vec_u8_partial_slot_fill() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2200);

        // Store 35 u8 elements:
        // - Slot 0: 32 elements (full)
        // - Slot 1: 3 elements + 29 zeros
        let data: Vec<u8> = (0..35).map(|i| (i + 1) as u8).collect();
        data.store(&mut contract, base_slot).unwrap();

        let data_start = calc_data_slot(base_slot);
        let byte_count = u8::BYTE_COUNT;

        // Verify slot 0 is completely filled (32 elements)
        let slot0_value = contract.sload(data_start).unwrap();
        let slot0_bytes = slot0_value.to_be_bytes::<32>();
        for i in 0..32 {
            assert_eq!(
                slot0_bytes[i],
                (i + 1) as u8,
                "Element {} in slot 0 incorrect",
                i
            );
        }

        // Verify slot 1 has exactly 3 elements
        let slot1_addr = data_start + U256::from(1);
        verify_packed_element(
            &mut contract,
            slot1_addr,
            33u8,
            0,
            byte_count,
            "slot1_elem[0]",
        );
        verify_packed_element(
            &mut contract,
            slot1_addr,
            34u8,
            1,
            byte_count,
            "slot1_elem[1]",
        );
        verify_packed_element(
            &mut contract,
            slot1_addr,
            35u8,
            2,
            byte_count,
            "slot1_elem[2]",
        );

        // Verify remaining 29 bytes in slot 1 are zero
        let slot1_value = contract.sload(slot1_addr).unwrap();
        let slot1_bytes = slot1_value.to_be_bytes::<32>();
        for i in 3..32 {
            assert_eq!(
                slot1_bytes[i], 0,
                "Unused byte at offset {} in slot 1 should be zero",
                i
            );
        }
    }

    #[test]
    fn test_vec_u256_individual_slots() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2300);

        // Store 3 U256 values (each should occupy its own slot)
        let data = vec![
            U256::from(0x1111111111111111u64),
            U256::from(0x2222222222222222u64),
            U256::from(0x3333333333333333u64),
        ];
        data.store(&mut contract, base_slot).unwrap();

        let data_start = calc_data_slot(base_slot);

        // Verify each U256 occupies its own sequential slot
        for (i, &expected) in data.iter().enumerate() {
            let slot_addr = data_start + U256::from(i);
            let stored_value = contract.sload(slot_addr).unwrap();
            assert_eq!(
                stored_value, expected,
                "U256 element {} at slot {:?} incorrect",
                i, slot_addr
            );
        }

        // Verify there's no data in slot 3 (should be empty)
        let slot3_addr = data_start + U256::from(3);
        let slot3_value = contract.sload(slot3_addr).unwrap();
        assert_eq!(slot3_value, U256::ZERO, "Slot 3 should be empty");
    }

    #[test]
    fn test_vec_address_unpacked_slots() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2400);

        // Store 3 addresses (each 20 bytes, but 32 % 20 != 0, so unpacked)
        let data = vec![
            Address::repeat_byte(0xAA),
            Address::repeat_byte(0xBB),
            Address::repeat_byte(0xCC),
        ];
        data.store(&mut contract, base_slot).unwrap();

        let data_start = calc_data_slot(base_slot);

        // Verify each address occupies its own slot (unpacked)
        for (i, &expected) in data.iter().enumerate() {
            let slot_addr = data_start + U256::from(i);
            let stored_value = contract.sload(slot_addr).unwrap();

            // Address should be right-aligned in the U256 slot (leftmost 12 bytes are zero)
            let expected_u256 = U256::from_be_slice(expected.as_slice());
            assert_eq!(
                stored_value, expected_u256,
                "Address element {} at slot {:?} incorrect",
                i, slot_addr
            );

            // Verify leftmost 12 bytes are zero
            let slot_bytes = stored_value.to_be_bytes::<32>();
            for j in 0..12 {
                assert_eq!(
                    slot_bytes[j], 0,
                    "Padding byte {} should be zero for address at slot {}",
                    j, i
                );
            }
        }
    }

    #[test]
    fn test_vec_struct_slot_allocation() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2500);

        // Store Vec<TestStruct> with 3 single-slot structs
        let data = vec![
            TestStruct { a: 100, b: 1 },
            TestStruct { a: 200, b: 2 },
            TestStruct { a: 300, b: 3 },
        ];
        data.store(&mut contract, base_slot).unwrap();

        let data_start = calc_data_slot(base_slot);

        // Each TestStruct uses one slot (32 bytes: two u128 values)
        // Verify each struct is stored at sequential slots
        for (i, expected_struct) in data.iter().enumerate() {
            let struct_slot = data_start + U256::from(i);
            let loaded_struct = TestStruct::load(&mut contract, struct_slot).unwrap();
            assert_eq!(
                loaded_struct, *expected_struct,
                "TestStruct at slot {} incorrect",
                i
            );
        }

        // Verify slot allocation is correct (no gaps)
        let slot0 = data_start;
        let slot1 = data_start + U256::from(1);
        let slot2 = data_start + U256::from(2);
        let slot3 = data_start + U256::from(3);

        // First 3 slots should be non-zero (contain structs)
        assert_ne!(
            contract.sload(slot0).unwrap(),
            U256::ZERO,
            "Slot 0 should contain data"
        );
        assert_ne!(
            contract.sload(slot1).unwrap(),
            U256::ZERO,
            "Slot 1 should contain data"
        );
        assert_ne!(
            contract.sload(slot2).unwrap(),
            U256::ZERO,
            "Slot 2 should contain data"
        );

        // Slot 3 should be empty
        assert_eq!(
            contract.sload(slot3).unwrap(),
            U256::ZERO,
            "Slot 3 should be empty"
        );
    }

    #[test]
    fn test_vec_length_slot_isolation() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2600);

        // Store a vec
        let data = vec![100u8, 200, 250];
        data.store(&mut contract, base_slot).unwrap();

        // Verify base slot contains length
        let length_value = contract.sload(base_slot).unwrap();
        assert_eq!(length_value, U256::from(3), "Length slot incorrect");

        // Verify data starts at keccak256(base_slot), not base_slot + 1
        let data_start = calc_data_slot(base_slot);
        assert_ne!(
            data_start,
            base_slot + U256::from(1),
            "Data should not start immediately after base slot"
        );

        // Verify data is at the calculated data_start location
        let data_slot_value = contract.sload(data_start).unwrap();
        assert_ne!(
            data_slot_value,
            U256::ZERO,
            "Data slot should contain packed elements"
        );

        // Verify we can extract all elements from the data slot
        verify_packed_element(&mut contract, data_start, 100u8, 0, 1, "elem[0]");
        verify_packed_element(&mut contract, data_start, 200u8, 1, 1, "elem[1]");
        verify_packed_element(&mut contract, data_start, 250u8, 2, 1, "elem[2]");
    }

    #[test]
    fn test_vec_overwrite_cleanup() {
        let mut contract = setup_test_contract();
        let base_slot = U256::from(2700);

        // Store a vec with 5 u8 elements (requires 1 slot)
        let data_long = vec![1u8, 2, 3, 4, 5];
        data_long.store(&mut contract, base_slot).unwrap();

        let data_start = calc_data_slot(base_slot);

        // Verify initial storage
        let slot0_before = contract.sload(data_start).unwrap();
        assert_ne!(slot0_before, U256::ZERO, "Initial data should be stored");

        // Overwrite with a shorter vec (3 elements)
        let data_short = vec![10u8, 20, 30];
        data_short.store(&mut contract, base_slot).unwrap();

        // Verify length updated
        let length_value = contract.sload(base_slot).unwrap();
        assert_eq!(length_value, U256::from(3), "Length should be updated");

        // Verify new data is correct
        verify_packed_element(&mut contract, data_start, 10u8, 0, 1, "new_elem[0]");
        verify_packed_element(&mut contract, data_start, 20u8, 1, 1, "new_elem[1]");
        verify_packed_element(&mut contract, data_start, 30u8, 2, 1, "new_elem[2]");

        let loaded: Vec<u8> = Storable::load(&mut contract, base_slot).unwrap();
        assert_eq!(loaded, data_short, "Loaded vec should match short version");
        assert_eq!(loaded.len(), 3, "Length should be 3");

        // If we want full cleanup, we should delete first, then store
        Vec::<u8>::delete(&mut contract, base_slot).unwrap();
        data_short.store(&mut contract, base_slot).unwrap();

        // Now verify old bytes are actually cleared
        let slot0_after_delete = contract.sload(data_start).unwrap();
        let slot_bytes = slot0_after_delete.to_be_bytes::<32>();

        // First 3 bytes should have new data
        assert_eq!(slot_bytes[0], 10);
        assert_eq!(slot_bytes[1], 20);
        assert_eq!(slot_bytes[2], 30);

        // Bytes 3-31 should be zero
        for i in 3..32 {
            assert_eq!(
                slot_bytes[i], 0,
                "Byte {} should be zero after delete+store",
                i
            );
        }
    }

    // -- PROPTEST STRATEGIES ------------------------------------------------------

    prop_compose! {
        fn arb_u8_vec(max_len: usize)
                     (vec in prop::collection::vec(any::<u8>(), 0..=max_len))
                     -> Vec<u8> {
            vec
        }
    }

    prop_compose! {
        fn arb_u16_vec(max_len: usize)
                      (vec in prop::collection::vec(any::<u16>(), 0..=max_len))
                      -> Vec<u16> {
            vec
        }
    }

    prop_compose! {
        fn arb_u256_vec(max_len: usize)
                       (vec in prop::collection::vec(any::<u64>(), 0..=max_len))
                       -> Vec<U256> {
            vec.into_iter().map(U256::from).collect()
        }
    }

    prop_compose! {
        fn arb_address_vec(max_len: usize)
                          (vec in prop::collection::vec(any::<[u8; 20]>(), 0..=max_len))
                          -> Vec<Address> {
            vec.into_iter().map(Address::from).collect()
        }
    }

    prop_compose! {
        fn arb_test_struct()
                          (a in any::<u64>(),
                           b in any::<u64>())
                          -> TestStruct {
            TestStruct {
                a: a as u128,
                b: b as u128,
            }
        }
    }

    prop_compose! {
        fn arb_test_struct_vec(max_len: usize)
                              (vec in prop::collection::vec(arb_test_struct(), 0..=max_len))
                              -> Vec<TestStruct> {
            vec
        }
    }

    // -- PROPERTY TESTS -----------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]
        #[test]
        fn proptest_vec_u8_roundtrip(data in arb_u8_vec(100), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();
            let data_len = data.len();

            // Store → Load roundtrip
            data.store(&mut contract, base_slot)?;
            let loaded: Vec<u8> = Storable::load(&mut contract, base_slot)?;
            prop_assert_eq!(&loaded, &data, "Vec<u8> roundtrip failed");

            // Delete + verify cleanup
            Vec::<u8>::delete(&mut contract, base_slot)?;
            let after_delete: Vec<u8> = Storable::load(&mut contract, base_slot)?;
            prop_assert!(after_delete.is_empty(), "Vec not empty after delete");

            // Verify data slots are cleared (if length > 0)
            if data_len > 0 {
                let data_start = calc_data_slot(base_slot);
                let byte_count = u8::BYTE_COUNT;
                let slot_count = calc_packed_slot_count(data_len, byte_count);

                for i in 0..slot_count {
                    let slot_value = contract.sload(data_start + U256::from(i))?;
                    prop_assert_eq!(slot_value, U256::ZERO, "Data slot {} not cleared", i);
                }
            }

            // EVM words roundtrip (should error)
            let words = data.to_evm_words()?;
            let result = Vec::<u8>::from_evm_words(words);
            prop_assert!(result.is_err(), "Vec should not be reconstructable from base slot alone");
        }

        #[test]
        fn proptest_vec_u16_roundtrip(data in arb_u16_vec(100), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();
            let data_len = data.len();

            // Store → Load roundtrip
            data.store(&mut contract, base_slot)?;
            let loaded: Vec<u16> = Storable::load(&mut contract, base_slot)?;
            prop_assert_eq!(&loaded, &data, "Vec<u16> roundtrip failed");

            // Delete + verify cleanup
            Vec::<u16>::delete(&mut contract, base_slot)?;
            let after_delete: Vec<u16> = Storable::load(&mut contract, base_slot)?;
            prop_assert!(after_delete.is_empty(), "Vec not empty after delete");

            // Verify data slots are cleared (if length > 0)
            if data_len > 0 {
                let data_start = calc_data_slot(base_slot);
                let byte_count = u16::BYTE_COUNT;
                let slot_count = calc_packed_slot_count(data_len, byte_count);

                for i in 0..slot_count {
                    let slot_value = contract.sload(data_start + U256::from(i))?;
                    prop_assert_eq!(slot_value, U256::ZERO, "Data slot {} not cleared", i);
                }
            }

            // EVM words roundtrip (should error)
            let words = data.to_evm_words()?;
            let result = Vec::<u16>::from_evm_words(words);
            prop_assert!(result.is_err(), "Vec should not be reconstructable from base slot alone");
        }

        #[test]
        fn proptest_vec_u256_roundtrip(data in arb_u256_vec(50), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();
            let data_len = data.len();

            // Store → Load roundtrip
            data.store(&mut contract, base_slot)?;
            let loaded: Vec<U256> = Storable::load(&mut contract, base_slot)?;
            prop_assert_eq!(&loaded, &data, "Vec<U256> roundtrip failed");

            // Delete + verify cleanup
            Vec::<U256>::delete(&mut contract, base_slot)?;
            let after_delete: Vec<U256> = Storable::load(&mut contract, base_slot)?;
            prop_assert!(after_delete.is_empty(), "Vec not empty after delete");

            // Verify data slots are cleared (if length > 0)
            if data_len > 0 {
                let data_start = calc_data_slot(base_slot);

                for i in 0..data_len {
                    let slot_value = contract.sload(data_start + U256::from(i))?;
                    prop_assert_eq!(slot_value, U256::ZERO, "Data slot {} not cleared", i);
                }
            }

            // EVM words roundtrip (should error)
            let words = data.to_evm_words()?;
            let result = Vec::<U256>::from_evm_words(words);
            prop_assert!(result.is_err(), "Vec should not be reconstructable from base slot alone");
        }

        #[test]
        fn proptest_vec_address_roundtrip(data in arb_address_vec(50), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();
            let data_len = data.len();

            // Store → Load roundtrip
            data.store(&mut contract, base_slot)?;
            let loaded: Vec<Address> = Storable::load(&mut contract, base_slot)?;
            prop_assert_eq!(&loaded, &data, "Vec<Address> roundtrip failed");

            // Delete + verify cleanup
            Vec::<Address>::delete(&mut contract, base_slot)?;
            let after_delete: Vec<Address> = Storable::load(&mut contract, base_slot)?;
            prop_assert!(after_delete.is_empty(), "Vec not empty after delete");

            // Verify data slots are cleared (if length > 0)
            // Address is 20 bytes, but 32 % 20 != 0, so they don't pack and each uses one slot
            if data_len > 0 {
                let data_start = calc_data_slot(base_slot);

                for i in 0..data_len {
                    let slot_value = contract.sload(data_start + U256::from(i))?;
                    prop_assert_eq!(slot_value, U256::ZERO, "Data slot {} not cleared", i);
                }
            }

            // EVM words roundtrip (should error)
            let words = data.to_evm_words()?;
            let result = Vec::<Address>::from_evm_words(words);
            prop_assert!(result.is_err(), "Vec should not be reconstructable from base slot alone");
        }

        #[test]
        fn proptest_vec_delete(data in arb_u8_vec(100), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();

            // Store data
            data.store(&mut contract, base_slot)?;

            // Delete
            Vec::<u8>::delete(&mut contract, base_slot)?;

            // Verify empty after delete
            let loaded: Vec<u8> = Storable::load(&mut contract, base_slot)?;
            prop_assert!(loaded.is_empty(), "Vec not empty after delete");

            // Verify data slots are cleared (if length > 0)
            if !data.is_empty() {
                let data_start = calc_data_slot(base_slot);
                let byte_count = u8::BYTE_COUNT;
                let slot_count = calc_packed_slot_count(data.len(), byte_count);

                for i in 0..slot_count {
                    let slot_value = contract.sload(data_start + U256::from(i))?;
                    prop_assert_eq!(slot_value, U256::ZERO, "Data slot {} not cleared", i);
                }
            }
        }

        #[test]
        fn proptest_vec_struct_roundtrip(data in arb_test_struct_vec(50), base_slot in arb_safe_slot()) {
            let mut contract = setup_test_contract();
            let data_len = data.len();

            // Store → Load roundtrip
            data.store(&mut contract, base_slot)?;
            let loaded: Vec<TestStruct> = Storable::load(&mut contract, base_slot)?;
            prop_assert_eq!(&loaded, &data, "Vec<TestStruct> roundtrip failed");

            // Delete + verify cleanup
            Vec::<TestStruct>::delete(&mut contract, base_slot)?;
            let after_delete: Vec<TestStruct> = Storable::load(&mut contract, base_slot)?;
            prop_assert!(after_delete.is_empty(), "Vec not empty after delete");

            // Verify data slots are cleared (if length > 0)
            if data_len > 0 {
                let data_start = calc_data_slot(base_slot);

                for i in 0..data_len {
                    let slot_value = contract.sload(data_start + U256::from(i))?;
                    prop_assert_eq!(slot_value, U256::ZERO, "Data slot {} not cleared", i);
                }
            }

            // EVM words roundtrip (should error)
            let words = data.to_evm_words()?;
            let result = Vec::<TestStruct>::from_evm_words(words);
            prop_assert!(result.is_err(), "Vec should not be reconstructable from base slot alone");
        }
    }
}
