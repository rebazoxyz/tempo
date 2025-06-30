# Reth Helper Type Implementation Proposal

## Overview

This proposal introduces an intermediate "helper" type between ExecutionPayload and Block to improve internal efficiency of payload processing and provide more flexible validation for different blockchain implementations.

**Important Note**: This proposal primarily improves the internal efficiency of the engine API. To completely eliminate the block -> payload -> block roundtrip for external consensus clients (like Malachite), additional API changes would be required.

## Implementation Steps

### 1. Define Core Traits and Types

First, create the helper trait that serves as the intermediate representation:

```rust
// In reth-primitives or reth-payload-primitives
pub trait ExecutionHelper: Send + Sync + 'static {
    type Block: Block;
    type Transaction: Transaction;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Convert to final block form
    fn into_block(self) -> Result<Self::Block, Self::Error>;

    /// Access transactions without full conversion
    fn transactions(&self) -> &[Self::Transaction];

    /// Get encoded payload bytes if available
    fn encoded_payload(&self) -> Option<&Bytes>;
}

/// Default implementation that wraps ExecutionPayload
pub struct DefaultExecutionHelper<P: PayloadTypes> {
    payload: P::ExecutionData,
    encoded: Option<Bytes>,
    // Cache decoded transactions to avoid re-decoding
    transactions: Option<Vec<RecoveredTx>>,
}
```

### 2. Update PayloadValidator Trait

Modify the existing `PayloadValidator` trait to use the helper:

```rust
pub trait PayloadValidator: Send + Sync + 'static {
    type Block: Block;
    type Helper: ExecutionHelper<Block = Self::Block>;

    /// Convert payload to helper (lightweight operation)
    fn payload_to_helper(&self, payload: ExecutionPayload) -> Result<Self::Helper, Error>;

    /// Validate the helper without full block conversion
    fn validate_helper(&self, helper: &Self::Helper) -> Result<(), Error>;

    /// Full validation with block conversion (for when needed)
    fn validate_and_convert(&self, payload: ExecutionPayload) -> Result<Self::Block, Error> {
        let helper = self.payload_to_helper(payload)?;
        self.validate_helper(&helper)?;
        helper.into_block()
    }
}
```

### 3. Create Specialized Implementations

For Ethereum:
```rust
pub struct EthExecutionHelper {
    header: Header,
    transactions: Vec<RecoveredTx>,
    ommers: Vec<Header>,
    withdrawals: Option<Vec<Withdrawal>>,
    requests: Option<Requests>,
    // Keep original payload for efficiency
    original_payload: Option<ExecutionPayload>,
}

impl ExecutionHelper for EthExecutionHelper {
    type Block = Block;
    type Transaction = TransactionSigned;
    type Error = PayloadError;

    fn into_block(self) -> Result<Block, Self::Error> {
        // Efficient conversion using pre-parsed data
        Ok(Block {
            header: self.header,
            body: BlockBody {
                transactions: self.transactions.into_iter().map(|tx| tx.into()).collect(),
                ommers: self.ommers,
                withdrawals: self.withdrawals,
                requests: self.requests,
            }
        })
    }
}
```

For Optimism (showing the benefit):
```rust
pub struct OpExecutionHelper {
    eth_helper: EthExecutionHelper,
    // Optimism-specific: keep encoded for efficiency
    encoded_txs: Vec<Bytes>,
    l1_attributes: Option<L1BlockInfo>,
}
```

### 4. Update Engine API Internal Implementation

The helper type improves the internal implementation of existing methods:

```rust
impl<V: PayloadValidator> EngineApi<V> {
    pub async fn new_payload(&self, payload: ExecutionPayload) -> PayloadStatus {
        // Step 1: Quick conversion to helper (cheap)
        let helper = match self.validator.payload_to_helper(payload) {
            Ok(h) => h,
            Err(e) => return PayloadStatus::invalid(e),
        };

        // Step 2: Pre-validation on helper (no full conversion)
        if let Err(e) = self.validator.validate_helper(&helper) {
            return PayloadStatus::invalid(e);
        }

        // Step 3: Only convert to block when needed for execution
        let block = match helper.into_block() {
            Ok(b) => b,
            Err(e) => return PayloadStatus::invalid(e),
        };

        // Continue with execution...
    }
}
```

### 5. Optional: Extended API for Consensus Clients

To fully eliminate the roundtrip for consensus clients like Malachite, we would need to add new API methods:

```rust
impl EngineApi {
    /// New method that accepts blocks directly from consensus
    pub async fn new_block(&self, block: Block) -> PayloadStatus {
        // Create helper directly from block (no payload conversion)
        let helper = BlockExecutionHelper::from_block(block);
        
        // Validate
        if let Err(e) = self.validator.validate_helper(&helper) {
            return PayloadStatus::invalid(e);
        }
        
        // Execute
        let block = helper.into_block()?;
        self.execute_block(block).await
    }
}
```

Without this API addition, consensus clients would still need to convert blocks to ExecutionPayload format:
```rust
// Without new API: Still requires conversion
let payload = block_to_payload(block);
engine.new_payload(payload).await?;

// With new API: Direct execution
engine.new_block(block).await?;
```

### 6. Migration Strategy

Create compatibility layer:
```rust
/// Adapter for existing code
impl<T: PayloadValidator> PayloadValidatorCompat for T {
    fn validate_payload(&self, payload: ExecutionPayload) -> Result<Block, Error> {
        self.validate_and_convert(payload)
    }
}
```

## What This Proposal Achieves

### Internal Engine Improvements

1. **More efficient payload processing**: The engine can validate payloads without fully converting them to blocks
2. **Lazy deserialization**: Only decode what's needed for validation
3. **Preserved encoded data**: Important for chains like Optimism that need original transaction encodings
4. **Reduced allocations**: The helper type can reference data instead of copying

### Current Limitations

**Without additional API changes**, consensus integrations like Malachite would still need to:
1. Convert their Block to ExecutionPayload format (`block_to_payload`)
2. Send the payload to the engine API
3. The engine would then use the helper internally (more efficient than before, but still a conversion)

The helper type improves step 3, but doesn't eliminate steps 1 and 2.

### Complete Solution

For a complete elimination of the roundtrip, we need:
1. **This proposal**: Helper type for internal efficiency
2. **API extension**: New methods like `new_block` that accept blocks directly
3. **Updated PayloadTypes**: Methods to create helpers directly from blocks

## Key Benefits

1. **Avoids full deserialization**: Helper can lazily decode only what's needed
2. **Preserves encoded data**: Useful for Optimism and other chains
3. **Flexible validation**: Can validate without full block construction
4. **Better performance**: Reduces allocations and conversions
5. **Extensible**: Different chains can have specialized helpers

