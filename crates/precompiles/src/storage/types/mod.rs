mod slot;
pub use slot::*;

pub mod mapping;
pub use mapping::*;

pub mod array;
pub mod vec;

mod bytes_like;
mod primitives;

use crate::{error::Result, storage::StorageOps};
use alloy::primitives::{Address, U256};
use std::rc::Rc;

/// Describes how a type is laid out in EVM storage.
///
/// This determines whether a type can be packed with other fields
/// and how many storage slots it occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Single slot, N bytes (1-32). Can be packed with other fields if N < 32.
    ///
    /// Used for primitive types like integers, booleans, and addresses.
    Bytes(usize),

    /// Occupies N full slots (each 32 bytes). Cannot be packed.
    ///
    /// Used for structs, fixed-size arrays, and dynamic types.
    Slots(usize),
}

impl Layout {
    /// Returns true if this field can be packed with adjacent fields.
    pub const fn is_packable(&self) -> bool {
        match self {
            // TODO(rusowsky): use `Self::Bytes(n) => *n < 32` to reduce gas usage.
            // Note that this requires a hardfork and must be properly coordinated.
            Self::Bytes(_) => true,
            Self::Slots(_) => false,
        }
    }

    /// Returns the number of storage slots this type occupies.
    pub const fn slots(&self) -> usize {
        match self {
            Self::Bytes(_) => 1,
            Self::Slots(n) => *n,
        }
    }

    /// Returns the number of bytes this type occupies.
    ///
    /// For `Bytes(n)`, returns n.
    /// For `Slots(n)`, returns n * 32 (each slot is 32 bytes).
    pub const fn bytes(&self) -> usize {
        match self {
            Self::Bytes(n) => *n,
            Self::Slots(n) => {
                // Compute n * 32 using repeated addition for const compatibility
                let (mut i, mut result) = (0, 0);
                while i < *n {
                    result += 32;
                    i += 1;
                }
                result
            }
        }
    }
}

/// Describes the context in which a storable value is being loaded or stored.
///
/// Determines whether the value occupies an entire storage slot or is packed
/// with other values at a specific byte offset within a slot.
///
/// **NOTE:** This type is not an enum to minimize its memory size, but its
/// implementation is equivalent to:
/// ```rs
/// enum LayoutCtx {
///    Full,
///    Packed(usize)
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct LayoutCtx(usize);

impl LayoutCtx {
    /// Load/store the entire value at a given slot.
    ///
    /// For writes, this directly overwrites the entire slot without needing SLOAD.
    /// All storable types support this context.
    pub const FULL: Self = Self(usize::MAX);

    /// Load/store a packed primitive at the given byte offset within a slot.
    ///
    /// For writes, this requires a read-modify-write: SLOAD the current slot value,
    /// modify the bytes at the offset, then SSTORE back. This preserves other
    /// packed fields in the same slot.
    ///
    /// Only primitive types with `Layout::Bytes(n)` where `n < 32` support this context.
    pub const fn packed(offset: usize) -> Self {
        debug_assert!(offset < 32);
        Self(offset)
    }

    /// Get the packed offset, returns `None` for `Full`
    #[inline]
    pub const fn packed_offset(&self) -> Option<usize> {
        if self.0 == usize::MAX {
            None
        } else {
            Some(self.0)
        }
    }
}

/// Helper trait to access storage layout information without requiring const generic parameter.
///
/// This trait provides compile-time layout information (slot count, byte size, packability)
/// and a factory method for creating handlers. It enables the derive macro to compute
/// struct layouts before the final slot count is known.
///
/// **NOTE:** Don't need to implement the trait manually. Use `#[derive(Storable)]` instead.
pub trait StorableType {
    /// Describes how this type is laid out in storage.
    ///
    /// - Primitives use `Layout::Bytes(N)` where N is their size
    /// - Dynamic types (String, Bytes, Vec) use `Layout::Slots(1)`
    /// - Structs and arrays use `Layout::Slots(N)` where N is the slot count
    const LAYOUT: Layout;

    /// Number of storage slots this type takes.
    const SLOTS: usize = Self::LAYOUT.slots();

    /// Number of bytes this type takes.
    const BYTES: usize = Self::LAYOUT.bytes();

    /// Whether this type can be packed with adjacent fields.
    const IS_PACKABLE: bool = Self::LAYOUT.is_packable();

    /// The handler type that provides storage access for this type.
    ///
    /// For primitives, this is `Slot<Self>`.
    /// For mappings, this is `Self` (mappings are their own handlers).
    /// For user-defined structs, this is a generated handler type (e.g., `MyStructHandler`).
    type Handler;

    /// Creates a handler for this type at the given storage location.
    fn handle(slot: U256, ctx: LayoutCtx, address: Rc<Address>) -> Self::Handler;
}

/// Abstracts reading, writing, and deleting values for [`Storable`] types.
pub trait Handler<T: Storable> {
    /// Reads the value from storage.
    fn read(&self) -> Result<T>;

    /// Writes the value to storage.
    fn write(&mut self, value: T) -> Result<()>;

    /// Deletes the value from storage (sets to zero).
    fn delete(&mut self) -> Result<()>;
}

/// High-level storage operations for storable types.
///
/// This trait provides storage I/O operations: load, store, delete.
/// Types implement their own logic for handling packed vs full-slot contexts.
///
/// The trait hides the const generic `WORDS` from [`Encodable<WORDS>`], allowing
/// [`Handler<T>`] to work uniformly across all types.
pub trait Storable: StorableType + Sized {
    /// Load this type from storage at the given slot.
    fn load<S: StorageOps>(storage: &S, slot: U256, ctx: LayoutCtx) -> Result<Self>;

    /// Store this type to storage at the given slot.
    fn store<S: StorageOps>(&self, storage: &mut S, slot: U256, ctx: LayoutCtx) -> Result<()>;

    /// Delete this type from storage (set to zero).
    ///
    /// Default implementation handles both full-slot and packed contexts:
    /// - `LayoutCtx::FULL`: Writes zero to all `Self::SLOTS` consecutive slots
    /// - `LayoutCtx::packed(offset)`: Clears only the bytes at the offset (read-modify-write)
    fn delete<S: StorageOps>(storage: &mut S, slot: U256, ctx: LayoutCtx) -> Result<()> {
        match ctx.packed_offset() {
            None => {
                for offset in 0..Self::SLOTS {
                    storage.sstore(slot + U256::from(offset), U256::ZERO)?;
                }
                Ok(())
            }
            Some(offset) => {
                // For packed context, we need to preserve other fields in the slot
                let bytes = Self::BYTES;
                let current = storage.sload(slot)?;
                let cleared = crate::storage::packing::zero_packed_value(current, offset, bytes)?;
                storage.sstore(slot, cleared)
            }
        }
    }

    /// Encode this value to a single U256 word for packing.
    ///
    /// Only valid for single-slot primitive types (`SLOTS == 1`).
    /// Multi-slot types will panic in debug builds and produce garbage in release.
    fn to_word(&self) -> Result<U256>;

    /// Decode this type from a single U256 word.
    ///
    /// Only valid for single-slot primitive types (`SLOTS == 1`).
    /// Multi-slot types will panic in debug builds and produce garbage in release.
    fn from_word(word: U256) -> Result<Self>;
}

/// Trait for encoding/decoding Rust types to/from EVM storage words.
///
/// This trait provides pure conversion between Rust types and arrays of U256 words.
///
/// # Type Parameter
///
/// - `WORDS`: The number of U256 words this type encodes to.
///   For single-word types (Address, U256, bool), this is `1`.
///   For fixed-size arrays, this depends on packing.
///   For user-defined structs, this is between `1` and the number of fields.
///
/// # Safety
///
/// Implementations must ensure that:
/// - Round-trip conversions preserve data: `from_evm_words(to_evm_words(x)) == Ok(x)`
/// - `WORDS` accurately reflects the number of words produced/consumed
/// - `to_evm_words` and `from_evm_words` produce/consume exactly `WORDS` words
pub trait Encodable<const WORDS: usize>: Sized + StorableType {
    /// Encode this type to an array of U256 words.
    ///
    /// Returns exactly `WORDS` words, where each word represents one storage slot.
    /// For single-slot types (`WORDS = 1`), returns a single-element array.
    /// For multi-slot types, each array element corresponds to one slot's data.
    ///
    /// # Packed Storage
    ///
    /// When multiple small fields are packed into a single slot, they are
    /// positioned and combined into a single U256 word according to their
    /// byte offsets. The derive macro handles this automatically.
    fn to_evm_words(&self) -> Result<[U256; WORDS]>;

    /// Decode this type from an array of U256 words.
    ///
    /// Accepts exactly `WORDS` words, where each word represents one storage slot.
    /// Constructs the complete type from all provided words.
    ///
    /// # Packed Storage
    ///
    /// When multiple small fields are packed into a single slot, they are
    /// extracted from the appropriate word using bit shifts and masks.
    /// The derive macro handles this automatically.
    fn from_evm_words(words: [U256; WORDS]) -> Result<Self>;

    /// Test helper to ensure `LAYOUT` and `WORDS` are in sync.
    fn validate_layout() {
        debug_assert_eq!(<Self as StorableType>::SLOTS, WORDS)
    }
}

/// Trait for types that can be used as storage mapping keys.
///
/// Keys are hashed using keccak256 along with the mapping's base slot
/// to determine the final storage location. This trait provides the
/// byte representation used in that hash.
pub trait StorageKey {
    fn as_storage_bytes(&self) -> impl AsRef<[u8]>;
}
