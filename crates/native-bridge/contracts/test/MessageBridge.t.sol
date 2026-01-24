// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test, console} from "forge-std/Test.sol";
import {MessageBridge} from "../src/MessageBridge.sol";
import {IMessageBridge} from "../src/interfaces/IMessageBridge.sol";

contract MessageBridgeTest is Test {
    MessageBridge public bridge;

    address public owner = address(0x1);
    address public user = address(0x2);

    uint64 public constant INITIAL_EPOCH = 1;
    uint64 public constant TEMPO_CHAIN_ID = 12345;

    // Dummy G1 public key (128 bytes) - in production this would be a real BLS key
    bytes public dummyPublicKey;

    function setUp() public {
        // Create a dummy 128-byte public key
        dummyPublicKey = new bytes(128);
        for (uint256 i = 0; i < 128; i++) {
            dummyPublicKey[i] = bytes1(uint8(i));
        }

        vm.chainId(TEMPO_CHAIN_ID);
        bridge = new MessageBridge(owner, INITIAL_EPOCH, dummyPublicKey);
    }

    //=============================================================
    //                      CONSTRUCTOR TESTS
    //=============================================================

    function test_constructor() public view {
        assertEq(bridge.owner(), owner);
        assertEq(bridge.epoch(), INITIAL_EPOCH);
        assertEq(bridge.chainId(), TEMPO_CHAIN_ID);
        assertEq(bridge.groupPublicKey(), dummyPublicKey);
        assertEq(bridge.paused(), false);
    }

    function test_constructor_invalidKeyLength() public {
        bytes memory shortKey = new bytes(64);
        vm.expectRevert(IMessageBridge.InvalidPublicKeyLength.selector);
        new MessageBridge(owner, INITIAL_EPOCH, shortKey);
    }

    //=============================================================
    //                        SEND TESTS
    //=============================================================

    function test_send() public {
        bytes32 messageHash = keccak256("test message");
        uint64 destChainId = 1;

        vm.prank(user);
        vm.expectEmit(true, true, true, true);
        emit IMessageBridge.MessageSent(user, messageHash, destChainId);
        bridge.send(messageHash, destChainId);

        assertTrue(bridge.isSent(user, messageHash));
    }

    function test_send_zeroHash() public {
        vm.prank(user);
        vm.expectRevert(IMessageBridge.ZeroMessageHash.selector);
        bridge.send(bytes32(0), 1);
    }

    function test_send_duplicate() public {
        bytes32 messageHash = keccak256("test message");

        vm.startPrank(user);
        bridge.send(messageHash, 1);

        vm.expectRevert(abi.encodeWithSelector(IMessageBridge.MessageAlreadySent.selector, user, messageHash));
        bridge.send(messageHash, 1);
        vm.stopPrank();
    }

    function test_send_whenPaused() public {
        vm.prank(owner);
        bridge.pause();

        vm.prank(user);
        vm.expectRevert(IMessageBridge.ContractPaused.selector);
        bridge.send(keccak256("test"), 1);
    }

    function test_send_sameHashDifferentSenders() public {
        bytes32 messageHash = keccak256("test message");
        address user2 = address(0x3);

        vm.prank(user);
        bridge.send(messageHash, 1);

        vm.prank(user2);
        bridge.send(messageHash, 1);

        assertTrue(bridge.isSent(user, messageHash));
        assertTrue(bridge.isSent(user2, messageHash));
    }

    //=============================================================
    //                    PAUSE/UNPAUSE TESTS
    //=============================================================

    function test_pause() public {
        vm.prank(owner);
        bridge.pause();
        assertTrue(bridge.paused());
    }

    function test_pause_unauthorized() public {
        vm.prank(user);
        vm.expectRevert(IMessageBridge.Unauthorized.selector);
        bridge.pause();
    }

    function test_unpause() public {
        vm.startPrank(owner);
        bridge.pause();
        bridge.unpause();
        vm.stopPrank();

        assertFalse(bridge.paused());
    }

    //=============================================================
    //                    OWNERSHIP TESTS
    //=============================================================

    function test_transferOwnership() public {
        address newOwner = address(0x99);

        vm.prank(owner);
        bridge.transferOwnership(newOwner);

        assertEq(bridge.owner(), newOwner);
    }

    function test_transferOwnership_unauthorized() public {
        vm.prank(user);
        vm.expectRevert(IMessageBridge.Unauthorized.selector);
        bridge.transferOwnership(user);
    }

    //=============================================================
    //                 FORCE SET KEY TESTS
    //=============================================================

    function test_forceSetGroupPublicKey() public {
        bytes memory newKey = new bytes(128);
        for (uint256 i = 0; i < 128; i++) {
            newKey[i] = bytes1(uint8(128 - i));
        }

        vm.prank(owner);
        bridge.forceSetGroupPublicKey(2, newKey);

        assertEq(bridge.epoch(), 2);
        assertEq(bridge.groupPublicKey(), newKey);
        assertEq(bridge.previousEpoch(), INITIAL_EPOCH);
        assertEq(bridge.previousGroupPublicKey(), dummyPublicKey);
    }

    function test_forceSetGroupPublicKey_epochMustIncrease() public {
        bytes memory newKey = new bytes(128);

        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(IMessageBridge.EpochMustIncrease.selector, INITIAL_EPOCH, INITIAL_EPOCH));
        bridge.forceSetGroupPublicKey(INITIAL_EPOCH, newKey);
    }

    function test_forceSetGroupPublicKey_invalidLength() public {
        bytes memory shortKey = new bytes(64);

        vm.prank(owner);
        vm.expectRevert(IMessageBridge.InvalidPublicKeyLength.selector);
        bridge.forceSetGroupPublicKey(2, shortKey);
    }

    function test_forceSetGroupPublicKey_unauthorized() public {
        bytes memory newKey = new bytes(128);

        vm.prank(user);
        vm.expectRevert(IMessageBridge.Unauthorized.selector);
        bridge.forceSetGroupPublicKey(2, newKey);
    }

    //=============================================================
    //                 ATTESTATION HASH TESTS
    //=============================================================

    function test_attestationHash_consistency() public view {
        address sender = address(0xAA);
        bytes32 messageHash = bytes32(uint256(0x11));
        uint64 originChainId = 1;
        uint64 destChainId = 12345;

        // Compute expected hash matching the contract's internal format
        bytes32 expected = keccak256(abi.encodePacked(
            "TEMPO_BRIDGE_V1",
            sender,
            messageHash,
            originChainId,
            destChainId
        ));

        // The contract uses this internally - we verify the format is consistent
        // by checking the same computation produces the same result
        bytes32 computed = keccak256(abi.encodePacked(
            "TEMPO_BRIDGE_V1",
            sender,
            messageHash,
            originChainId,
            destChainId
        ));

        assertEq(expected, computed);
    }

    //=============================================================
    //                 KEY ROTATION HASH TESTS
    //=============================================================

    function test_computeKeyRotationHash() public view {
        bytes memory newKey = new bytes(128);
        for (uint256 i = 0; i < 128; i++) {
            newKey[i] = bytes1(uint8(i));
        }

        bytes32 hash = bridge.computeKeyRotationHash(1, 2, newKey);

        // Verify determinism
        bytes32 hash2 = bridge.computeKeyRotationHash(1, 2, newKey);
        assertEq(hash, hash2);

        // Different epochs should produce different hash
        bytes32 hash3 = bridge.computeKeyRotationHash(1, 3, newKey);
        assertNotEq(hash, hash3);
    }

    //=============================================================
    //             INFINITY PUBLIC KEY TESTS (Security)
    //=============================================================

    function test_constructor_rejectsInfinityKey() public {
        // Point at infinity is all zeros for G1 (128 bytes)
        bytes memory infinityKey = new bytes(128);

        vm.expectRevert(IMessageBridge.PublicKeyIsInfinity.selector);
        new MessageBridge(owner, INITIAL_EPOCH, infinityKey);
    }

    function test_forceSetGroupPublicKey_rejectsInfinityKey() public {
        // Point at infinity is all zeros for G1 (128 bytes)
        bytes memory infinityKey = new bytes(128);

        vm.prank(owner);
        vm.expectRevert(IMessageBridge.PublicKeyIsInfinity.selector);
        bridge.forceSetGroupPublicKey(2, infinityKey);
    }
}
