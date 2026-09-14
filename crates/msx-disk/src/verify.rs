//! Content checksums: CRC32 and SHA-1.
//!
//! These are the hashes software databases (TOSEC, Generation MSX, openMSX's
//! software DB) key on, so computing them lets a dump be matched against a
//! known catalogue. CRC32 identifies a file cheaply; SHA-1 confirms it.
//!
//! Both algorithms are hand-rolled. The crate already hand-rolls CRC-16 for the
//! archive and DMK checks (see [`crate::archive`]) and keeps its dependency set
//! minimal, so adding a hashing crate is not warranted for ~100 lines pinned by
//! the standard known-answer vectors in the tests below.

use crate::error::Result;
use crate::fs::DiskFs;

/// The CRC32 and SHA-1 of a byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checksums {
    /// CRC32 (IEEE 802.3, the zip/PNG/TOSEC variant).
    pub crc32: u32,
    /// SHA-1 digest (RFC 3174), 20 bytes.
    pub sha1: [u8; 20],
}

impl Checksums {
    /// Compute both checksums over `bytes`.
    pub fn of(bytes: &[u8]) -> Checksums {
        Checksums {
            crc32: crc32(bytes),
            sha1: sha1(bytes),
        }
    }

    /// CRC32 as eight uppercase hex digits, e.g. `CBF43926`.
    pub fn crc32_hex(&self) -> String {
        format!("{:08X}", self.crc32)
    }

    /// SHA-1 as forty lowercase hex digits.
    pub fn sha1_hex(&self) -> String {
        use std::fmt::Write;
        self.sha1
            .iter()
            .fold(String::with_capacity(40), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            })
    }
}

/// Checksums over the whole normalized sector buffer (i.e. the disk image).
pub fn image_checksums(data: &[u8]) -> Checksums {
    Checksums::of(data)
}

/// Checksums over the byte content of `path` on a mounted floppy filesystem.
pub fn file_checksums(fs: &DiskFs, path: &str) -> Result<Checksums> {
    Ok(Checksums::of(&fs.read_file(path)?))
}

/// CRC32 (IEEE 802.3): reflected input/output, polynomial `0xEDB88820`.
///
/// Same bitwise shape as [`crate::archive`]'s CRC-16, just the 32-bit poll with
/// the standard `0xFFFFFFFF` pre/post conditioning.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// SHA-1 (RFC 3174). Verified against the standard `"abc"` and empty vectors.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];

    // Pre-processing: append 0x80, pad with zeros so the length is 56 mod 64,
    // then the 64-bit big-endian bit length.
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vector() {
        // The canonical CRC32 check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn crc32_of_empty_input_is_zero() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn sha1_matches_rfc3174_vectors() {
        // "abc" — the one-block vector.
        assert_eq!(
            Checksums::of(b"abc").sha1_hex(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // The empty string.
        assert_eq!(
            Checksums::of(b"").sha1_hex(),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        // The 56-byte two-block vector (exercises the length-padding spillover).
        assert_eq!(
            Checksums::of(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq").sha1_hex(),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn crc32_hex_is_eight_uppercase_digits() {
        let c = Checksums {
            crc32: 0x000F_43A2,
            sha1: [0; 20],
        };
        assert_eq!(c.crc32_hex(), "000F43A2");
    }

    #[test]
    fn image_checksums_is_deterministic() {
        let data = vec![0xABu8; 4096];
        assert_eq!(image_checksums(&data), image_checksums(&data));
        assert_eq!(image_checksums(&data), Checksums::of(&data));
    }
}
