//! `.xsa` — XelaSoft Archive, a compressed MSX disk image.
//!
//! Decompression is a faithful port of openMSX `src/fdc/XSAExtractor.cc`
//! (LZ77 + an adaptive Huffman tree over 16 distance-code classes). The format
//! is cross-confirmed identical to MAME's `imgtool/modules/xsa.c`.
//!
//! VERIFICATION: the literal + header + END-token + bit-reader paths are covered
//! by a hand-traced golden test below. The match-copy and adaptive-Huffman
//! distance paths are validated end-to-end by the `real_fixtures` integration
//! test, which decompresses a genuine `.xsa` into a mountable FAT disk.

use crate::error::{Error, Result};

/// XSA file signature: the ASCII "PCK" followed by byte 0x08.
pub const MAGIC: &[u8] = b"PCK\x08";

const MAX_STR_LEN: u32 = 254;
const TBL_SIZE: usize = 16;
const MAX_HUF_CNT: i32 = 127;

/// Extra distance bits per code class (from openMSX).
const CPD_EXT: [u8; TBL_SIZE] = [0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

/// Upper bound on the declared output size (in sectors) to reject absurd or
/// malicious headers. 65536 sectors == 32 MB, far above any real MSX disk.
const MAX_OUTPUT_SECTORS: usize = 65_536;

/// A node in the adaptive Huffman tree. `child1 < 0` marks a leaf.
#[derive(Clone, Copy)]
struct HufNode {
    weight: i32,
    child1: i32,
    child2: i32,
}

const ROOT: usize = 2 * TBL_SIZE - 2;

struct Decoder<'a> {
    file: &'a [u8],
    pos: usize,
    out: Vec<u8>,
    out_idx: usize,
    bit_flg: u8,
    bit_cnt: u8,
    upd_huf_cnt: i32,
    cp_dist: [i32; TBL_SIZE + 1],
    tbl_sizes: [i32; TBL_SIZE],
    huf_tbl: [HufNode; 2 * TBL_SIZE - 1],
}

/// Decompress an XSA image into a normalized linear sector buffer.
pub fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut d = Decoder {
        file: bytes,
        pos: 0,
        out: Vec::new(),
        out_idx: 0,
        bit_flg: 0,
        bit_cnt: 0,
        upd_huf_cnt: 0,
        cp_dist: [0; TBL_SIZE + 1],
        tbl_sizes: [0; TBL_SIZE],
        huf_tbl: [HufNode {
            weight: 0,
            child1: -1,
            child2: -1,
        }; 2 * TBL_SIZE - 1],
    };
    d.check_magic()?;
    d.chk_header()?;
    d.init_huf_info();
    d.un_lz77()?;
    Ok(d.out)
}

impl Decoder<'_> {
    /// Read the next raw byte from the input, advancing the cursor.
    fn char_in(&mut self) -> Result<u8> {
        let b = *self
            .file
            .get(self.pos)
            .ok_or_else(|| Error::Malformed("xsa: unexpected end of file".into()))?;
        self.pos += 1;
        Ok(b)
    }

    /// Read a single bit, LSB-first within each byte.
    fn bit_in(&mut self) -> Result<bool> {
        if self.bit_cnt == 0 {
            self.bit_flg = self.char_in()?;
            self.bit_cnt = 8;
        }
        let bit = self.bit_flg & 1 == 1;
        self.bit_cnt -= 1;
        self.bit_flg >>= 1;
        Ok(bit)
    }

    /// Read `n` bits (n <= 8), MSB-first into the result.
    fn get_n_bits(&mut self, n: u8) -> Result<u8> {
        let mut result = 0u8;
        for _ in 0..n {
            result = (result << 1) | u8::from(self.bit_in()?);
        }
        Ok(result)
    }

    fn check_magic(&mut self) -> Result<()> {
        for &expected in MAGIC {
            if self.char_in()? != expected {
                return Err(Error::Malformed("not an XSA image (bad signature)".into()));
            }
        }
        Ok(())
    }

    fn chk_header(&mut self) -> Result<()> {
        // Original (decompressed) length, little-endian.
        let mut out_len = 0u32;
        for i in 0..4 {
            out_len |= u32::from(self.char_in()?) << (8 * i);
        }
        let sectors = (out_len as usize).div_ceil(512);
        if sectors == 0 || sectors > MAX_OUTPUT_SECTORS {
            return Err(Error::Malformed(format!(
                "xsa: implausible decompressed size {out_len}"
            )));
        }
        self.out = vec![0u8; sectors * 512];

        // Skip the compressed length (4 bytes) and the NUL-terminated filename.
        for _ in 0..4 {
            self.char_in()?;
        }
        while self.char_in()? != 0 {}
        Ok(())
    }

    fn un_lz77(&mut self) -> Result<()> {
        self.bit_cnt = 0;
        let mut remaining = self.out.len();
        loop {
            if self.bit_in()? {
                let str_len = self.rd_str_len()?;
                if str_len == MAX_STR_LEN + 1 {
                    return Ok(());
                }
                let str_pos = self.rd_str_pos()? as usize;
                if str_pos == 0 || str_pos > self.out_idx {
                    return Err(Error::Malformed(
                        "xsa: invalid back-reference offset".into(),
                    ));
                }
                let str_len = str_len as usize;
                if remaining < str_len {
                    return Err(Error::Malformed("xsa: output buffer overrun".into()));
                }
                remaining -= str_len;
                for _ in 0..str_len {
                    self.out[self.out_idx] = self.out[self.out_idx - str_pos];
                    self.out_idx += 1;
                }
            } else {
                if remaining == 0 {
                    return Err(Error::Malformed("xsa: output buffer overrun".into()));
                }
                remaining -= 1;
                self.out[self.out_idx] = self.char_in()?;
                self.out_idx += 1;
            }
        }
    }

    fn rd_str_len(&mut self) -> Result<u32> {
        if !self.bit_in()? {
            return Ok(2);
        }
        if !self.bit_in()? {
            return Ok(3);
        }
        if !self.bit_in()? {
            return Ok(4);
        }
        let mut nr_bits = 2u8;
        while nr_bits != 7 && self.bit_in()? {
            nr_bits += 1;
        }
        let mut len = 1u32;
        for _ in 0..nr_bits {
            len = (len << 1) | u32::from(self.bit_in()?);
        }
        Ok(len + 1)
    }

    fn rd_str_pos(&mut self) -> Result<i32> {
        let mut huf = ROOT;
        while self.huf_tbl[huf].child1 >= 0 {
            huf = if self.bit_in()? {
                self.huf_tbl[huf].child2 as usize
            } else {
                self.huf_tbl[huf].child1 as usize
            };
        }
        let cpd_index = huf;
        self.tbl_sizes[cpd_index] += 1;

        let ext = CPD_EXT[cpd_index];
        let str_pos = if ext >= 8 {
            let lsb = i32::from(self.char_in()?);
            let msb = i32::from(self.get_n_bits(ext - 8)?);
            lsb + 256 * msb
        } else {
            i32::from(self.get_n_bits(ext)?)
        };

        // Rebuild the table every MAX_HUF_CNT+1 reads (matches the original's
        // post-decrement-then-test semantics).
        let rebuild = self.upd_huf_cnt == 0;
        self.upd_huf_cnt -= 1;
        if rebuild {
            self.mk_huf_tbl();
        }
        Ok(str_pos + self.cp_dist[cpd_index])
    }

    fn init_huf_info(&mut self) {
        let mut offs = 1i32;
        for (i, &ext) in CPD_EXT.iter().enumerate() {
            self.cp_dist[i] = offs;
            offs += 1 << ext;
        }
        self.cp_dist[TBL_SIZE] = offs;

        for i in 0..TBL_SIZE {
            self.tbl_sizes[i] = 0;
            self.huf_tbl[i].child1 = -1; // mark leaf nodes
        }
        self.mk_huf_tbl();
    }

    fn mk_huf_tbl(&mut self) {
        // Seed leaf weights (halving the counters ages older statistics) and
        // mark every internal slot as free with weight -1.
        for i in 0..TBL_SIZE {
            self.tbl_sizes[i] >>= 1;
            self.huf_tbl[i].weight = 1 + self.tbl_sizes[i];
        }
        for i in TBL_SIZE..(2 * TBL_SIZE - 1) {
            self.huf_tbl[i].weight = -1;
        }

        // Repeatedly combine the two lowest positive-weight nodes into the next
        // free slot until the root has a weight.
        while self.huf_tbl[ROOT].weight == -1 {
            let mut pos = 0;
            while self.huf_tbl[pos].weight == 0 {
                pos += 1;
            }
            let mut l1 = pos;
            pos += 1;
            while self.huf_tbl[pos].weight == 0 {
                pos += 1;
            }
            let mut l2 = if self.huf_tbl[pos].weight < self.huf_tbl[l1].weight {
                let tmp = l1;
                l1 = pos;
                pos += 1;
                tmp
            } else {
                let tmp = pos;
                pos += 1;
                tmp
            };
            loop {
                let w = self.huf_tbl[pos].weight;
                if w == -1 {
                    break;
                }
                if w != 0 {
                    if w < self.huf_tbl[l1].weight {
                        l2 = l1;
                        l1 = pos;
                    } else if w < self.huf_tbl[l2].weight {
                        l2 = pos;
                    }
                }
                pos += 1;
            }
            self.huf_tbl[pos].weight = self.huf_tbl[l1].weight + self.huf_tbl[l2].weight;
            self.huf_tbl[pos].child1 = l1 as i32;
            self.huf_tbl[l1].weight = 0;
            self.huf_tbl[pos].child2 = l2 as i32;
            self.huf_tbl[l2].weight = 0;
        }
        self.upd_huf_cnt = MAX_HUF_CNT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_magic() {
        assert!(decompress(b"NOPE....").is_err());
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(decompress(b"PCK\x08\x01\x00").is_err());
    }

    /// Hand-traced golden vector: a single literal byte 0xAB then the END
    /// token. The body bytes were computed by simulating openMSX's exact
    /// bit/byte pull order (see module docs), so this independently verifies
    /// magic, header parsing, the LSB-first bit reader, the literal path, and
    /// END-token detection.
    #[test]
    fn decompresses_single_literal_and_end() {
        let mut input = Vec::new();
        input.extend_from_slice(MAGIC);
        input.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // original length = 1
        input.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // compressed length (skipped)
        input.push(0x00); // empty filename
                          // body: flag#0=0xFE (literal then start of END), literal=0xAB,
                          //       flag#1=0xFF, flag#2=0x00 (rest of END length code)
        input.extend_from_slice(&[0xFE, 0xAB, 0xFF, 0x00]);

        let out = decompress(&input).expect("decompress");
        assert_eq!(out.len(), 512);
        assert_eq!(out[0], 0xAB);
        assert!(out[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn huffman_table_builds_without_panic() {
        // init_huf_info builds the full tree; a successful traversal setup means
        // every internal slot was filled and the root resolved.
        let mut d = Decoder {
            file: &[],
            pos: 0,
            out: Vec::new(),
            out_idx: 0,
            bit_flg: 0,
            bit_cnt: 0,
            upd_huf_cnt: 0,
            cp_dist: [0; TBL_SIZE + 1],
            tbl_sizes: [0; TBL_SIZE],
            huf_tbl: [HufNode {
                weight: 0,
                child1: -1,
                child2: -1,
            }; 2 * TBL_SIZE - 1],
        };
        d.init_huf_info();
        // Root must be an internal node (has children) and cp_dist must be
        // strictly increasing.
        assert!(d.huf_tbl[ROOT].child1 >= 0);
        assert_eq!(d.cp_dist[0], 1);
        for i in 1..=TBL_SIZE {
            assert!(d.cp_dist[i] > d.cp_dist[i - 1]);
        }
    }
}
