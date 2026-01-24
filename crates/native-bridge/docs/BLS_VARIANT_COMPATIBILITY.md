# BLS12-381 Variant Compatibility: Consensus vs Bridge

## Executive Summary

**The Tempo consensus system and native bridge use incompatible BLS12-381 variants.**

- **Consensus**: Uses `MinSig` (G2 public keys, G1 signatures)
- **Bridge**: Uses `MinPk` (G1 public keys, G2 signatures)

This document explains why the same DKG shares cannot be directly reused, and presents solution options.

---

## Table of Contents

1. [Background: BLS12-381 Variants](#background-bls12-381-variants)
2. [Current Implementation Analysis](#current-implementation-analysis)
3. [The Incompatibility Problem](#the-incompatibility-problem)
4. [Solution Options](#solution-options)
5. [Recommended Approach](#recommended-approach)
6. [Implementation Guide](#implementation-guide)

---

## Background: BLS12-381 Variants

### BLS Signature Scheme Overview

BLS signatures use bilinear pairings on elliptic curves. BLS12-381 provides two groups:

| Group | Point Size (Compressed) | Point Size (Uncompressed) | Operations |
|-------|------------------------|---------------------------|------------|
| **G1** | 48 bytes | 128 bytes (EIP-2537) | Faster, smaller |
| **G2** | 96 bytes | 256 bytes (EIP-2537) | Slower, larger |

The signature scheme requires assigning public keys to one group and signatures to the other.

### MinPk vs MinSig Variants

| Variant | Public Key | Signature | Hash-to-Curve Target | Use Case |
|---------|-----------|-----------|---------------------|----------|
| **MinPk** | G1 (48B) | G2 (96B) | G2 | Minimize public key size |
| **MinSig** | G2 (96B) | G1 (48B) | G1 | Minimize signature size |

#### MinPk (Minimum Public Key Size)
```
Public Key:  pk ∈ G1  (48 bytes compressed)
Signature:   σ  ∈ G2  (96 bytes compressed)
Hash:        H(m) → G2
Verification: e(pk, H(m)) == e(G1_generator, σ)
```

#### MinSig (Minimum Signature Size)
```
Public Key:  pk ∈ G2  (96 bytes compressed)
Signature:   σ  ∈ G1  (48 bytes compressed)
Hash:        H(m) → G1
Verification: e(H(m), pk) == e(σ, G2_generator)
```

### Domain Separation Tags (DST)

The DST determines which curve the message is hashed to:

| Variant | DST Suffix | Target Curve |
|---------|-----------|--------------|
| MinPk | `BLS12381G2_XMD:SHA-256_SSWU_RO_` | G2 |
| MinSig | `BLS12381G1_XMD:SHA-256_SSWU_RO_` | G1 |

---

## Current Implementation Analysis

### Consensus System (MinSig)

**Location**: `crates/commonware-node/src/`

```rust
// crates/commonware-node/src/subblocks.rs
use commonware_cryptography::bls12381::primitives::variant::MinSig;

// crates/commonware-node/src/dkg/manager/actor/mod.rs
if share.public::<MinSig>() != partial {
    // Verification uses MinSig variant
}
```

The DKG ceremony produces:
- `Sharing<MinSig>` with polynomial coefficients on **G2**
- Group public key: **G2 point** (96 bytes compressed)
- Each validator's partial public key: **G2 point**
- Signatures: **G1 points** (48 bytes compressed)

### Bridge System (MinPk)

**Location**: `crates/native-bridge/src/`

```rust
// crates/native-bridge/src/signer.rs
use commonware_cryptography::bls12381::primitives::variant::MinPk;

pub fn sign_partial(&self, attestation_hash: B256) -> Result<PartialSignature> {
    let signature: G2 = sign::<MinPk>(&self.share.private, BLS_DST, ...);
    // Returns G2 signature (96 bytes)
}

pub fn public_key(&self) -> G1 {
    self.share.public::<MinPk>()  // Returns G1 public key
}
```

**Location**: `crates/native-bridge/src/message.rs`

```rust
pub const BLS_DST: &[u8] = b"TEMPO_BRIDGE_BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_";
//                                                      ^^
//                                          Hashes to G2 (MinPk style)
```

### On-Chain Contract (MinPk)

**Location**: `crates/native-bridge/contracts/src/BLS12381.sol`

```solidity
uint256 internal constant G1_POINT_LENGTH = 128;  // Public key
uint256 internal constant G2_POINT_LENGTH = 256;  // Signature

// Pairing check: e(pk, H(m)) * e(-G1, sig) == 1
bytes memory input = abi.encodePacked(
    publicKey,          // G1: 128 bytes
    hashedMessage,      // G2: 256 bytes (H(m) on G2)
    NEG_G1_GENERATOR,   // G1: 128 bytes
    signature           // G2: 256 bytes
);
```

---

## The Incompatibility Problem

### Problem 1: Different Public Key Curves

The same private scalar produces different public keys depending on the variant:

```rust
let private: Private = /* scalar from DKG */;

// MinSig: public key on G2
let pk_minsig: G2 = private * G2_GENERATOR;  // 96 bytes

// MinPk: public key on G1
let pk_minpk: G1 = private * G1_GENERATOR;   // 48 bytes
```

**These are mathematically related but live on different curves.**

### Problem 2: DKG Polynomial Curve Mismatch

The DKG ceremony produces a polynomial whose coefficients live on the public key curve:

| DKG Variant | Polynomial Coefficients | Group Public Key |
|-------------|------------------------|------------------|
| `Sharing<MinSig>` | `Poly<G2>` | G2 point |
| `Sharing<MinPk>` | `Poly<G1>` | G1 point |

The consensus DKG produces `Sharing<MinSig>`:
- Group public key: `Σ coefficients[0]` → **G2 point**
- Partial verification: `share.public::<MinSig>()` evaluates polynomial on G2

If you try to use this share with MinPk:
- `share.public::<MinPk>()` computes `private * G1_GENERATOR` → **G1 point**
- This G1 point has **no relationship** to the `Sharing<MinSig>` polynomial
- Lagrange interpolation of partial signatures will produce garbage

### Problem 3: Signature Aggregation Failure

Threshold signature aggregation requires:
1. Collect `t` partial signatures from validators
2. Interpolate using Lagrange coefficients
3. Result should equal `group_private_key * H(m)`

With mixed variants:
- Partial signatures would be G2 points (MinPk signing)
- But the group public key is G2 (from MinSig DKG)
- Verification equation: `e(G2_garbage, H(m)) * e(-G1, G2_sig)` → **always fails**

### Problem 4: Contract Expects MinPk Format

The Solidity contract is hardcoded for MinPk:

```solidity
// MessageBridge.sol
bytes public groupPublicKey;  // Expected: 128-byte G1 point

// BLS12381.sol
function pairingCheck(...) {
    // Pair 1: (G1 publicKey, G2 hashedMessage)
    // Pair 2: (G1 negGenerator, G2 signature)
}
```

If you passed the MinSig group public key (G2, 96 bytes compressed → 256 bytes uncompressed):
- Length check fails: `publicKey.length != 128`
- Even if bypassed, pairing input would be malformed

---

## Solution Options

### Option A: Separate DKG for Bridge

**Description**: Run an independent DKG ceremony using MinPk for bridge operations.

**Pros**:
- Clean separation of concerns
- No changes to consensus
- Bridge gets optimal key format for EIP-2537

**Cons**:
- Doubles DKG ceremony complexity
- Two sets of shares to manage per validator
- Increased operational overhead

**Implementation**:
```rust
// New bridge-specific DKG
let (bridge_sharing, bridge_shares) = dkg::deal::<MinPk, N3f1>(...);
```

### Option B: Migrate Consensus to MinPk

**Description**: Change the consensus system to use MinPk throughout.

**Pros**:
- Single DKG, single variant
- Shares work for both consensus and bridge
- Aligns with Ethereum ecosystem (most implementations use MinPk)

**Cons**:
- Breaking change for existing validators
- Requires DKG migration ceremony
- Larger consensus messages (G2 signatures = 96 bytes vs 48 bytes)

**Implementation**:
```rust
// All of commonware-node changes from:
use variant::MinSig;
// To:
use variant::MinPk;
```

### Option C: Modify Bridge for MinSig

**Description**: Update the bridge contract and signer to use MinSig.

**Pros**:
- No consensus changes
- Reuses existing DKG shares

**Cons**:
- Larger on-chain public keys (256 bytes vs 128 bytes)
- Non-standard for Ethereum BLS usage
- More complex contract changes

**Contract Changes Required**:
```solidity
// MessageBridge.sol
uint256 internal constant G1_POINT_LENGTH = 64;   // Signature (was 128)
uint256 internal constant G2_POINT_LENGTH = 256;  // Public key (was for sig)

// BLS12381.sol - swap pairing operands
function pairingCheck(...) {
    // New: Pair 1: (G1 signature, G2 hashedMessage) 
    //      Pair 2: (G1 negGenerator, G2 publicKey)
}

// Change hash-to-curve to target G1
bytes public constant BLS_DST = "TEMPO_BRIDGE_BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_";
```

### Option D: Dual Public Key Derivation

**Description**: The private scalar is variant-agnostic. Derive both MinSig (for consensus) and MinPk (for bridge) public keys from the same scalar.

**Pros**:
- Single DKG ceremony
- Single share file per validator
- Each system uses its preferred variant

**Cons**:
- Requires computing MinPk group public key separately
- Bridge group key ≠ consensus group key (different curves)
- Need to track/publish both group public keys

**Mathematical Basis**:
```
Given: private scalar s from DKG

Consensus (MinSig):
  pk_consensus = s * G2_generator  ∈ G2
  
Bridge (MinPk):
  pk_bridge = s * G1_generator  ∈ G1

Both are valid public keys for the SAME private key.
```

**Implementation**:
```rust
// Share contains the private scalar
let share: Share = load_from_dkg();

// For consensus (existing)
let consensus_pk: G2 = share.public::<MinSig>();

// For bridge (new derivation)  
let bridge_pk: G1 = share.public::<MinPk>();
```

**Challenge**: The group public key for the bridge must be computed as:
```
group_pk_bridge = Σ (lagrange_i * share_i.public::<MinPk>())
```

This is NOT the same as the consensus group public key. The bridge contract must store this separately derived G1 group key.

---

## Recommended Approach

### For Immediate Compatibility: Option D (Dual Derivation)

This approach allows reusing consensus DKG shares while keeping both systems functional.

#### Architecture

```
                    ┌─────────────────────────────────────┐
                    │         DKG Ceremony                │
                    │    (produces Share with scalar s)   │
                    └─────────────────┬───────────────────┘
                                      │
                    ┌─────────────────┴───────────────────┐
                    │                                     │
                    ▼                                     ▼
        ┌───────────────────────┐           ┌───────────────────────┐
        │   Consensus (MinSig)  │           │    Bridge (MinPk)     │
        ├───────────────────────┤           ├───────────────────────┤
        │ pk = s * G2_gen       │           │ pk = s * G1_gen       │
        │ sig ∈ G1 (48 bytes)   │           │ sig ∈ G2 (96 bytes)   │
        │ H(m) → G1             │           │ H(m) → G2             │
        └───────────────────────┘           └───────────────────────┘
                    │                                     │
                    ▼                                     ▼
        ┌───────────────────────┐           ┌───────────────────────┐
        │  Group PK (G2, 96B)   │           │  Group PK (G1, 48B)   │
        │  Stored in consensus  │           │  Stored in contract   │
        └───────────────────────┘           └───────────────────────┘
```

#### Key Insight

The `Share` struct contains a `Private` scalar that is curve-agnostic:

```rust
pub struct Share {
    pub index: Participant,
    pub private: Private,  // Scalar, works with any variant
}
```

Calling `share.public::<V>()` computes `private * V::generator()`, which produces:
- G2 point for `MinSig`
- G1 point for `MinPk`

Both are valid public keys for the same private key.

---

## Implementation Guide

### Step 1: Compute Bridge Group Public Key

The bridge needs its own group public key (G1), derived from the same DKG output.

```rust
// In bridge initialization or key rotation
use commonware_cryptography::bls12381::primitives::{
    ops::aggregate_public_keys,
    variant::MinPk,
};

/// Compute the MinPk group public key from MinSig DKG shares.
/// 
/// This must be done by collecting all validators' MinPk partial public keys
/// and combining them (NOT by converting the MinSig group key).
pub fn compute_bridge_group_key(
    sharing: &Sharing<MinSig>,
    shares: &[Share],
) -> G1 {
    // Extract the MinPk public keys from each share
    let minpk_publics: Vec<G1> = shares
        .iter()
        .map(|s| s.public::<MinPk>())
        .collect();
    
    // Aggregate using Lagrange interpolation at x=0
    // This gives us the group public key on G1
    aggregate_public_keys_at_zero::<MinPk>(&minpk_publics, &indices)
}
```

**Important**: You cannot simply convert a G2 point to G1. The group public key must be recomputed from individual shares.

### Step 2: Modify BLSSigner

Update the signer to work with consensus shares:

```rust
// crates/native-bridge/src/signer.rs

impl BLSSigner {
    /// Create from a consensus DKG share.
    /// 
    /// The share's private scalar works for both MinSig (consensus) and 
    /// MinPk (bridge) variants. We use MinPk for bridge signing.
    pub fn from_consensus_share(share: Share) -> Self {
        Self {
            validator_index: share.index.get(),
            share,  // Same share, different variant usage
        }
    }
    
    /// Sign using MinPk variant (G2 signature).
    /// 
    /// Even though the share came from a MinSig DKG, the private scalar
    /// works identically for MinPk signing.
    pub fn sign_partial(&self, attestation_hash: B256) -> Result<PartialSignature> {
        // This is correct: sign::<MinPk> uses the scalar to produce G2 sig
        let signature: G2 = sign::<MinPk>(
            &self.share.private, 
            BLS_DST,  // G2 DST
            attestation_hash.as_slice()
        );
        // ... rest unchanged
    }
    
    /// Get the bridge public key (G1) for this validator.
    pub fn bridge_public_key(&self) -> G1 {
        self.share.public::<MinPk>()  // G1 point
    }
    
    /// Get the consensus public key (G2) for this validator.
    pub fn consensus_public_key(&self) -> G2 {
        self.share.public::<MinSig>()  // G2 point
    }
}
```

### Step 3: Aggregator Changes

The signature aggregator must use MinPk Lagrange interpolation:

```rust
// crates/native-bridge/src/aggregator.rs (or similar)

use commonware_cryptography::bls12381::primitives::{
    ops::aggregate_partial_signatures,
    variant::MinPk,
};

/// Aggregate partial signatures into a threshold signature.
pub fn aggregate_bridge_signatures(
    partials: &[(Participant, G2)],  // G2 partial sigs (MinPk)
    threshold: u32,
) -> Result<G2> {
    // Use MinPk aggregation (operates on G2 signatures)
    aggregate_partial_signatures::<MinPk>(partials, threshold)
}
```

### Step 4: Bridge Group Key Initialization

When deploying the bridge contract or rotating keys:

```rust
/// Compute the bridge group public key from validator shares.
/// 
/// This must be called during bridge setup, collecting MinPk public keys
/// from all validators.
pub fn initialize_bridge_group_key(
    validator_shares: &[Share],
    threshold: u32,
) -> [u8; G1_COMPRESSED_LEN] {
    // Get MinPk public keys from each share
    let publics: Vec<(Participant, G1)> = validator_shares
        .iter()
        .map(|s| (s.index, s.public::<MinPk>()))
        .collect();
    
    // Compute group key using Lagrange interpolation
    // For a (t, n) threshold scheme, we need the constant term of the polynomial
    let group_key = compute_group_public_key::<MinPk>(&publics);
    
    // Serialize for contract
    group_key.encode().try_into().unwrap()
}
```

### Step 5: Contract Deployment

The contract must be initialized with the MinPk group public key:

```solidity
// Deploy with MinPk group key (128 bytes uncompressed)
MessageBridge bridge = new MessageBridge(
    owner,
    initialEpoch,
    bridgeGroupPublicKey  // G1 point, NOT the consensus G2 key
);
```

### Step 6: Key Rotation Coordination

When the validator set changes:

1. **Consensus DKG** produces new `Sharing<MinSig>` and shares
2. **Bridge coordinator** collects MinPk public keys from all new validators
3. **Compute new bridge group key**: Aggregate MinPk publics → new G1 group key
4. **Rotate on-chain**: Call `rotateKey()` with new G1 group public key
5. **Validators update**: Each validator's signer uses their new share

---

## Verification Checklist

Before deployment, verify:

- [ ] Bridge group public key is G1 (48 bytes compressed, 128 bytes EIP-2537)
- [ ] Bridge signatures are G2 (96 bytes compressed, 256 bytes EIP-2537)
- [ ] DST is `TEMPO_BRIDGE_BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_`
- [ ] Contract pairing check matches MinPk equation
- [ ] Aggregator uses MinPk Lagrange interpolation
- [ ] Group key derivation uses MinPk variant
- [ ] Test end-to-end: sign → aggregate → verify on-chain

---

## Appendix: Mathematical Details

### Why the Same Scalar Works for Both Variants

The private key in BLS is a scalar `s ∈ F_r` (the scalar field of BLS12-381).

For MinSig:
```
pk = s · G2_generator ∈ G2
sig = s · H(m) where H(m) ∈ G1
```

For MinPk:
```
pk = s · G1_generator ∈ G1
sig = s · H(m) where H(m) ∈ G2
```

The same scalar `s` can be used with either variant because:
1. Scalar multiplication is defined for both G1 and G2
2. The pairing `e: G1 × G2 → GT` is bilinear
3. Verification only requires `e(pk, H(m)) = e(generator, sig)`

### Threshold Signature Compatibility

For threshold signatures, each validator has share `s_i` such that:
```
s = Σ λ_i · s_i  (Lagrange interpolation)
```

This works identically regardless of which group we use:
```
MinSig: group_pk = Σ λ_i · (s_i · G2_gen) = s · G2_gen
MinPk:  group_pk = Σ λ_i · (s_i · G1_gen) = s · G1_gen
```

The shares `s_i` are the same; only the derived public keys differ.

---

## References

- [EIP-2537: BLS12-381 Precompiles](https://eips.ethereum.org/EIPS/eip-2537)
- [RFC 9380: Hashing to Elliptic Curves](https://www.rfc-editor.org/rfc/rfc9380)
- [BLS Signatures Draft](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-bls-signature)
- [Commonware Cryptography Library](https://github.com/commonwarexyz/monorepo)
