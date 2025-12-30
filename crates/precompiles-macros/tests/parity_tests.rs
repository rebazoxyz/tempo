//! Parity tests comparing `#[solidity]` macro output against `sol!` macro.
//!
//! These tests verify that `#[solidity]` produces identical ABI behavior to alloy's `sol!`
//! for regression detection and confidence in codegen correctness.

mod parity;
