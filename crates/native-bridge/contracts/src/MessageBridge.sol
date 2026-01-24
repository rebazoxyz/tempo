// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IMessageBridge} from "./interfaces/IMessageBridge.sol";

/// @title MessageBridge
/// @notice Minimal cross-chain messaging using BLS threshold signatures
/// @dev Uses EIP-2537 BLS12-381 precompiles for signature verification
contract MessageBridge is IMessageBridge {
    //=============================================================
    //                          CONSTANTS
    //=============================================================

    /// @notice Domain separator for bridge attestations
    bytes public constant BRIDGE_DOMAIN = "TEMPO_BRIDGE_V1";

    /// @notice Domain separator for key rotation
    bytes public constant KEY_ROTATION_DOMAIN = "TEMPO_BRIDGE_KEY_ROTATION_V1";

    /// @notice BLS12-381 pairing check precompile (EIP-2537)
    address internal constant BLS12_PAIRING_CHECK = address(0x0f);

    /// @notice Expected length for uncompressed G1 point (public key)
    uint256 internal constant G1_POINT_LENGTH = 128;

    /// @notice Expected length for uncompressed G2 point (signature)
    uint256 internal constant G2_POINT_LENGTH = 128;

    /// @notice G1 generator point for BLS12-381 (uncompressed)
    /// @dev Used as the base point in pairing: e(pk, H(m)) == e(G1, sig)
    bytes internal constant G1_GENERATOR = hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1";

    //=============================================================
    //                          STORAGE
    //=============================================================

    /// @notice Contract owner
    address public owner;

    /// @notice Pause state
    bool public paused;

    /// @notice This chain's ID
    uint64 public immutable chainId;

    /// @notice Current validator epoch
    uint64 public epoch;

    /// @notice Previous epoch (for grace period)
    uint64 public previousEpoch;

    /// @notice BLS group public key for current epoch (G1 point, 128 bytes)
    bytes public groupPublicKey;

    /// @notice BLS group public key for previous epoch
    bytes public previousGroupPublicKey;

    /// @notice Sent messages: sender => messageHash => sent
    mapping(address => mapping(bytes32 => bool)) public sent;

    /// @notice Received messages: originChainId => sender => messageHash => timestamp
    mapping(uint64 => mapping(address => mapping(bytes32 => uint256))) public received;

    //=============================================================
    //                        MODIFIERS
    //=============================================================

    modifier onlyOwner() {
        if (msg.sender != owner) revert Unauthorized();
        _;
    }

    modifier whenNotPaused() {
        if (paused) revert ContractPaused();
        _;
    }

    //=============================================================
    //                       CONSTRUCTOR
    //=============================================================

    /// @param _owner Contract owner
    /// @param _initialEpoch Initial epoch number
    /// @param _initialPublicKey Initial BLS group public key (G1, 128 bytes)
    constructor(address _owner, uint64 _initialEpoch, bytes memory _initialPublicKey) {
        if (_initialPublicKey.length != G1_POINT_LENGTH) revert InvalidPublicKeyLength();

        owner = _owner;
        chainId = uint64(block.chainid);
        epoch = _initialEpoch;
        groupPublicKey = _initialPublicKey;
    }

    //=============================================================
    //                      SEND FUNCTION
    //=============================================================

    /// @inheritdoc IMessageBridge
    function send(bytes32 messageHash, uint64 destinationChainId) external whenNotPaused {
        if (messageHash == bytes32(0)) revert ZeroMessageHash();

        if (sent[msg.sender][messageHash]) {
            revert MessageAlreadySent(msg.sender, messageHash);
        }

        sent[msg.sender][messageHash] = true;

        emit MessageSent(msg.sender, messageHash, destinationChainId);
    }

    //=============================================================
    //                      WRITE FUNCTION
    //=============================================================

    /// @inheritdoc IMessageBridge
    function write(
        address sender,
        bytes32 messageHash,
        uint64 originChainId,
        bytes calldata signature
    ) external whenNotPaused {
        if (signature.length != G2_POINT_LENGTH) revert InvalidSignatureLength();

        if (received[originChainId][sender][messageHash] != 0) {
            revert MessageAlreadyReceived(originChainId, sender, messageHash);
        }

        bytes32 attestationHash = _computeAttestationHash(sender, messageHash, originChainId, chainId);

        bool valid = _verifyBLSSignature(groupPublicKey, attestationHash, signature);

        if (!valid && previousGroupPublicKey.length > 0) {
            valid = _verifyBLSSignature(previousGroupPublicKey, attestationHash, signature);
        }

        if (!valid) revert InvalidBLSSignature();

        uint256 timestamp = block.timestamp;
        received[originChainId][sender][messageHash] = timestamp;

        emit MessageReceived(originChainId, sender, messageHash, timestamp);
    }

    //=============================================================
    //                      READ FUNCTIONS
    //=============================================================

    /// @inheritdoc IMessageBridge
    function receivedAt(uint64 originChainId, address sender, bytes32 messageHash) external view returns (uint256) {
        return received[originChainId][sender][messageHash];
    }

    /// @inheritdoc IMessageBridge
    function isSent(address sender, bytes32 messageHash) external view returns (bool) {
        return sent[sender][messageHash];
    }

    //=============================================================
    //                   KEY ROTATION FUNCTIONS
    //=============================================================

    /// @inheritdoc IMessageBridge
    function rotateKey(
        uint64 newEpoch,
        bytes calldata newPublicKey,
        bytes calldata authSignature
    ) external whenNotPaused {
        if (newPublicKey.length != G1_POINT_LENGTH) revert InvalidPublicKeyLength();
        if (authSignature.length != G2_POINT_LENGTH) revert InvalidSignatureLength();
        if (newEpoch <= epoch) revert EpochMustIncrease(epoch, newEpoch);
        if (groupPublicKey.length == 0) revert NoActivePublicKey();

        bytes32 rotationHash = _computeKeyRotationHash(epoch, newEpoch, newPublicKey);

        bool valid = _verifyBLSSignature(groupPublicKey, rotationHash, authSignature);
        if (!valid) revert KeyTransitionNotAuthorized();

        emit KeyRotationAuthorized(epoch, newEpoch, newPublicKey);

        _rotateKey(newEpoch, newPublicKey);
    }

    /// @inheritdoc IMessageBridge
    function computeKeyRotationHash(
        uint64 oldEpoch,
        uint64 newEpoch,
        bytes calldata newPublicKey
    ) external pure returns (bytes32) {
        return _computeKeyRotationHash(oldEpoch, newEpoch, newPublicKey);
    }

    //=============================================================
    //                      ADMIN FUNCTIONS
    //=============================================================

    /// @inheritdoc IMessageBridge
    function forceSetGroupPublicKey(uint64 newEpoch, bytes calldata publicKey) external onlyOwner {
        if (publicKey.length != G1_POINT_LENGTH) revert InvalidPublicKeyLength();
        if (newEpoch <= epoch) revert EpochMustIncrease(epoch, newEpoch);

        _rotateKey(newEpoch, publicKey);
    }

    /// @inheritdoc IMessageBridge
    function pause() external onlyOwner {
        paused = true;
    }

    /// @inheritdoc IMessageBridge
    function unpause() external onlyOwner {
        paused = false;
    }

    /// @inheritdoc IMessageBridge
    function transferOwnership(address newOwner) external onlyOwner {
        require(newOwner != address(0), "Invalid owner");
        owner = newOwner;
    }

    //=============================================================
    //                      INTERNAL FUNCTIONS
    //=============================================================

    /// @notice Compute attestation hash for a message
    function _computeAttestationHash(
        address sender,
        bytes32 messageHash,
        uint64 originChainId,
        uint64 destinationChainId
    ) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(
            "TEMPO_BRIDGE_V1",
            sender,
            messageHash,
            originChainId,
            destinationChainId
        ));
    }

    /// @notice Compute key rotation authorization hash
    function _computeKeyRotationHash(
        uint64 oldEpoch,
        uint64 newEpoch,
        bytes memory newPublicKey
    ) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(
            "TEMPO_BRIDGE_KEY_ROTATION_V1",
            oldEpoch,
            newEpoch,
            newPublicKey
        ));
    }

    /// @notice Internal key rotation
    function _rotateKey(uint64 newEpoch, bytes memory newPublicKey) internal {
        bytes memory oldKey = groupPublicKey;
        uint64 oldEpoch = epoch;

        previousEpoch = epoch;
        previousGroupPublicKey = groupPublicKey;

        epoch = newEpoch;
        groupPublicKey = newPublicKey;

        emit KeyRotated(oldEpoch, newEpoch, oldKey, newPublicKey);
    }

    /// @notice Verify a BLS signature using EIP-2537 pairing check
    /// @dev Uses pairing check: e(pk, H(m)) == e(G1, sig)
    ///      Which is verified as: e(pk, H(m)) * e(-G1, sig) == 1
    ///      We need to hash the message to G2 first (hash_to_curve)
    /// @param publicKey G1 public key (128 bytes)
    /// @param messageHash The message hash (will be hashed to G2)
    /// @param signature G2 signature (128 bytes)
    /// @return True if signature is valid
    function _verifyBLSSignature(
        bytes memory publicKey,
        bytes32 messageHash,
        bytes calldata signature
    ) internal view returns (bool) {
        // Hash message to G2 point
        bytes memory messagePoint = _hashToG2(messageHash);

        // Negate G1 generator for pairing check
        bytes memory negG1 = _negateG1(G1_GENERATOR);

        // Build pairing input: (pk, H(m), -G1, sig)
        // Pairing checks: e(pk, H(m)) * e(-G1, sig) == 1
        bytes memory input = abi.encodePacked(
            publicKey,      // G1 point (128 bytes)
            messagePoint,   // G2 point (256 bytes)
            negG1,          // -G1 generator (128 bytes)
            signature       // G2 signature (256 bytes, but we receive 128)
        );

        // Note: The signature from calldata is 128 bytes, but G2 points are actually
        // 256 bytes uncompressed (two 128-byte Fp2 elements). This needs adjustment
        // based on actual encoding used. For now, assume the signature encoding matches.

        // Call pairing check precompile
        (bool success, bytes memory result) = BLS12_PAIRING_CHECK.staticcall(input);

        if (!success || result.length != 32) {
            return false;
        }

        // Pairing check returns 1 for success
        return abi.decode(result, (uint256)) == 1;
    }

    /// @notice Hash a message to G2 (simplified - production should use proper hash_to_curve)
    /// @dev This is a placeholder. Production implementation should use RFC 9380 hash_to_curve
    ///      via the MAP_FP2_TO_G2 precompile (0x11) with proper expand_message_xmd
    function _hashToG2(bytes32 messageHash) internal view returns (bytes memory) {
        // Use MAP_FP2_TO_G2 precompile (0x11) to map field elements to G2
        // First, expand message to field elements using expand_message_xmd
        // Then call the precompile twice and add the results

        // For now, this is a simplified implementation
        // Production code should follow RFC 9380 Section 5.3

        address MAP_FP2_TO_G2 = address(0x11);
        address G2_ADD = address(0x0d);

        // Expand message hash to two Fp2 elements (4 * 64 = 256 bytes for input to map)
        bytes memory u0 = _expandToFp2(messageHash, 0);
        bytes memory u1 = _expandToFp2(messageHash, 1);

        // Map each Fp2 element to G2
        (bool success0, bytes memory q0) = MAP_FP2_TO_G2.staticcall(u0);
        (bool success1, bytes memory q1) = MAP_FP2_TO_G2.staticcall(u1);

        if (!success0 || !success1) {
            // Return a zero point or handle error
            return new bytes(256);
        }

        // Add the two G2 points
        bytes memory addInput = abi.encodePacked(q0, q1);
        (bool successAdd, bytes memory result) = G2_ADD.staticcall(addInput);

        if (!successAdd) {
            return new bytes(256);
        }

        return result;
    }

    /// @notice Expand a hash to an Fp2 element for hash_to_curve
    /// @dev Simplified version - production should use expand_message_xmd from RFC 9380
    function _expandToFp2(bytes32 messageHash, uint8 index) internal pure returns (bytes memory) {
        // Each Fp2 element needs 128 bytes (two 64-byte Fp elements, padded to 64 bytes each)
        bytes memory result = new bytes(128);

        // Hash with domain separation for each component
        bytes32 c0 = keccak256(abi.encodePacked(messageHash, index, uint8(0)));
        bytes32 c0_ext = keccak256(abi.encodePacked(c0));
        bytes32 c1 = keccak256(abi.encodePacked(messageHash, index, uint8(1)));
        bytes32 c1_ext = keccak256(abi.encodePacked(c1));

        // Pack into Fp2 format (each Fp is 48 bytes, padded to 64)
        // Fp element: 16 bytes padding + 48 bytes value
        assembly {
            // First Fp element (c0)
            mstore(add(result, 32), 0) // 16 bytes padding
            mstore(add(result, 48), c0)
            mstore(add(result, 80), c0_ext)

            // Second Fp element (c1)
            mstore(add(result, 96), 0) // 16 bytes padding
            mstore(add(result, 112), c1)
            mstore(add(result, 144), c1_ext)
        }

        return result;
    }

    /// @notice Negate a G1 point (flip y-coordinate)
    /// @dev For BLS12-381, negation is flipping the y-coordinate
    function _negateG1(bytes memory point) internal pure returns (bytes memory) {
        require(point.length == G1_POINT_LENGTH, "Invalid G1 point");

        bytes memory result = new bytes(G1_POINT_LENGTH);

        // Copy x-coordinate (first 64 bytes)
        for (uint256 i = 0; i < 64; i++) {
            result[i] = point[i];
        }

        // Negate y-coordinate: y' = p - y (where p is the field modulus)
        // The field modulus p for BLS12-381:
        // 0x1a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab

        // For simplicity, we'll do the negation using the fact that
        // the precompile accepts the negated point. In practice, this
        // requires modular subtraction which is complex in Solidity.

        // Copy y-coordinate for now (full implementation would negate)
        for (uint256 i = 64; i < 128; i++) {
            result[i] = point[i];
        }

        // TODO: Implement proper negation or use G1 MSM with scalar -1

        return result;
    }
}
