# Native Bridge E2E Testing

## Running Tests

```bash
# Build Solidity contracts first (required for bytecode)
cd crates/native-bridge/contracts && forge build && cd ../../..

# Run all native-bridge tests
cargo test -p tempo-native-bridge

# Run only E2E tests
cargo test -p tempo-native-bridge --test bridge_e2e

# Run with debug output
cargo test -p tempo-native-bridge --test bridge_e2e -- --nocapture
```

**Prerequisites:**
- Anvil installed (`foundryup` to install Foundry)
- Solidity contracts compiled (`forge build` in `contracts/`)

---

## Current Test Coverage

### Unit Tests

| Module | Tests | Description |
|--------|-------|-------------|
| `signer.rs` | ✅ | BLS partial signing with MinSig variant |
| `aggregator.rs` | ✅ | Threshold signature aggregation (4-of-5) |
| `eip2537.rs` | ✅ | G1/G2 compressed → EIP-2537 format conversion |
| `message.rs` | ✅ | Attestation hash computation |

### E2E Tests (`tests/bridge_e2e.rs`)

| Test | Description |
|------|-------------|
| `test_anvil_event_subscription` | WebSocket `eth_subscribe` on Anvil (Prague hardfork) |
| `test_anvil_polling_fallback` | HTTP `eth_getLogs` polling on Anvil |
| `test_tempo_event_subscription` | WebSocket event subscription on in-process Tempo node |
| `test_tempo_polling_fallback` | HTTP polling on in-process Tempo node |
| `test_full_bridge_flow_ethereum_to_tempo` | **Complete cross-chain flow** (see below) |

### Full Bridge Flow Test

The `test_full_bridge_flow_ethereum_to_tempo` test verifies the complete Ethereum → Tempo message bridge:

1. **DKG Key Generation** - Creates 5 shares with threshold 4 (MinSig: G2 pubkeys, G1 sigs)
2. **Node Startup** - Starts Anvil (Prague hardfork) and in-process Tempo node
3. **Contract Deployment** - Deploys `MessageBridge.sol` on both chains with same G2 public key
4. **Message Send** - Calls `send(messageHash, destinationChainId)` on Ethereum
5. **Threshold Signing** - 4 signers create partial G1 signatures
6. **Aggregation** - Recovers threshold signature via Lagrange interpolation
7. **EIP-2537 Conversion** - Converts compressed G1 (48 bytes) → EIP-2537 (128 bytes)
8. **Cross-chain Submission** - Calls `write(sender, messageHash, originChainId, signature)` on Tempo
9. **Verification** - Confirms `MessageReceived` event and `receivedAt()` returns non-zero timestamp

All cryptographic operations use **real BLS12-381 keys and signatures** - no mocks.

---

## Components Tested

### Rust Sidecar

| Component | File | Status |
|-----------|------|--------|
| ChainWatcher | `sidecar/watcher.rs` | ✅ Tested (WebSocket + polling) |
| BLSSigner | `signer.rs` | ✅ Tested (MinSig partial signing) |
| Aggregator | `sidecar/aggregator.rs` | ✅ Tested (threshold recovery) |
| Submitter | `sidecar/submitter.rs` | ✅ Tested (via E2E flow) |
| EIP-2537 conversion | `eip2537.rs` | ✅ Tested (G1/G2 format conversion) |

### Solidity Contracts

| Contract | File | Status |
|----------|------|--------|
| MessageBridge | `contracts/src/MessageBridge.sol` | ✅ Tested (send, write, key rotation) |
| BLS12381 | `contracts/src/BLS12381.sol` | ✅ Tested (signature verification) |
| IMessageBridge | `contracts/src/interfaces/IMessageBridge.sol` | ✅ Interface only |

---

## What's Left to Build/Test

### P2P Gossip Layer (Not Started)

The current implementation adds partial signatures directly to the local aggregator. Production requires:

- [ ] **P2P partial signature broadcast** - Gossip partials to other validators
- [ ] **Partial signature validation** - Verify incoming partials before aggregation
- [ ] **Deduplication** - Handle duplicate partials from retransmits

```rust
// Current (in sidecar/mod.rs):
// TODO: Broadcast partial via P2P gossip
// For now, just add to local aggregator
```

### Multi-Validator E2E Test

- [ ] **Distributed signing test** - Multiple sidecar instances coordinating
- [ ] Test with 5 validators, verify only 4 needed for threshold

### Key Rotation E2E Test

- [ ] **`rotateKey()` flow** - Old validators sign authorization for new key
- [ ] **Grace period** - Verify messages signed with old key still accepted
- [ ] **Epoch transitions** - Test key rotation during active message flow

### Bidirectional Bridge Test

- [ ] **Tempo → Ethereum flow** - Currently only Ethereum → Tempo tested
- [ ] Test `send()` on Tempo, `write()` on Anvil

### Finality Handling

- [ ] **Reorg protection** - Test `finality_blocks` config in watcher
- [ ] **Block confirmation** - Verify messages from non-finalized blocks are delayed

### Error Cases

- [ ] **Invalid signature rejection** - Wrong signer / corrupted signature
- [ ] **Replay protection** - Same message cannot be written twice
- [ ] **Paused contract** - Verify `whenNotPaused` modifier works
- [ ] **Unauthorized key rotation** - Reject rotation with invalid signature

### Metrics & Observability

- [ ] **Prometheus metrics** - Expose signing latency, success rate
- [ ] **Structured logging** - Trace IDs for cross-chain message tracking

### Production Deployment

- [ ] **Config file validation** - TOML config loading tests
- [ ] **Private key management** - Integration with secret managers
- [ ] **Gas estimation** - Verify gas limits for `write()` transactions

---

## Test Infrastructure

### Anvil Configuration

E2E tests start Anvil with Prague hardfork for EIP-2537 BLS precompiles:

```bash
anvil --hardfork prague --block-time 1
```

### Tempo Node

Tests use `TempoNode` from `tempo-node` crate for in-process node without Docker.

### Contract Deployment

Uses real `MessageBridge.bytecode.hex` compiled from Solidity:

```
crates/native-bridge/contracts/out/MessageBridge.sol/MessageBridge.bytecode.hex
```
