# BUG-001: Oversized Initcode Not Rejected (C8 Violation)

**Severity**: High  
**Status**: Open  
**Found By**: Invariant Fuzzer  
**Date**: 2026-01-15  
**Component**: Tempo Transaction Handler (EVM)

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

The Tempo transaction handler in `tempo-foundry` (and likely the main `tempo` node) does not validate initcode size before executing CREATE operations.

### Affected Code Paths

1. **Tempo Transaction Validation** (`tempo-foundry/crates/tempo/src/...`)
   - Missing check: `if initcode.len() > MAX_INITCODE_SIZE { return Err(...) }`

2. **CREATE Call Execution**
   - The EVM's CREATE opcode handler may also need size validation

---

## Impact

1. **DoS Vector**: Attackers can submit transactions with arbitrarily large initcode, consuming excessive resources
2. **Gas Metering**: EIP-3860 introduced `INITCODE_WORD_COST` (2 gas per 32-byte word) which may not be applied correctly
3. **Consensus Divergence**: If other nodes enforce the limit, this could cause chain splits

---

## Recommended Fix

### Option 1: Transaction Validation Layer

Add size check during Tempo transaction validation:

```rust
// In tempo transaction handler
const MAX_INITCODE_SIZE: usize = 49152; // 2 * 24576

fn validate_tempo_transaction(tx: &TempoTransaction) -> Result<(), Error> {
    for call in &tx.calls {
        if call.to.is_zero() {
            // This is a CREATE operation
            if call.data.len() > MAX_INITCODE_SIZE {
                return Err(Error::InitcodeTooLarge {
                    size: call.data.len(),
                    max: MAX_INITCODE_SIZE,
                });
            }
        }
    }
    Ok(())
}
```

### Option 2: EVM Layer

Ensure the EVM's CREATE handler validates initcode size:

```rust
fn create(&mut self, initcode: &[u8], ...) -> Result<Address, Error> {
    if initcode.len() > MAX_INITCODE_SIZE {
        return Err(Error::InitcodeTooLarge);
    }
    // ... rest of CREATE logic
}
```

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
| TBD | Fix implemented |
| TBD | Fix verified |
