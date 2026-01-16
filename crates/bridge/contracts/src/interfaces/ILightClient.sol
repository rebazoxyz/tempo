// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ILightClient
/// @notice Interface for light client implementations that verify cross-chain state
/// @dev Inspired by IBC light client interface from cosmos/solidity-ibc-eureka
interface ILightClient {
    /// @notice Consensus state containing the minimum data needed to verify proofs
    struct ConsensusState {
        /// @dev Root of the state tree at this height
        bytes32 stateRoot;
        /// @dev Timestamp of the block
        uint64 timestamp;
    }

    /// @notice Emitted when the client state is updated
    /// @param height The height at which the client was updated
    /// @param stateRoot The new state root
    event ClientUpdated(uint64 indexed height, bytes32 stateRoot);

    /// @notice Update the light client with a new consensus state
    /// @param proof Proof data (format depends on implementation)
    /// @return success True if the update was successful
    function updateClient(bytes calldata proof) external returns (bool success);

    /// @notice Verify that a value exists at a given path in the counterparty state
    /// @param proof Merkle proof of inclusion
    /// @param height The height at which to verify
    /// @param path The path to the value in state
    /// @param value The expected value
    /// @return valid True if the membership proof is valid
    function verifyMembership(
        bytes calldata proof,
        uint64 height,
        bytes calldata path,
        bytes calldata value
    ) external view returns (bool valid);

    /// @notice Get the consensus state at a given height
    /// @param height The height to query
    /// @return state The consensus state at that height
    function getConsensusState(uint64 height) external view returns (ConsensusState memory state);

    /// @notice Get the latest verified height
    /// @return height The latest height
    function getLatestHeight() external view returns (uint64 height);
}
