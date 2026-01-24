//! EIP-2537 BLS12-381 format conversion utilities.
//!
//! The EIP-2537 precompiles expect points in uncompressed format with
//! specific padding for each field element:
//!
//! - G1 points: 128 bytes (2 × 64-byte Fp elements)
//! - G2 points: 256 bytes (4 × 64-byte Fp elements)
//!
//! Each Fp element is 64 bytes: 16 zero-padding bytes + 48-byte value (big-endian).
//!
//! This module provides conversion between:
//! - Compressed G2 signatures (96 bytes) used internally
//! - EIP-2537 uncompressed G2 format (256 bytes) required by on-chain verification

use crate::error::{BridgeError, Result};
use crate::message::{G1_COMPRESSED_LEN, G1_UNCOMPRESSED_LEN, G2_COMPRESSED_LEN, G2_UNCOMPRESSED_LEN};

use blst::{blst_p1_affine, blst_p1_uncompress, blst_p2_affine, blst_p2_uncompress, BLST_ERROR};

/// Convert a compressed G2 signature (96 bytes) to EIP-2537 format (256 bytes).
///
/// The EIP-2537 format for G2 points is:
/// - x.c0: 16 zero bytes + 48-byte field element (big-endian)
/// - x.c1: 16 zero bytes + 48-byte field element (big-endian)  
/// - y.c0: 16 zero bytes + 48-byte field element (big-endian)
/// - y.c1: 16 zero bytes + 48-byte field element (big-endian)
///
/// Total: 4 × 64 = 256 bytes
pub fn g2_to_eip2537(compressed: &[u8; G2_COMPRESSED_LEN]) -> Result<[u8; G2_UNCOMPRESSED_LEN]> {
    // Decompress the G2 point
    let mut affine = blst_p2_affine::default();
    
    // SAFETY: blst_p2_uncompress validates the compressed point encoding
    let result = unsafe { blst_p2_uncompress(&mut affine, compressed.as_ptr()) };
    
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BridgeError::Signing(format!(
            "failed to decompress G2 point: {:?}",
            result
        )));
    }

    // Convert affine point to EIP-2537 format
    // blst_p2_affine contains: x (Fp2) and y (Fp2)
    // Each Fp2 contains: fp[0] (c0) and fp[1] (c1)
    // Each Fp is 48 bytes in blst (6 × u64 in little-endian)
    
    let mut output = [0u8; G2_UNCOMPRESSED_LEN];
    
    // x.c0 (bytes 0-63): 16 padding + 48-byte value
    fp_to_eip2537(&affine.x.fp[0].l, &mut output[0..64]);
    
    // x.c1 (bytes 64-127): 16 padding + 48-byte value
    fp_to_eip2537(&affine.x.fp[1].l, &mut output[64..128]);
    
    // y.c0 (bytes 128-191): 16 padding + 48-byte value
    fp_to_eip2537(&affine.y.fp[0].l, &mut output[128..192]);
    
    // y.c1 (bytes 192-255): 16 padding + 48-byte value
    fp_to_eip2537(&affine.y.fp[1].l, &mut output[192..256]);

    Ok(output)
}

/// Convert a compressed G1 public key (48 bytes) to EIP-2537 format (128 bytes).
///
/// The EIP-2537 format for G1 points is:
/// - x: 16 zero bytes + 48-byte field element (big-endian)
/// - y: 16 zero bytes + 48-byte field element (big-endian)
///
/// Total: 2 × 64 = 128 bytes
pub fn g1_to_eip2537(compressed: &[u8; G1_COMPRESSED_LEN]) -> Result<[u8; G1_UNCOMPRESSED_LEN]> {
    // Decompress the G1 point
    let mut affine = blst_p1_affine::default();
    
    // SAFETY: blst_p1_uncompress validates the compressed point encoding
    let result = unsafe { blst_p1_uncompress(&mut affine, compressed.as_ptr()) };
    
    if result != BLST_ERROR::BLST_SUCCESS {
        return Err(BridgeError::Signing(format!(
            "failed to decompress G1 point: {:?}",
            result
        )));
    }

    let mut output = [0u8; G1_UNCOMPRESSED_LEN];
    
    // x (bytes 0-63): 16 padding + 48-byte value
    fp_to_eip2537(&affine.x.l, &mut output[0..64]);
    
    // y (bytes 64-127): 16 padding + 48-byte value
    fp_to_eip2537(&affine.y.l, &mut output[64..128]);

    Ok(output)
}

/// Convert blst Fp limbs (6 × u64 little-endian) to EIP-2537 Fp format (64 bytes).
///
/// EIP-2537 Fp format: 16 zero-padding bytes + 48-byte big-endian value
fn fp_to_eip2537(limbs: &[u64; 6], out: &mut [u8]) {
    assert!(out.len() >= 64);
    
    // First 16 bytes are zero padding
    out[0..16].fill(0);
    
    // Convert 6 × 64-bit limbs (little-endian) to 48-byte big-endian
    // blst stores Fp as 6 × u64 in little-endian (least significant limb first)
    // We need big-endian output (most significant byte first)
    for (i, limb) in limbs.iter().rev().enumerate() {
        let bytes = limb.to_be_bytes();
        out[16 + i * 8..16 + (i + 1) * 8].copy_from_slice(&bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::Encode;
    use commonware_cryptography::bls12381::{
        dkg,
        primitives::{
            group::G2,
            ops::sign,
            sharing::Mode,
            variant::MinPk,
        },
    };
    use commonware_utils::{NZU32, N3f1};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn test_g2_to_eip2537_produces_256_bytes() {
        // Create test share
        let mut rng = StdRng::seed_from_u64(42);
        let n = NZU32!(5);
        let (_sharing, shares) = dkg::deal_anonymous::<MinPk, N3f1>(&mut rng, Mode::default(), n);
        let share = &shares[0];
        
        let message = b"test message";
        let dst = b"TEMPO_BRIDGE_BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_";

        let signature: G2 = sign::<MinPk>(&share.private, dst, message);

        // Get compressed signature (96 bytes)
        let compressed = signature.encode();
        assert_eq!(compressed.len(), G2_COMPRESSED_LEN);

        let compressed_array: [u8; G2_COMPRESSED_LEN] = compressed.as_ref().try_into().unwrap();

        // Convert to EIP-2537 format
        let eip2537 = g2_to_eip2537(&compressed_array).unwrap();
        assert_eq!(eip2537.len(), G2_UNCOMPRESSED_LEN);

        // Verify padding structure: each 64-byte element starts with 16 zero bytes
        for i in 0..4 {
            let offset = i * 64;
            assert_eq!(
                &eip2537[offset..offset + 16],
                &[0u8; 16],
                "element {} should have 16-byte zero padding",
                i
            );
        }
    }

    #[test]
    fn test_g2_to_eip2537_deterministic() {
        let mut rng = StdRng::seed_from_u64(123);
        let n = NZU32!(5);
        let (_sharing, shares) = dkg::deal_anonymous::<MinPk, N3f1>(&mut rng, Mode::default(), n);
        let share = &shares[0];
        
        let signature: G2 = sign::<MinPk>(
            &share.private,
            b"TEMPO_BRIDGE_BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_",
            b"hello",
        );

        let compressed = signature.encode();
        let compressed_array: [u8; G2_COMPRESSED_LEN] = compressed.as_ref().try_into().unwrap();

        let result1 = g2_to_eip2537(&compressed_array).unwrap();
        let result2 = g2_to_eip2537(&compressed_array).unwrap();

        assert_eq!(result1, result2);
    }

    #[test]
    fn test_g2_to_eip2537_different_signatures_produce_different_output() {
        let mut rng = StdRng::seed_from_u64(456);
        let n = NZU32!(5);
        let (_sharing, shares) = dkg::deal_anonymous::<MinPk, N3f1>(&mut rng, Mode::default(), n);
        let share = &shares[0];
        
        let dst = b"TEMPO_BRIDGE_BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_";

        let sig1: G2 = sign::<MinPk>(&share.private, dst, b"message1");
        let sig2: G2 = sign::<MinPk>(&share.private, dst, b"message2");

        let c1: [u8; G2_COMPRESSED_LEN] = sig1.encode().as_ref().try_into().unwrap();
        let c2: [u8; G2_COMPRESSED_LEN] = sig2.encode().as_ref().try_into().unwrap();

        let eip1 = g2_to_eip2537(&c1).unwrap();
        let eip2 = g2_to_eip2537(&c2).unwrap();

        assert_ne!(eip1, eip2);
    }
}
