# Tempo ↔ Ethereum Trustless Bridge

A trustless bridge implementation inspired by [cosmos/solidity-ibc-eureka](https://github.com/cosmos/solidity-ibc-eureka) for cross-chain communication between Tempo and Ethereum.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Tempo Chain                                     │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────────────┐  │
│  │  TokenBridge    │───▶│     Bridge      │───▶│  EthereumLightClient    │  │
│  │  (ICS20-like)   │    │  (IBC Router)   │    │  (Beacon Chain Sync)    │  │
│  └─────────────────┘    └─────────────────┘    └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ Relayer submits packets + proofs
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                             Ethereum Chain                                   │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────────────┐  │
│  │  TokenBridge    │───▶│     Bridge      │───▶│   TempoLightClient      │  │
│  │  (ICS20-like)   │    │  (IBC Router)   │    │  (BLS Finalization)     │  │
│  └─────────────────┘    └─────────────────┘    └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Contracts

### Core Interfaces

- **[ILightClient](src/interfaces/ILightClient.sol)** - Interface for light client implementations
- **[IBridge](src/interfaces/IBridge.sol)** - Interface for the bridge router

### Light Clients

- **[TempoLightClient](src/light-clients/TempoLightClient.sol)** - Deployed on Ethereum to verify Tempo state
  - Verifies BLS threshold signatures from Tempo's finalization certificates
  - Uses `consensus_getFinalization` RPC data
  - Storage merkle proofs for membership verification

- **[EthereumLightClient](src/light-clients/EthereumLightClient.sol)** - Deployed on Tempo to verify Ethereum state
  - Implements beacon chain sync committee protocol
  - Merkle-Patricia trie proof verification
  - Tracks execution layer state roots

### Bridge Router

- **[Bridge](src/Bridge.sol)** - Core IBC-inspired packet router
  - Manages light client registrations
  - Handles packet lifecycle: send → recv → ack/timeout
  - Stores packet commitments as keccak256 hashes
  - Verifies proofs through registered light clients

### Applications

- **[TokenBridge](src/TokenBridge.sol)** - ICS20-inspired token transfer
  - Lock tokens on source chain, mint wrapped on destination
  - Burn wrapped tokens to unlock on origin chain
  - Denomination tracking for canonical vs wrapped tokens
  - Automatic refunds on timeout or failure

## Packet Lifecycle

```
Source Chain                    Relayer                    Destination Chain
     │                            │                              │
     │  sendPacket()              │                              │
     │──────────────▶             │                              │
     │  (stores commitment)       │                              │
     │                            │                              │
     │  PacketSent event          │                              │
     │  ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─▶│                              │
     │                            │  recvPacket(proof)           │
     │                            │─────────────────────────────▶│
     │                            │  (verifies proof via         │
     │                            │   light client)              │
     │                            │                              │
     │                            │  PacketReceived event        │
     │                            │◀─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─│
     │                            │                              │
     │  acknowledgePacket(proof)  │                              │
     │◀───────────────────────────│                              │
     │  (clears commitment)       │                              │
     │                            │                              │
```

## Security Model

### Trust Assumptions

1. **Tempo → Ethereum**: Trust that 2/3+ of Tempo validators correctly sign finalization certificates
2. **Ethereum → Tempo**: Trust Ethereum's proof-of-stake consensus and sync committee signatures
3. **Relayers**: Untrusted - they can only submit valid proofs, cannot forge state

### Verification

- All state transitions are verified through light client proofs
- Packet commitments are stored on-chain and verified via merkle proofs
- No admin keys can bypass proof verification

## Development

### Prerequisites

- [Foundry](https://book.getfoundry.sh/getting-started/installation)
- Solidity 0.8.24+

### Build

```bash
forge build
```

### Test

```bash
forge test
```

### Deploy

```bash
# Deploy to Ethereum
forge script script/Deploy.s.sol --rpc-url $ETH_RPC_URL --broadcast

# Deploy to Tempo
forge script script/Deploy.s.sol --rpc-url $TEMPO_RPC_URL --broadcast
```

## Integration

### Sending Tokens

```solidity
// Approve tokens first
IERC20(token).approve(address(tokenBridge), amount);

// Transfer to Tempo
uint64 sequence = tokenBridge.transfer(
    "tempo-mainnet",      // destClient
    token,                // token address
    amount,               // amount
    receiver,             // receiver on Tempo
    block.timestamp + 1 hours, // timeout
    ""                    // memo
);
```

### Relayer Integration

Relayers need to:

1. Watch for `PacketSent` events
2. Wait for source chain finality
3. Generate merkle proofs for packet commitments
4. Submit `recvPacket` on destination chain
5. Watch for acknowledgements and relay back

## License

MIT
