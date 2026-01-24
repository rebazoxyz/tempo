// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title BLS12381
/// @notice BLS12-381 signature verification using EIP-2537 precompiles
/// @dev Implements RFC 9380 hash-to-curve for BLS12-381 G2
library BLS12381 {
    //=============================================================
    //                     PRECOMPILE ADDRESSES
    //=============================================================

    /// @notice G2 point addition precompile
    address internal constant BLS12_G2ADD = address(0x0d);

    /// @notice Pairing check precompile
    address internal constant BLS12_PAIRING_CHECK = address(0x0f);

    /// @notice Map Fp2 element to G2 point precompile
    address internal constant BLS12_MAP_FP2_TO_G2 = address(0x11);

    /// @notice SHA-256 precompile
    address internal constant SHA256_PRECOMPILE = address(0x02);

    /// @notice Modular exponentiation precompile
    address internal constant MODEXP_PRECOMPILE = address(0x05);

    //=============================================================
    //                         CONSTANTS
    //=============================================================

    /// @notice G1 point length (uncompressed)
    uint256 internal constant G1_POINT_LENGTH = 128;

    /// @notice G2 point length (uncompressed)
    uint256 internal constant G2_POINT_LENGTH = 256;

    /// @notice Fp element length (64 bytes: 16-byte padding + 48-byte value)
    uint256 internal constant FP_LENGTH = 64;

    /// @notice Fp2 element length (128 bytes: two Fp elements)
    uint256 internal constant FP2_LENGTH = 128;

    /// @notice BLS12-381 field modulus p
    /// @dev p = 0x1a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab
    bytes internal constant BLS12_381_P = hex"000000000000000000000000000000001a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab";

    /// @notice G1 generator point (uncompressed, 128 bytes)
    bytes internal constant G1_GENERATOR = hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1";

    /// @notice Negated G1 generator point (-G1, same x, y' = p - y)
    /// @dev Used for pairing check: e(pk, H(m)) * e(-G1, sig) == 1
    bytes internal constant NEG_G1_GENERATOR = hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb00000000000000000000000000000000114d1d6855d545a8aa7d76c8cf2e21f267816aef1db507c96655b9d5caac42364e6f38ba0ecb751bad54dcd6b939c2ca";

    //=============================================================
    //                         ERRORS
    //=============================================================

    error InvalidPublicKeyLength();
    error InvalidSignatureLength();
    error HashToG2Failed();
    error PairingCheckFailed();
    error MapToG2Failed();
    error G2AddFailed();
    error ModExpFailed();
    error PublicKeyIsInfinity();
    error SignatureIsInfinity();

    //=============================================================
    //                    SIGNATURE VERIFICATION
    //=============================================================

    /// @notice Verify a BLS signature
    /// @param publicKey G1 public key (128 bytes uncompressed)
    /// @param message The message that was signed (will be hashed to G2)
    /// @param dst Domain separation tag for hash-to-curve
    /// @param signature G2 signature (256 bytes uncompressed)
    /// @return True if signature is valid
    function verify(
        bytes memory publicKey,
        bytes memory message,
        bytes memory dst,
        bytes memory signature
    ) internal view returns (bool) {
        if (publicKey.length != G1_POINT_LENGTH) revert InvalidPublicKeyLength();
        if (signature.length != G2_POINT_LENGTH) revert InvalidSignatureLength();

        // Reject point at infinity for public key and signature
        // This prevents the rogue key attack where e(infinity, H(m)) * e(-G1, infinity) = 1
        if (_isG1Infinity(publicKey)) revert PublicKeyIsInfinity();
        if (_isG2Infinity(signature)) revert SignatureIsInfinity();

        // Hash message to G2 point
        bytes memory hm = hashToG2(message, dst);

        // Pairing check: e(pk, H(m)) * e(-G1, sig) == 1
        return pairingCheck(publicKey, hm, signature);
    }

    /// @notice Verify a BLS signature with pre-hashed message
    /// @param publicKey G1 public key (128 bytes uncompressed)
    /// @param messageHash 32-byte hash to sign (will be hashed to G2 with DST)
    /// @param dst Domain separation tag for hash-to-curve
    /// @param signature G2 signature (256 bytes uncompressed)
    /// @return True if signature is valid
    function verifyHash(
        bytes memory publicKey,
        bytes32 messageHash,
        bytes memory dst,
        bytes memory signature
    ) internal view returns (bool) {
        return verify(publicKey, abi.encodePacked(messageHash), dst, signature);
    }

    //=============================================================
    //                    PAIRING CHECK
    //=============================================================

    /// @notice Perform pairing check: e(pk, hm) * e(-G1, sig) == 1
    /// @param publicKey G1 public key (128 bytes)
    /// @param hashedMessage G2 point H(m) (256 bytes)
    /// @param signature G2 signature (256 bytes)
    /// @return True if pairing check passes
    function pairingCheck(
        bytes memory publicKey,
        bytes memory hashedMessage,
        bytes memory signature
    ) internal view returns (bool) {
        // Build pairing input: two pairs of (G1, G2) points
        // Pair 1: (publicKey, hashedMessage)
        // Pair 2: (-G1_generator, signature)
        // Total: 384 * 2 = 768 bytes
        bytes memory input = abi.encodePacked(
            publicKey,          // G1: 128 bytes
            hashedMessage,      // G2: 256 bytes
            NEG_G1_GENERATOR,   // G1: 128 bytes
            signature           // G2: 256 bytes
        );

        (bool success, bytes memory result) = BLS12_PAIRING_CHECK.staticcall(input);

        if (!success || result.length != 32) {
            return false;
        }

        // Pairing check returns 1 for success (equation holds)
        return abi.decode(result, (uint256)) == 1;
    }

    //=============================================================
    //                    HASH TO CURVE (RFC 9380)
    //=============================================================

    /// @notice Hash a message to a G2 point following RFC 9380
    /// @dev Implements hash_to_curve for BLS12-381 G2 with expand_message_xmd
    /// @param message The message to hash
    /// @param dst Domain separation tag
    /// @return G2 point (256 bytes uncompressed)
    function hashToG2(
        bytes memory message,
        bytes memory dst
    ) internal view returns (bytes memory) {
        // RFC 9380 Section 5.3: hash_to_curve
        // 1. u = hash_to_field(msg, 2) - produces 2 Fp2 elements
        // 2. Q0 = map_to_curve(u[0])
        // 3. Q1 = map_to_curve(u[1])
        // 4. R = Q0 + Q1
        // 5. P = clear_cofactor(R) - handled by MAP_FP2_TO_G2 precompile

        // For BLS12-381 G2:
        // - m = 2 (extension degree of Fp2)
        // - L = ceil((ceil(log2(p)) + k) / 8) = ceil((381 + 128) / 8) = 64
        // - len_in_bytes = count * m * L = 2 * 2 * 64 = 256

        bytes memory uniformBytes = expandMessageXmd(message, dst, 256);

        // Split into 4 field elements (each 64 bytes), assemble into 2 Fp2 elements
        bytes memory u0 = hashToFp2(uniformBytes, 0);   // bytes 0-127 -> Fp2
        bytes memory u1 = hashToFp2(uniformBytes, 128); // bytes 128-255 -> Fp2

        // Map each Fp2 to G2
        bytes memory q0 = mapToG2(u0);
        bytes memory q1 = mapToG2(u1);

        // Add the two G2 points
        return g2Add(q0, q1);
    }

    /// @notice Convert 128 bytes of uniform randomness to an Fp2 element
    /// @param uniformBytes The uniform random bytes (at least offset + 128)
    /// @param offset Starting offset in uniformBytes
    /// @return Fp2 element (128 bytes in EIP-2537 format)
    function hashToFp2(
        bytes memory uniformBytes,
        uint256 offset
    ) internal view returns (bytes memory) {
        // Each Fp2 = (c0, c1) where c0, c1 are Fp elements
        // Each Fp element comes from 64 bytes of uniform randomness reduced mod p

        bytes memory fp0 = reduceModP(uniformBytes, offset);        // First 64 bytes -> Fp
        bytes memory fp1 = reduceModP(uniformBytes, offset + 64);   // Next 64 bytes -> Fp

        // EIP-2537 Fp2 format: c0 || c1 (each 64 bytes)
        return abi.encodePacked(fp0, fp1);
    }

    /// @notice Reduce 64 bytes to an Fp element mod p using modexp precompile
    /// @param data Source data
    /// @param offset Offset to start reading 64 bytes
    /// @return 64-byte Fp element (with proper padding)
    function reduceModP(
        bytes memory data,
        uint256 offset
    ) internal view returns (bytes memory) {
        // Extract 64 bytes
        bytes memory input = new bytes(64);
        for (uint256 i = 0; i < 64; i++) {
            input[i] = data[offset + i];
        }

        // Use modexp precompile: base^1 mod p = base mod p
        // Input format: base_len (32) || exp_len (32) || mod_len (32) || base || exp || mod

        bytes memory modexpInput = abi.encodePacked(
            uint256(64),        // base length
            uint256(1),         // exponent length
            uint256(64),        // modulus length (p is 48 bytes but padded to 64)
            input,              // base (64 bytes)
            uint8(1),           // exponent = 1
            BLS12_381_P         // modulus p (64 bytes with padding)
        );

        (bool success, bytes memory result) = MODEXP_PRECOMPILE.staticcall(modexpInput);
        if (!success || result.length != 64) revert ModExpFailed();

        return result;
    }

    /// @notice Map an Fp2 element to a G2 point using EIP-2537 precompile
    /// @param fp2Element 128-byte Fp2 element
    /// @return 256-byte G2 point
    function mapToG2(bytes memory fp2Element) internal view returns (bytes memory) {
        (bool success, bytes memory result) = BLS12_MAP_FP2_TO_G2.staticcall(fp2Element);
        if (!success || result.length != G2_POINT_LENGTH) revert MapToG2Failed();
        return result;
    }

    /// @notice Add two G2 points using EIP-2537 precompile
    /// @param p1 First G2 point (256 bytes)
    /// @param p2 Second G2 point (256 bytes)
    /// @return Sum of the two points (256 bytes)
    function g2Add(bytes memory p1, bytes memory p2) internal view returns (bytes memory) {
        bytes memory input = abi.encodePacked(p1, p2);
        (bool success, bytes memory result) = BLS12_G2ADD.staticcall(input);
        if (!success || result.length != G2_POINT_LENGTH) revert G2AddFailed();
        return result;
    }

    //=============================================================
    //              EXPAND MESSAGE XMD (RFC 9380 Section 5.3.1)
    //=============================================================

    /// @notice Expand a message to uniform bytes using SHA-256
    /// @dev Implements expand_message_xmd from RFC 9380 Section 5.3.1
    /// @param message The message to expand
    /// @param dst Domain separation tag (must be <= 255 bytes)
    /// @param lenInBytes Number of output bytes (must be <= 255 * 32)
    /// @return Uniform random bytes
    function expandMessageXmd(
        bytes memory message,
        bytes memory dst,
        uint256 lenInBytes
    ) internal pure returns (bytes memory) {
        // Parameters for SHA-256:
        // b_in_bytes = 32 (output size)
        // r_in_bytes = 64 (input block size)

        uint256 ell = (lenInBytes + 31) / 32; // ceil(len_in_bytes / 32)
        require(ell <= 255, "ell too large");
        require(dst.length <= 255, "DST too long");

        // DST_prime = DST || I2OSP(len(DST), 1)
        bytes memory dstPrime = abi.encodePacked(dst, uint8(dst.length));

        // Z_pad = I2OSP(0, r_in_bytes) = 64 zero bytes
        bytes memory zPad = new bytes(64);

        // l_i_b_str = I2OSP(len_in_bytes, 2)
        bytes memory libStr = abi.encodePacked(uint16(lenInBytes));

        // msg_prime = Z_pad || msg || l_i_b_str || I2OSP(0, 1) || DST_prime
        bytes memory msgPrime = abi.encodePacked(
            zPad,
            message,
            libStr,
            uint8(0),
            dstPrime
        );

        // b_0 = H(msg_prime)
        bytes32 b0 = sha256(msgPrime);

        // b_1 = H(b_0 || I2OSP(1, 1) || DST_prime)
        bytes memory b = new bytes(lenInBytes);
        bytes32 bi = sha256(abi.encodePacked(b0, uint8(1), dstPrime));

        // Copy b_1 to output
        _copyBytes32ToBytes(bi, b, 0);

        // For i = 2 to ell: b_i = H((b_0 XOR b_{i-1}) || I2OSP(i, 1) || DST_prime)
        for (uint256 i = 2; i <= ell; i++) {
            bytes32 xored = b0 ^ bi;
            bi = sha256(abi.encodePacked(xored, uint8(i), dstPrime));
            _copyBytes32ToBytes(bi, b, (i - 1) * 32);
        }

        // Truncate to exact length (though we typically use exact multiples)
        if (b.length > lenInBytes) {
            bytes memory result = new bytes(lenInBytes);
            for (uint256 i = 0; i < lenInBytes; i++) {
                result[i] = b[i];
            }
            return result;
        }

        return b;
    }

    /// @notice Copy a bytes32 value into a bytes array at a given offset
    function _copyBytes32ToBytes(bytes32 src, bytes memory dst, uint256 offset) private pure {
        uint256 remaining = dst.length - offset;
        uint256 toCopy = remaining < 32 ? remaining : 32;

        for (uint256 i = 0; i < toCopy; i++) {
            dst[offset + i] = src[i];
        }
    }

    //=============================================================
    //                    INFINITY POINT CHECKS
    //=============================================================

    /// @notice Check if a G1 point (128 bytes) is the point at infinity (all zeros)
    /// @param point The G1 point to check
    /// @return True if the point is infinity
    function _isG1Infinity(bytes memory point) private pure returns (bool) {
        if (point.length != G1_POINT_LENGTH) return false;
        for (uint256 i = 0; i < G1_POINT_LENGTH; i++) {
            if (point[i] != 0) return false;
        }
        return true;
    }

    /// @notice Check if a G2 point (256 bytes) is the point at infinity (all zeros)
    /// @param point The G2 point to check
    /// @return True if the point is infinity
    function _isG2Infinity(bytes memory point) private pure returns (bool) {
        if (point.length != G2_POINT_LENGTH) return false;
        for (uint256 i = 0; i < G2_POINT_LENGTH; i++) {
            if (point[i] != 0) return false;
        }
        return true;
    }

    /// @notice Public function to validate a G1 public key is not infinity
    /// @param publicKey The G1 public key to validate
    /// @return True if the public key is valid (not infinity)
    function isValidPublicKey(bytes memory publicKey) internal pure returns (bool) {
        if (publicKey.length != G1_POINT_LENGTH) return false;
        return !_isG1Infinity(publicKey);
    }
}
