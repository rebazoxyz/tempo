// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test} from "forge-std/Test.sol";
import {BLS12381} from "../src/BLS12381.sol";

/// @title BLS12381Test
/// @notice Tests for the BLS12381 library
/// @dev Note: Full signature verification tests require EIP-2537 precompiles
///      which are not available in standard Foundry. These tests focus on
///      the pure Solidity components (expand_message_xmd, etc.)
contract BLS12381Test is Test {
    // Test DST matching the bridge
    bytes constant TEST_DST = "TEMPO_BRIDGE_BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_";

    //=============================================================
    //                    EXPAND MESSAGE XMD TESTS
    //=============================================================

    function test_expandMessageXmd_deterministic() public pure {
        bytes memory message = "test message";
        bytes memory result1 = BLS12381.expandMessageXmd(message, TEST_DST, 256);
        bytes memory result2 = BLS12381.expandMessageXmd(message, TEST_DST, 256);

        assertEq(result1.length, 256);
        assertEq(keccak256(result1), keccak256(result2));
    }

    function test_expandMessageXmd_differentMessages() public pure {
        bytes memory msg1 = "message one";
        bytes memory msg2 = "message two";

        bytes memory result1 = BLS12381.expandMessageXmd(msg1, TEST_DST, 256);
        bytes memory result2 = BLS12381.expandMessageXmd(msg2, TEST_DST, 256);

        assertNotEq(keccak256(result1), keccak256(result2));
    }

    function test_expandMessageXmd_differentDSTs() public pure {
        bytes memory message = "test message";
        bytes memory dst1 = "DST_ONE";
        bytes memory dst2 = "DST_TWO";

        bytes memory result1 = BLS12381.expandMessageXmd(message, dst1, 256);
        bytes memory result2 = BLS12381.expandMessageXmd(message, dst2, 256);

        assertNotEq(keccak256(result1), keccak256(result2));
    }

    function test_expandMessageXmd_correctLength() public pure {
        bytes memory message = "test";

        // Test various lengths
        assertEq(BLS12381.expandMessageXmd(message, TEST_DST, 32).length, 32);
        assertEq(BLS12381.expandMessageXmd(message, TEST_DST, 64).length, 64);
        assertEq(BLS12381.expandMessageXmd(message, TEST_DST, 128).length, 128);
        assertEq(BLS12381.expandMessageXmd(message, TEST_DST, 256).length, 256);
    }

    function test_expandMessageXmd_emptyMessage() public pure {
        bytes memory empty = "";
        bytes memory result = BLS12381.expandMessageXmd(empty, TEST_DST, 256);

        assertEq(result.length, 256);
        // Should still produce non-zero output
        assertNotEq(keccak256(result), keccak256(new bytes(256)));
    }

    function test_expandMessageXmd_bytes32Input() public pure {
        // Test with a bytes32 hash as input (common use case)
        bytes32 hash = keccak256("attestation data");
        bytes memory result = BLS12381.expandMessageXmd(abi.encodePacked(hash), TEST_DST, 256);

        assertEq(result.length, 256);
    }

    //=============================================================
    //                    CONSTANTS VALIDATION
    //=============================================================

    function test_g1GeneratorLength() public pure {
        assertEq(BLS12381.G1_GENERATOR.length, 128);
    }

    function test_negG1GeneratorLength() public pure {
        assertEq(BLS12381.NEG_G1_GENERATOR.length, 128);
    }

    function test_g1GeneratorAndNegHaveSameX() public pure {
        bytes memory g1 = BLS12381.G1_GENERATOR;
        bytes memory negG1 = BLS12381.NEG_G1_GENERATOR;

        // First 64 bytes (x-coordinate) should be identical
        for (uint256 i = 0; i < 64; i++) {
            assertEq(g1[i], negG1[i], "X coordinates should match");
        }
    }

    function test_g1GeneratorAndNegHaveDifferentY() public pure {
        bytes memory g1 = BLS12381.G1_GENERATOR;
        bytes memory negG1 = BLS12381.NEG_G1_GENERATOR;

        // Y coordinates (bytes 64-127) should be different
        bool yDifferent = false;
        for (uint256 i = 64; i < 128; i++) {
            if (g1[i] != negG1[i]) {
                yDifferent = true;
                break;
            }
        }
        assertTrue(yDifferent, "Y coordinates should differ");
    }

    function test_fieldModulusLength() public pure {
        assertEq(BLS12381.BLS12_381_P.length, 64);
    }

    //=============================================================
    //                    INPUT VALIDATION TESTS
    //=============================================================

    function test_verify_invalidPublicKeyLength() public {
        bytes memory shortPk = new bytes(64);
        bytes memory message = "test";
        bytes memory signature = new bytes(256);

        // Library internal functions revert at the same depth, so we catch differently
        try this.callVerify(shortPk, message, TEST_DST, signature) {
            fail("Should have reverted");
        } catch {
            // Expected revert
        }
    }

    function test_verify_invalidSignatureLength() public {
        bytes memory pk = new bytes(128);
        bytes memory message = "test";
        bytes memory shortSig = new bytes(128);

        try this.callVerify(pk, message, TEST_DST, shortSig) {
            fail("Should have reverted");
        } catch {
            // Expected revert
        }
    }

    /// @notice External wrapper to test library function reverts
    function callVerify(
        bytes memory pk,
        bytes memory message,
        bytes memory dst,
        bytes memory signature
    ) external view returns (bool) {
        return BLS12381.verify(pk, message, dst, signature);
    }

    function test_verify_rejectsInfinityPublicKey() public {
        // Point at infinity is all zeros for G1 (128 bytes)
        bytes memory infinityPk = new bytes(128);
        bytes memory message = "test";
        // Valid length signature (256 bytes with some non-zero data)
        bytes memory signature = new bytes(256);
        signature[0] = 0x01;

        try this.callVerify(infinityPk, message, TEST_DST, signature) {
            fail("Should have reverted with PublicKeyIsInfinity");
        } catch (bytes memory reason) {
            assertEq(bytes4(reason), BLS12381.PublicKeyIsInfinity.selector);
        }
    }

    function test_verify_rejectsInfinitySignature() public {
        // Valid length public key with some non-zero data
        bytes memory pk = new bytes(128);
        pk[0] = 0x01;
        bytes memory message = "test";
        // Point at infinity is all zeros for G2 (256 bytes)
        bytes memory infinitySig = new bytes(256);

        try this.callVerify(pk, message, TEST_DST, infinitySig) {
            fail("Should have reverted with SignatureIsInfinity");
        } catch (bytes memory reason) {
            assertEq(bytes4(reason), BLS12381.SignatureIsInfinity.selector);
        }
    }

    function test_isValidPublicKey_rejectsInfinity() public pure {
        bytes memory infinityPk = new bytes(128);
        assertFalse(BLS12381.isValidPublicKey(infinityPk));
    }

    function test_isValidPublicKey_acceptsNonZero() public pure {
        bytes memory validPk = new bytes(128);
        validPk[0] = 0x01;
        assertTrue(BLS12381.isValidPublicKey(validPk));
    }

    function test_isValidPublicKey_rejectsWrongLength() public pure {
        bytes memory shortPk = new bytes(64);
        shortPk[0] = 0x01;
        assertFalse(BLS12381.isValidPublicKey(shortPk));
    }

    //=============================================================
    //              RFC 9380 TEST VECTORS (expand_message_xmd)
    //=============================================================

    /// @notice Test expand_message_xmd against RFC 9380 test vectors
    /// @dev Test vector from RFC 9380 Section A.3.1 (SHA-256, 0x20 output)
    function test_expandMessageXmd_rfc9380_vector_empty_32() public pure {
        // RFC 9380 Section A.3.1 - expand_message_xmd with SHA-256
        // DST = "QUUX-V01-CS02-with-expander-SHA256-128"
        // msg = ""
        // len_in_bytes = 0x20 (32)
        bytes memory dst = "QUUX-V01-CS02-with-expander-SHA256-128";
        bytes memory result = BLS12381.expandMessageXmd("", dst, 32);

        // Expected from RFC 9380:
        bytes32 expected = hex"68a985b87eb6b46952128911f2a4412bbc302a9d759667f87f7a21d803f07235";
        assertEq(keccak256(result), keccak256(abi.encodePacked(expected)));
    }

    /// @notice RFC 9380 A.3.1 test vector: msg = "abc", len = 32
    function test_expandMessageXmd_rfc9380_vector_abc_32() public pure {
        bytes memory dst = "QUUX-V01-CS02-with-expander-SHA256-128";
        bytes memory result = BLS12381.expandMessageXmd("abc", dst, 32);

        bytes32 expected = hex"d8ccab23b5985ccea865c6c97b6e5b8350e794e603b4b97902f53a8a0d605615";
        assertEq(keccak256(result), keccak256(abi.encodePacked(expected)));
    }

    //=============================================================
    //    DIFFERENTIAL TEST VECTORS (Rust vs Solidity)
    //    Generated from Rust expand_message_xmd implementation
    //=============================================================

    /// @notice Differential test: empty message with bridge DST
    /// @dev Expected value generated from Rust implementation
    function test_expandMessageXmd_differential_empty() public pure {
        bytes memory result = BLS12381.expandMessageXmd("", TEST_DST, 256);

        // Expected from Rust: expand_message_xmd(b"", BLS_DST, 256)
        bytes memory expected = hex"16492f3f7d1a240be0e00102fb8e6a03a76e55371552f54987f0c5d1d26b5a53e3317641f3edc5a3b7dfb76724c77fd86f43208b0ce4766d418dc64613d224a005c2571bd09ded0f9b79afda75d47c1ead76b806e808febf4e0886a4186a0555fac4ce3f247d2612e90f5e7fed11ec8922a5a33db0a0cc60621f1aab72c05632c4f9c78686efa5d294fc5ce60f8485ad3c807348d4f247c519b1b9ac97c1b1564b41586dcf270306276fbbc7d2fb1492b0a70f47a38e0dbb7ae23c29186bbe642a48fe05ef85162ffacb7c18d31b5b3e1335023faf5f02e5d340bd587825665bc238d09d646b1fe86360467a871c190d90496b97601f82e1330a18d77606c048";
        
        assertEq(keccak256(result), keccak256(expected), "Solidity should match Rust for empty message");
    }

    /// @notice Differential test: "test" message with bridge DST
    function test_expandMessageXmd_differential_test() public pure {
        bytes memory result = BLS12381.expandMessageXmd("test", TEST_DST, 256);

        // Expected from Rust: expand_message_xmd(b"test", BLS_DST, 256)
        bytes memory expected = hex"33388e19d7674f2f029e0de0e62b8b46c284e4915c8c12cb0df4ef92e1b61d072d1ce4a9a501e2f9eae1e431319d5ec930a53bbcf7b9f7fbba04dd47cabd02b3f76c14b7fda800c0db139920fef0507de46f9742143863b03141b6481d55ff9df2b0032c738099e75f3f00b28e201d7d7136fe4ecec8c603c1377ff7d5f12400a55ff562e3ddd10bdd8ba008457007acfd12bafc9667a0f5255cfc994a31b11c78a1444be70fc60e87704b997d8f41c5a39ea52d32ebfe24f727eae3fbcb10da58148722b692f23c730aba1f50de0ff568e0a08c9eeb75aaf09621b2e3d66f927e62d29594232238427530c48494a2061b300302b105e1f79720219202fec505";
        
        assertEq(keccak256(result), keccak256(expected), "Solidity should match Rust for 'test' message");
    }

    /// @notice Differential test: 32-byte hash input (attestation hash simulation)
    function test_expandMessageXmd_differential_hash32() public pure {
        // 0x42 repeated 32 times (simulates attestation hash)
        bytes memory message = hex"4242424242424242424242424242424242424242424242424242424242424242";
        bytes memory result = BLS12381.expandMessageXmd(message, TEST_DST, 256);

        // Expected from Rust: expand_message_xmd(&[0x42u8; 32], BLS_DST, 256)
        bytes memory expected = hex"97ee2a4bb87efa1327ca89a2da22fe6ac3daf2cd4d974fa341a6f43e4738aea1fdab1a22ca13c9a335638a5e9a02752b6db51c16af3a56446c075d78dfc240d3e301c615fb62c53e290ee00f5f65021296e84d3e6117fabb389f52a7651858b34c604be8563c0dcd5932f088887b38d7e020d9b9262eefea81020929652e5af96cf88f9a62512754e75e2b30b50bc52c16cce0920bc3a4ee6982f9b8cfb011ab3f8065e04f8906b2eaf4333775e0513edb6248fcdcfe01506f2ca821e493e88f5882000caeb020c948c05db8273f660a56eba7e2f53664360af534a0580524e2b8c08611a2a0d3cb5949a490a84fe937c43be905deed291b565fee30e8724233";
        
        assertEq(keccak256(result), keccak256(expected), "Solidity should match Rust for 32-byte hash input");
    }

    /// @notice Differential test: short output (32 bytes)
    function test_expandMessageXmd_differential_short() public pure {
        bytes memory result = BLS12381.expandMessageXmd("short", TEST_DST, 32);

        // Expected from Rust: expand_message_xmd(b"short", BLS_DST, 32)
        bytes32 expected = hex"647b59246b8fb81b72409a012bf469ed0dda1cac81fe5da0b4b0287a683788fc";
        
        assertEq(keccak256(result), keccak256(abi.encodePacked(expected)), "Solidity should match Rust for short output");
    }
}
