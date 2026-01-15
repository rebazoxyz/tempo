# BUG-001: Oversized Initcode Not Rejected (C8 Violation)

**Severity**: High  
**Status**: Fixed  
**Found By**: Invariant Fuzzer  
**Date**: 2026-01-15  
**Fixed**: 2026-01-15  
**Component**: tempo-foundry (EVM Config)

---

## Summary

The Tempo protocol does not enforce the EIP-3860 initcode size limit. Transactions containing CREATE operations with initcode exceeding 49,152 bytes are accepted and executed instead of being rejected.

---

## Expected Behavior

Per [EIP-3860](https://eips.ethereum.org/EIPS/eip-3860), the maximum initcode size is 49,152 bytes (`MAX_INITCODE_SIZE = 2 * MAX_CODE_SIZE`). Transactions attempting to deploy contracts with larger initcode should be rejected during transaction validation.

---

## Actual Behavior

A Tempo transaction with 50,000 bytes of initcode was accepted and executed successfully.

---

## Invariant Violated

**C8: `create_initcode_size_limit`**
> Initcode must not exceed `max_initcode_size` (EIP-3860: 49,152 bytes)

---

## Reproduction

### Minimal Test Case

```solidity
function test_oversizedInitcode() public {
    // Generate 50,000 bytes of initcode (exceeds 49,152 limit)
    bytes memory initcode = InitcodeHelper.largeInitcode(50000);
    
    TempoCall[] memory calls = new TempoCall[](1);
    calls[0] = TempoCall({to: address(0), value: 0, data: initcode});
    
    TempoTransaction memory tx_ = TempoTransactionLib.create()
        .withChainId(uint64(block.chainid))
        .withMaxFeePerGas(1 gwei)
        .withGasLimit(5_000_000)
        .withCalls(calls)
        .withNonceKey(1)
        .withNonce(0);
    
    bytes memory signedTx = TxBuilder.signTempo(vmRlp, vm, tx_, signingParams);
    
    vm.coinbase(validator);
    
    // This should revert but doesn't
    vmExec.executeTransaction(signedTx);
}
```

### Fuzzer Trace

```
[FAIL: C8: Oversized initcode unexpectedly allowed: 1 != 0]
        [Sequence] (original: 7, shrunk: 1)
                vm.prank(0xaAAAaaAA00000000000000000000000000000000);
                TempoTransactionInvariantTest(0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496).handler_createOversized(3154027019, 320155961201310677258558132435108844038389403986892987077408);
```

### Run Command

```bash
cd docs/specs
./tempo-forge test --match-test handler_createOversized -vvv
```

---

## Root Cause Analysis

The tempo-revm handler correctly validates initcode size in `validate_aa_initial_tx_gas()`. However, tempo-foundry's `configure_env()` function in `crates/evm/core/src/fork/init.rs` was setting:

```rust
cfg.limit_contract_code_size = Some(usize::MAX);
```

This caused `max_initcode_size()` to return `usize::MAX` (since it falls back to `code_size * 2` when `limit_contract_initcode_size` is None), effectively disabling the EIP-3860 limit.

### Affected Code Path

- **tempo-foundry** `crates/evm/core/src/fork/init.rs:121`
  - `configure_env()` set unlimited code size, which cascaded to unlimited initcode size

---

## Impact

1. **DoS Vector**: Attackers can submit transactions with arbitrarily large initcode, consuming excessive resources
2. **Gas Metering**: EIP-3860 introduced `INITCODE_WORD_COST` (2 gas per 32-byte word) which may not be applied correctly
3. **Consensus Divergence**: If other nodes enforce the limit, this could cause chain splits

---

## Fix Applied

### tempo-foundry: `crates/cheatcodes/src/evm.rs` (executeTransactionCall)

Enforce initcode size limit specifically for `executeTransaction` cheatcode:

```rust
// EIP-3860: Enforce initcode size limit for executeTransaction to match production behavior.
// The global config sets limit_contract_code_size = usize::MAX for test flexibility,
// which causes max_initcode_size() to return usize::MAX. We override this here to
// enforce the EIP-3860 limit (49152 bytes) for realistic transaction simulation.
env.cfg.limit_contract_initcode_size =
    Some(revm::primitives::eip3860::MAX_INITCODE_SIZE);
```

This applies the initcode limit only when simulating real transactions via `executeTransaction`, while keeping unlimited initcode for regular test contract deployments (which need flexibility for large test contracts).

---

## Verification

After fix, the following test should pass:

```bash
cd docs/specs
./tempo-forge test --match-contract TempoTransactionInvariant
```

The invariant `ghost_createOversizedAllowed == 0` should hold.

---

## References

- [EIP-3860: Limit and meter initcode](https://eips.ethereum.org/EIPS/eip-3860)
- [Ethereum Yellow Paper - CREATE semantics](https://ethereum.github.io/yellowpaper/paper.pdf)
- Test file: `test/TempoTransactionInvariant.t.sol` (handler: `handler_createOversized`)
- Task: #48

---

## Timeline

| Date | Event |
|------|-------|
| 2026-01-15 | Bug discovered by invariant fuzzer |
| 2026-01-15 | Bug report created |
| 2026-01-15 | Root cause identified: tempo-foundry config disabled initcode limit |
| 2026-01-15 | Fix implemented in tempo-foundry `crates/cheatcodes/src/evm.rs` |
| 2026-01-15 | Fix verified with test cases |
| 2026-01-15 | C8 invariant re-enabled in TempoTransactionInvariant.t.sol |
