//! Result type for `#[abi]` trait methods.
//!
//! This module provides a `Result` type alias for use in `#[abi]`-generated traits.
//!
//! When the `precompile` feature is enabled, this is intended to be shadowed by
//! the consuming crate's own `Result` type (e.g., `tempo_precompiles::error::Result`).
//!
//! When `precompile` is disabled, this provides a placeholder that allows the
//! ABI types to compile without the full precompile runtime.

use core::convert::Infallible;

/// Placeholder result type for `#[abi]` trait methods.
///
/// This type is used by the `#[abi]` macro to generate trait signatures.
/// In the precompile runtime, this should be shadowed by importing
/// `crate::error::Result` which uses `TempoPrecompileError`.
pub type Result<T> = core::result::Result<T, Infallible>;
