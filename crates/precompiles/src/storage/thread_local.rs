use std::{cell::Cell, marker::PhantomData};

use crate::{
    error::{Result, TempoPrecompileError},
    storage::PrecompileStorageProvider,
};

// Thread-local storage for accessing `PrecompileStorageProvider`
thread_local! {
    static STORAGE: Cell<Option<*mut dyn PrecompileStorageProvider>> = const { Cell::new(None) };
}

/// Thread-local storage guard for precompiles.
///
/// This guard sets up thread-local access to a storage provider for the duration
/// of its lifetime. When dropped, it cleans up the thread-local storage.
///
/// # IMPORTANT
///
/// The caller must ensure that:
/// 1. Only one `StorageGuard` exists at a time, in the same thread.
/// 2. If multiple storage providers are instantiated in parallel threads,
///    they CANNOT point to the same storage addresses.
#[derive(Default)]
pub struct StorageGuard<'s> {
    _storage: PhantomData<&'s mut dyn PrecompileStorageProvider>,
}

impl<'s> StorageGuard<'s> {
    /// Creates a new storage guard, initializing thread-local storage.
    /// See type-level documentation for important notes.
    pub fn new(storage: &'s mut dyn PrecompileStorageProvider) -> Result<Self> {
        if STORAGE.with(|s| s.get()).is_some() {
            return Err(TempoPrecompileError::Fatal(
                "'StorageGuard' already initialized".to_string(),
            ));
        }

        // SAFETY: Transmuting lifetime to 'static for `Cell` storage.
        //
        // This is safe because:
        // 1. Type system ensures this guard can't outlive 's
        // 2. The Drop impl clears the thread-local before the guard is destroyed
        // 3. Only one guard can exist per thread (checked above)
        let ptr: *mut dyn PrecompileStorageProvider = storage;
        let ptr_static: *mut (dyn PrecompileStorageProvider + 'static) =
            unsafe { std::mem::transmute(ptr) };

        STORAGE.with(|s| s.set(Some(ptr_static)));

        Ok(Self::default())
    }
}

impl Drop for StorageGuard<'_> {
    fn drop(&mut self) {
        STORAGE.with(|s| s.set(None));
    }
}

/// Execute a function with access to the current thread-local storage provider.
pub fn with_storage<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&mut dyn PrecompileStorageProvider) -> Result<R>,
{
    let storage_ptr = STORAGE
        .with(|s| s.get())
        .ok_or(TempoPrecompileError::Fatal(
            "No storage context. 'StorageGuard' must be initialized".to_string(),
        ))?;

    // SAFETY:
    // - Caller must ensure NO recursive calls.
    // - Type system ensures the storage pointer is valid.
    let storage = unsafe { &mut *storage_ptr };
    f(storage)
}
