# Native Bridge

## EIP-2537 BLS Precompiles

- EIP-2537 (BLS12-381 precompiles at 0x0b-0x12) is live on Ethereum mainnet since the Pectra hardfork
- Anvil requires `--hardfork prague` flag to enable EIP-2537 precompiles for testing BLS contracts
- G1 generator point for test deployments (128 bytes uncompressed):
  ```
  0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1
  ```

## Testing

- E2E tests should use real contracts, not mocks - the MessageBridge bytecode is at `contracts/out/MessageBridge.sol/MessageBridge.bytecode.hex`
- Run `forge build` in `contracts/` to regenerate bytecode after Solidity changes
