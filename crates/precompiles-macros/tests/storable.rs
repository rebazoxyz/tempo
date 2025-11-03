//! Unit tests for the #[derive(Storable)] macro in isolation.
//! These tests verify that user-defined structs properly implement store, load, and delete operations.

// Re-export `tempo_precompiles::storage` as a local module so `crate::storage` works
mod storage {
    pub(super) use tempo_precompiles::storage::*;
}

use alloy::primitives::{Address, U256};
use storage::{
    ContractStorage, PrecompileStorageProvider, Storable, StorableType,
    hashmap::HashMapStorageProvider,
};
use tempo_precompiles::error;
use tempo_precompiles_macros::Storable;

// Test wrapper that combines address + storage provider to implement ContractStorage
struct TestStorage<S> {
    address: Address,
    storage: S,
}

impl<S: PrecompileStorageProvider> ContractStorage for TestStorage<S> {
    type Storage = S;
    fn address(&self) -> Address {
        self.address
    }
    fn storage(&mut self) -> &mut Self::Storage {
        &mut self.storage
    }
}

// Helper to generate addresses
fn test_address(byte: u8) -> Address {
    let mut bytes = [0u8; 20];
    bytes[19] = byte;
    Address::from(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Storable)]
struct PackedTwo {
    pub addr: Address, // 20 bytes   (slot 0)
    pub count: u64,    // 8 bytes    (slot 0)
}

#[derive(Debug, Clone, PartialEq, Eq, Storable)]
struct PackedThree {
    pub a: u64, // 8 bytes (slot 0)
    pub b: u64, // 8 bytes (slot 0)
    pub c: u64, // 8 bytes (slot 0)
}

#[derive(Debug, Clone, PartialEq, Eq, Storable)]
struct PartiallyPacked {
    pub addr1: Address, // 20 bytes (slot 0)
    pub flag: bool,     // 1 byte   (slot 0)
    pub value: U256,    // 32 bytes (slot 1)
    pub addr2: Address, // 20 bytes (slot 2)
}

// NOTE(rusowsky): Nested struct support has not been implemented yet.
/*
#[derive(Debug, Clone, PartialEq, Eq, Storable)]
struct WithNestedStruct {
    pub id: i16,           // 2 bytes    (slot 0)
    pub nested: PackedTwo, // 28 bytes   (slot 0)
    pub active: bool,      // 1 byte     (slot 1)
    pub value: U256,       // 32 bytes   (slot 2)
}
*/

#[test]
fn test_slot_count_calculation() {
    // Verify SLOT_COUNT is correctly calculated for various struct sizes
    assert_eq!(PackedTwo::SLOT_COUNT, 1);
    assert_eq!(PackedTwo::BYTE_COUNT, 28);
    assert_eq!(PackedThree::SLOT_COUNT, 1);
    assert_eq!(PackedThree::BYTE_COUNT, 24);
    assert_eq!(PartiallyPacked::SLOT_COUNT, 3);
    assert_eq!(PartiallyPacked::BYTE_COUNT, 84);

    // Verify nested structs (commented out - nested structs no longer supported)
    // assert_eq!(WithNestedStruct::SLOT_COUNT, 2);
    // assert_eq!(WithNestedStruct::BYTE_COUNT, 64); // 2 + 28 + 1 + 32
}

#[test]
fn test_packed_two_fields() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };
    let base_slot = U256::from(1000);

    let original = PackedTwo {
        addr: test_address(42),
        count: 12345,
    };

    // Store
    original.store(&mut storage, base_slot).unwrap();

    // Load and verify
    let loaded = PackedTwo::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, original);
    assert_eq!(loaded.addr, test_address(42));
    assert_eq!(loaded.count, 12345);
}

#[test]
fn test_packed_three_fields() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };
    let base_slot = U256::from(2000);

    let original = PackedThree {
        a: 111,
        b: 222,
        c: 333,
    };

    // Store
    original.store(&mut storage, base_slot).unwrap();

    // Load and verify
    let loaded = PackedThree::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, original);
    assert_eq!(loaded.a, 111);
    assert_eq!(loaded.b, 222);
    assert_eq!(loaded.c, 333);
}

#[test]
fn test_partially_packed() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };
    let base_slot = U256::from(3000);

    let original = PartiallyPacked {
        addr1: test_address(10),
        flag: true,
        value: U256::from(999_999),
        addr2: test_address(20),
    };

    // Store
    original.store(&mut storage, base_slot).unwrap();

    // Load and verify
    let loaded = PartiallyPacked::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, original);
    assert_eq!(loaded.addr1, test_address(10));
    assert_eq!(loaded.flag, true);
    assert_eq!(loaded.value, U256::from(999_999));
    assert_eq!(loaded.addr2, test_address(20));
}

#[test]
fn test_packed_fields_update_individual() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };

    // Test PackedTwoFields
    let base_slot = U256::from(5000);

    // Store initial values
    let initial = PackedTwo {
        addr: test_address(10),
        count: 100,
    };
    initial.store(&mut storage, base_slot).unwrap();

    // Verify initial load
    let loaded1 = PackedTwo::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded1, initial);

    // Update with different values
    let updated = PackedTwo {
        addr: test_address(20),
        count: 200,
    };
    updated.store(&mut storage, base_slot).unwrap();

    // Verify updated values
    let loaded2 = PackedTwo::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded2, updated);
    assert_eq!(loaded2.addr, test_address(20));
    assert_eq!(loaded2.count, 200);

    // Test PackedThreeSmall
    let base_slot = U256::from(5100);

    // Store initial values
    let initial = PackedThree {
        a: 100,
        b: 200,
        c: 300,
    };
    initial.store(&mut storage, base_slot).unwrap();

    // Verify initial load
    let loaded1 = PackedThree::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded1, initial);

    // Update with different values
    let updated = PackedThree {
        a: 111,
        b: 222,
        c: 333,
    };
    updated.store(&mut storage, base_slot).unwrap();

    // Verify updated values
    let loaded2 = PackedThree::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded2, updated);
    assert_eq!(loaded2.a, 111);
    assert_eq!(loaded2.b, 222);
    assert_eq!(loaded2.c, 333);

    // Test PartiallyPacked
    let base_slot = U256::from(5200);

    // Store initial values
    let initial = PartiallyPacked {
        addr1: test_address(30),
        flag: false,
        value: U256::from(111_111),
        addr2: test_address(40),
    };
    initial.store(&mut storage, base_slot).unwrap();

    // Verify initial load
    let loaded1 = PartiallyPacked::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded1, initial);

    // Update with different values
    let updated = PartiallyPacked {
        addr1: test_address(50),
        flag: true,
        value: U256::from(999_999),
        addr2: test_address(60),
    };
    updated.store(&mut storage, base_slot).unwrap();

    // Verify updated values
    let loaded2 = PartiallyPacked::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded2, updated);
    assert_eq!(loaded2.addr1, test_address(50));
    assert_eq!(loaded2.flag, true);
    assert_eq!(loaded2.value, U256::from(999_999));
    assert_eq!(loaded2.addr2, test_address(60));
}

#[test]
fn test_packed_fields_delete() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };

    // Test PackedTwoFields
    let base_slot = U256::from(6000);

    let data = PackedTwo {
        addr: test_address(99),
        count: 12345,
    };

    // Store data
    data.store(&mut storage, base_slot).unwrap();
    let loaded = PackedTwo::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, data);

    // Delete (should return zero values)
    PackedTwo::delete(&mut storage, base_slot).unwrap();
    let after_delete = PackedTwo::load(&mut storage, base_slot).unwrap();
    assert_eq!(after_delete.addr, Address::ZERO);
    assert_eq!(after_delete.count, 0);

    // Test PackedThreeSmall
    let base_slot = U256::from(6100);

    let data = PackedThree {
        a: 777,
        b: 888,
        c: 999,
    };

    // Store data
    data.store(&mut storage, base_slot).unwrap();

    // Verify stored
    let loaded = PackedThree::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, data);

    // Delete
    PackedThree::delete(&mut storage, base_slot).unwrap();

    // Verify deleted (should return zero values)
    let after_delete = PackedThree::load(&mut storage, base_slot).unwrap();
    assert_eq!(after_delete.a, 0);
    assert_eq!(after_delete.b, 0);
    assert_eq!(after_delete.c, 0);

    // Test PartiallyPacked
    let base_slot = U256::from(6200);

    let data = PartiallyPacked {
        addr1: test_address(77),
        flag: true,
        value: U256::from(555_555),
        addr2: test_address(88),
    };

    // Store data
    data.store(&mut storage, base_slot).unwrap();

    // Verify stored
    let loaded = PartiallyPacked::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, data);

    // Delete
    PartiallyPacked::delete(&mut storage, base_slot).unwrap();

    // Verify deleted (should return zero values)
    let after_delete = PartiallyPacked::load(&mut storage, base_slot).unwrap();
    assert_eq!(after_delete.addr1, Address::ZERO);
    assert_eq!(after_delete.flag, false);
    assert_eq!(after_delete.value, U256::ZERO);
    assert_eq!(after_delete.addr2, Address::ZERO);
}

/* The following tests are commented out because nested structs are no supported yet

#[test]
fn test_nested_struct_store_load() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };
    let base_slot = U256::from(7000);

    // Create nested data
    let nested = PackedTwo {
        addr: test_address(55),
        count: 9999,
    };

    let original = WithNestedStruct {
        id: 42,
        active: true,
        nested,
        value: U256::from(123_456),
    };

    // Store
    original.store(&mut storage, base_slot).unwrap();

    // Load and verify all fields including nested
    let loaded = WithNestedStruct::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, original);
    assert_eq!(loaded.id, 42);
    assert_eq!(loaded.nested.addr, test_address(55));
    assert_eq!(loaded.nested.count, 9999);
    assert_eq!(loaded.active, true);
    assert_eq!(loaded.value, U256::from(123_456));
}

#[test]
fn test_nested_struct_update() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };
    let base_slot = U256::from(8000);

    // Store initial values
    let initial = WithNestedStruct {
        id: 10,
        active: false,
        nested: PackedTwo {
            addr: test_address(11),
            count: 100,
        },
        value: U256::from(1000),
    };
    initial.store(&mut storage, base_slot).unwrap();

    // Verify initial load
    let loaded1 = WithNestedStruct::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded1, initial);

    // Update all fields including nested struct
    let updated = WithNestedStruct {
        id: 20,
        nested: PackedTwo {
            addr: test_address(22),
            count: 200,
        },
        active: true,
        value: U256::from(2000),
    };
    updated.store(&mut storage, base_slot).unwrap();

    // Verify all fields were updated correctly
    let loaded2 = WithNestedStruct::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded2, updated);
    assert_eq!(loaded2.id, 20);
    assert_eq!(loaded2.nested.addr, test_address(22));
    assert_eq!(loaded2.nested.count, 200);
    assert_eq!(loaded2.active, true);
    assert_eq!(loaded2.value, U256::from(2000));
}

#[test]
fn test_nested_struct_delete() {
    let mut storage = TestStorage {
        address: test_address(1),
        storage: HashMapStorageProvider::new(1),
    };
    let base_slot = U256::from(9000);

    let data = WithNestedStruct {
        id: 99,
        nested: PackedTwo {
            addr: test_address(88),
            count: 7777,
        },
        active: true,
        value: U256::from(888_888),
    };

    // Store data
    data.store(&mut storage, base_slot).unwrap();

    // Verify stored
    let loaded = WithNestedStruct::load(&mut storage, base_slot).unwrap();
    assert_eq!(loaded, data);

    // Delete
    WithNestedStruct::delete(&mut storage, base_slot).unwrap();

    // Verify all fields including nested struct are zeroed
    let after_delete = WithNestedStruct::load(&mut storage, base_slot).unwrap();
    assert_eq!(after_delete.id, 0);
    assert_eq!(after_delete.nested.addr, Address::ZERO);
    assert_eq!(after_delete.nested.count, 0);
    assert_eq!(after_delete.active, false);
    assert_eq!(after_delete.value, U256::ZERO);
}
*/
