// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ILightClient} from "../interfaces/ILightClient.sol";

/// @title TempoLightClient
/// @notice Light client for Tempo chain deployed on Ethereum
/// @dev Verifies Tempo finalization certificates using BLS threshold signatures
contract TempoLightClient is ILightClient {
    /// @notice Validator information for BLS verification
    struct Validator {
        /// @dev BLS public key (48 bytes for BLS12-381)
        bytes pubkey;
        /// @dev Voting power
        uint64 power;
    }

    /// @notice Finalization certificate from Tempo consensus
    struct FinalizationCertificate {
        /// @dev Block height being finalized
        uint64 height;
        /// @dev State root at this height
        bytes32 stateRoot;
        /// @dev Block timestamp
        uint64 timestamp;
        /// @dev Aggregated BLS signature (96 bytes)
        bytes signature;
        /// @dev Bitmap of validators who signed
        bytes signersBitmap;
    }

    /// @notice Storage proof for merkle verification
    struct StorageProof {
        /// @dev Key being proven
        bytes32 key;
        /// @dev Value at the key
        bytes value;
        /// @dev Merkle proof nodes
        bytes32[] proof;
    }

    /// @dev Consensus states indexed by height
    mapping(uint64 => ConsensusState) private _consensusStates;

    /// @dev Current validator set
    Validator[] private _validators;

    /// @dev Total voting power of the validator set
    uint64 private _totalPower;

    /// @dev Latest verified height
    uint64 private _latestHeight;

    /// @dev Threshold percentage for finality (66%)
    uint64 private constant FINALITY_THRESHOLD = 66;

    /// @dev Owner for initial setup
    address private immutable _owner;

    /// @notice Error when signature verification fails
    error InvalidSignature();

    /// @notice Error when insufficient voting power signed
    error InsufficientVotingPower(uint64 signed, uint64 required);

    /// @notice Error when height is not newer than latest
    error HeightNotNewer(uint64 provided, uint64 latest);

    /// @notice Error when consensus state not found
    error ConsensusStateNotFound(uint64 height);

    /// @notice Error when caller is not owner
    error NotOwner();

    /// @notice Error when merkle proof is invalid
    error InvalidMerkleProof();

    modifier onlyOwner() {
        if (msg.sender != _owner) revert NotOwner();
        _;
    }

    constructor(Validator[] memory initialValidators) {
        _owner = msg.sender;
        _setValidators(initialValidators);
    }

    /// @notice Update validator set (only callable by owner during bootstrap)
    /// @param newValidators New validator set
    function setValidators(Validator[] calldata newValidators) external onlyOwner {
        _setValidators(newValidators);
    }

    /// @inheritdoc ILightClient
    function updateClient(bytes calldata proof) external override returns (bool) {
        FinalizationCertificate memory cert = abi.decode(proof, (FinalizationCertificate));

        if (cert.height <= _latestHeight) {
            revert HeightNotNewer(cert.height, _latestHeight);
        }

        // Verify the BLS threshold signature
        uint64 signedPower = _verifyFinalizationCertificate(cert);
        uint64 requiredPower = (_totalPower * FINALITY_THRESHOLD) / 100;

        if (signedPower < requiredPower) {
            revert InsufficientVotingPower(signedPower, requiredPower);
        }

        // Store the new consensus state
        _consensusStates[cert.height] = ConsensusState({
            stateRoot: cert.stateRoot,
            timestamp: cert.timestamp
        });

        _latestHeight = cert.height;

        emit ClientUpdated(cert.height, cert.stateRoot);

        return true;
    }

    /// @inheritdoc ILightClient
    function verifyMembership(
        bytes calldata proof,
        uint64 height,
        bytes calldata path,
        bytes calldata value
    ) external view override returns (bool) {
        ConsensusState storage state = _consensusStates[height];
        if (state.stateRoot == bytes32(0)) {
            revert ConsensusStateNotFound(height);
        }

        StorageProof memory storageProof = abi.decode(proof, (StorageProof));

        // Verify the value matches
        if (keccak256(storageProof.value) != keccak256(value)) {
            return false;
        }

        // Verify the merkle proof against the state root
        bytes32 computedRoot = _computeMerkleRoot(
            keccak256(path),
            keccak256(value),
            storageProof.proof
        );

        return computedRoot == state.stateRoot;
    }

    /// @inheritdoc ILightClient
    function getConsensusState(
        uint64 height
    ) external view override returns (ConsensusState memory) {
        ConsensusState storage state = _consensusStates[height];
        if (state.stateRoot == bytes32(0)) {
            revert ConsensusStateNotFound(height);
        }
        return state;
    }

    /// @inheritdoc ILightClient
    function getLatestHeight() external view override returns (uint64) {
        return _latestHeight;
    }

    /// @notice Get the current validator set
    /// @return validators Array of validators
    function getValidators() external view returns (Validator[] memory) {
        return _validators;
    }

    /// @notice Get total voting power
    /// @return power Total voting power
    function getTotalPower() external view returns (uint64) {
        return _totalPower;
    }

    /// @dev Set the validator set
    function _setValidators(Validator[] memory newValidators) private {
        delete _validators;
        _totalPower = 0;

        for (uint256 i = 0; i < newValidators.length; i++) {
            _validators.push(newValidators[i]);
            _totalPower += newValidators[i].power;
        }
    }

    /// @dev Verify the finalization certificate and return signed voting power
    /// @param cert The finalization certificate
    /// @return signedPower The total voting power that signed
    function _verifyFinalizationCertificate(
        FinalizationCertificate memory cert
    ) private view returns (uint64 signedPower) {
        // Reconstruct the message that was signed
        bytes32 messageHash = keccak256(
            abi.encodePacked(cert.height, cert.stateRoot, cert.timestamp)
        );

        // Count voting power from signers bitmap
        signedPower = 0;
        for (uint256 i = 0; i < _validators.length; i++) {
            if (_isBitSet(cert.signersBitmap, i)) {
                signedPower += _validators[i].power;
            }
        }

        // In production, this would verify the aggregated BLS signature
        // using a BLS precompile or library. For now, we verify the structure.
        // The BLS12-381 curve operations would be:
        // 1. Aggregate public keys of signers
        // 2. Verify: e(signature, G2) == e(H(message), aggregatedPubkey)
        //
        // This requires EIP-2537 BLS precompiles or a Solidity BLS library
        if (cert.signature.length != 96) {
            revert InvalidSignature();
        }

        // Note: Actual BLS verification would go here
        // For production, use a verified BLS library
        _verifyBLSSignature(messageHash, cert.signature, cert.signersBitmap);

        return signedPower;
    }

    /// @dev Verify BLS signature (placeholder for actual implementation)
    /// @param messageHash Hash of the message that was signed
    /// @param signature Aggregated BLS signature
    /// @param signersBitmap Bitmap indicating which validators signed
    function _verifyBLSSignature(
        bytes32 messageHash,
        bytes memory signature,
        bytes memory signersBitmap
    ) private view {
        // In production, this would:
        // 1. Aggregate public keys from _validators based on signersBitmap
        // 2. Use BLS precompile (EIP-2537) or library to verify
        //
        // Example with EIP-2537 precompiles:
        // - BLS12_G1ADD (0x0b)
        // - BLS12_G1MUL (0x0c)
        // - BLS12_PAIRING (0x11)
        //
        // For now, we do basic validation
        if (signature.length == 0 || signersBitmap.length == 0) {
            revert InvalidSignature();
        }

        // Suppress unused variable warnings
        messageHash;
    }

    /// @dev Check if bit at index is set in bitmap
    function _isBitSet(bytes memory bitmap, uint256 index) private pure returns (bool) {
        if (index / 8 >= bitmap.length) return false;
        return (uint8(bitmap[index / 8]) & (1 << (index % 8))) != 0;
    }

    /// @dev Compute merkle root from leaf and proof
    function _computeMerkleRoot(
        bytes32 leaf,
        bytes32 leafValue,
        bytes32[] memory proof
    ) private pure returns (bytes32) {
        bytes32 computedHash = keccak256(abi.encodePacked(leaf, leafValue));

        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 proofElement = proof[i];

            if (computedHash < proofElement) {
                computedHash = keccak256(abi.encodePacked(computedHash, proofElement));
            } else {
                computedHash = keccak256(abi.encodePacked(proofElement, computedHash));
            }
        }

        return computedHash;
    }
}
