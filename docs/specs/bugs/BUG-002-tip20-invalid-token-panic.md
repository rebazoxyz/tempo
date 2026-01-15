# BUG-002: TIP20 Invalid Token Causes EVM Panic

**Severity**: Critical  
**Status**: Open  
**Found By**: Invariant Fuzzer  
**Date**: 2026-01-15  
**Component**: tempo-foundry / tempo-revm

---

## Summary

The Tempo EVM handler panics (crashes) when encountering an invalid TIP20 token during fee validation. This should return a graceful error, not crash the entire process.

---

## Expected Behavior

When a transaction references an invalid fee token, the EVM should:
1. Validate the token address
2. Return `Err(InvalidFeeToken)` or similar
3. Reject the transaction gracefully
4. Continue processing other transactions

---

## Actual Behavior

The EVM panics with message:
```
TIP20 prefix already validated: TIP20(InvalidToken(InvalidToken))
```

This crashes the entire test runner / node process.

---

## Stack Trace

```
Location: crates/revm/src/handler.rs:1356

Key frames:
15: expect<tempo_precompiles::tip20::TIP20Token, tempo_precompiles::error::TempoPrecompileError>
16: get_token_balance<...>
17: validate_against_state_and_deduct_caller<...>
18: pre_execution<tempo_revm::handler::TempoEvmHandler<...>>
```

---

## Root Cause

In `tempo-foundry/crates/tempo/crates/revm/src/handler.rs:1356`, there's an `.expect()` call that panics when `get_token_balance` encounters an invalid TIP20 token:

```rust
// Likely code pattern (inferred from stack trace)
let balance = get_token_balance(token_address).expect("TIP20 prefix already validated");
```

The expectation is that token validation happened earlier, but the fuzzer found a code path where validation was bypassed or insufficient.

---

## Reproduction

The crash occurred during invariant testing with the following handlers:
- `handler_invalidFeeToken` - attempts to use a non-TIP20 address as fee token
- Potentially other handlers that manipulate fee token addresses

### Reproduction Steps

```bash
cd docs/specs
rm -rf cache/invariant/failures
FOUNDRY_INVARIANT_RUNS=5 FOUNDRY_INVARIANT_DEPTH=20 ./tempo-forge test --match-contract TempoTransactionInvariant
```

---

## Impact

1. **DoS Vector**: Attackers can crash nodes by submitting malformed transactions
2. **Consensus Risk**: If only some nodes crash, chain can fork
3. **Test Instability**: Invariant tests cannot complete due to panic

---

## Recommended Fix

### Option 1: Replace expect() with proper error handling

```rust
// Before (panics)
let balance = get_token_balance(token_address)
    .expect("TIP20 prefix already validated");

// After (returns error)
let balance = get_token_balance(token_address)
    .map_err(|e| TempoInvalidTransaction::InvalidFeeToken)?;
```

### Option 2: Add early validation

Ensure token validation happens before any `.expect()` calls:

```rust
fn validate_against_state_and_deduct_caller(...) {
    // Early validation
    if !is_valid_tip20_token(fee_token) {
        return Err(TempoInvalidTransaction::InvalidFeeToken);
    }
    
    // Now safe to expect
    let balance = get_token_balance(fee_token).expect("validated above");
}
```

---

## Files to Review

- `tempo-foundry/crates/tempo/crates/revm/src/handler.rs` (line 1356)
- `tempo-foundry/crates/tempo/crates/precompiles/src/tip20.rs`
- Look for all `.expect()` and `.unwrap()` calls in transaction validation path

---

## Verification

After fix:
1. The test should complete without panicking
2. Invalid fee tokens should be rejected with `InvalidFeeToken` error
3. `ghost_invalidFeeTokenRejected` counter should increment

```bash
./tempo-forge test --match-contract TempoTransactionInvariant
```

---

## References

- Stack trace from invariant test run
- Related test handler: `handler_invalidFeeToken`
- Test file: `test/TempoTransactionInvariant.t.sol`

---

## Timeline

| Date | Event |
|------|-------|
| 2026-01-15 | Bug discovered during invariant fuzzing |
| 2026-01-15 | Bug report created |
| TBD | Fix implemented |
| TBD | Fix verified |
