//! `.xsa` — XelaSoft Archive, a compressed MSX disk image.
//!
//! Decompression is a faithful port of openMSX `src/fdc/XSAExtractor.cc`
//! (LZ77 + an adaptive Huffman tree over 16 distance-code classes), validated
//! against a real `.xsa` by the `real_fixtures` integration test.
//!
//! Compression ([`compress`]) is the inverse: an LZ77 match finder feeding a
//! bit/byte writer that mirrors the decoder's exact consumption order, with the
//! same adaptive Huffman table evolved in lockstep. Because the decoder is known
//! correct against real data, a successful compress -> decompress round trip
//! means the output is real-format-correct and readable by other XSA tools.

use crate::error::{Error, Result};

/// XSA file signature: the ASCII "PCK" followed by byte 0x08.
pub const MAGIC: &[u8] = b"PCK\x08";

const MAX_STR_LEN: u32 = 254;
const TBL_SIZE: usize = 16;
const MAX_HUF_CNT: i32 = 127;
const ROOT: usize = 2 * TBL_SIZE - 2;

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

/// The adaptive Huffman state shared by the decoder and encoder, so both evolve
/// the distance-code table identically.
struct Huffman {
    huf_tbl: [HufNode; 2 * TBL_SIZE - 1],
    tbl_sizes: [i32; TBL_SIZE],
    cp_dist: [i32; TBL_SIZE + 1],
    upd_huf_cnt: i32,
}

impl Huffman {
    fn new() -> Huffman {
        let mut h = Huffman {
            huf_tbl: [HufNode {
                weight: 0,
                child1: -1,
                child2: -1,
            }; 2 * TBL_SIZE - 1],
            tbl_sizes: [0; TBL_SIZE],
            cp_dist: [0; TBL_SIZE + 1],
            upd_huf_cnt: 0,
        };
        let mut offs = 1i32;
        for (i, &ext) in CPD_EXT.iter().enumerate() {
            h.cp_dist[i] = offs;
            offs += 1 << ext;
        }
        h.cp_dist[TBL_SIZE] = offs;
        for i in 0..TBL_SIZE {
            h.tbl_sizes[i] = 0;
            h.huf_tbl[i].child1 = -1; // mark leaf nodes
        }
        h.mk_huf_tbl();
        h
    }

    fn is_leaf(&self, node: usize) -> bool {
        self.huf_tbl[node].child1 < 0
    }

    fn child(&self, node: usize, one: bool) -> usize {
        if one {
            self.huf_tbl[node].child2 as usize
        } else {
            self.huf_tbl[node].child1 as usize
        }
    }

    /// Record that distance code `idx` was used, updating counters and rebuilding
    /// the table on schedule (matching the decoder's post-decrement semantics).
    fn note_use(&mut self, idx: usize) {
        self.tbl_sizes[idx] += 1;
        let rebuild = self.upd_huf_cnt == 0;
        self.upd_huf_cnt -= 1;
        if rebuild {
            self.mk_huf_tbl();
        }
    }

    /// The root-to-leaf bit path for distance code `idx` (false = child1).
    fn code(&self, idx: usize) -> Vec<bool> {
        fn dfs(h: &Huffman, node: usize, target: usize, path: &mut Vec<bool>) -> bool {
            if node == target {
                return true;
            }
            if h.is_leaf(node) {
                return false;
            }
            path.push(false);
            if dfs(h, h.huf_tbl[node].child1 as usize, target, path) {
                return true;
            }
            path.pop();
            path.push(true);
            if dfs(h, h.huf_tbl[node].child2 as usize, target, path) {
                return true;
            }
            path.pop();
            false
        }
        let mut path = Vec::new();
        dfs(self, ROOT, idx, &mut path);
        path
    }

    fn mk_huf_tbl(&mut self) {
        for i in 0..TBL_SIZE {
            self.tbl_sizes[i] >>= 1;
            self.huf_tbl[i].weight = 1 + self.tbl_sizes[i];
        }
        for i in TBL_SIZE..(2 * TBL_SIZE - 1) {
            self.huf_tbl[i].weight = -1;
        }
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

// ---------------------------------------------------------------------------
// Decompression
// ---------------------------------------------------------------------------

struct Decoder<'a> {
    file: &'a [u8],
    pos: usize,
    out: Vec<u8>,
    out_idx: usize,
    bit_flg: u8,
    bit_cnt: u8,
    huf: Huffman,
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
        huf: Huffman::new(),
    };
    d.check_magic()?;
    d.chk_header()?;
    d.un_lz77()?;
    Ok(d.out)
}

impl Decoder<'_> {
    fn char_in(&mut self) -> Result<u8> {
        let b = *self
            .file
            .get(self.pos)
            .ok_or_else(|| Error::Malformed("xsa: unexpected end of file".into()))?;
        self.pos += 1;
        Ok(b)
    }

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
        let mut node = ROOT;
        while !self.huf.is_leaf(node) {
            let one = self.bit_in()?;
            node = self.huf.child(node, one);
        }
        let idx = node;
        let ext = CPD_EXT[idx];
        let str_pos = if ext >= 8 {
            let lsb = i32::from(self.char_in()?);
            let msb = i32::from(self.get_n_bits(ext - 8)?);
            lsb + 256 * msb
        } else {
            i32::from(self.get_n_bits(ext)?)
        };
        let base = self.huf.cp_dist[idx];
        self.huf.note_use(idx);
        Ok(str_pos + base)
    }
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = MAX_STR_LEN as usize;
/// Largest back-reference distance the format can encode (`cpDist[16] - 1`).
const WINDOW: usize = 8194;
const HASH_BITS: u32 = 16;
const MAX_CHAIN: usize = 64;

/// Emits bits (LSB-first into flag bytes) and raw bytes in exactly the order the
/// decoder consumes them. A flag byte's slot is reserved in the stream when its
/// first bit is written and backpatched once full, so raw bytes written while it
/// is being filled land after it (and before the next flag) — matching how the
/// decoder pulls a flag byte, then reads the raw bytes used during its 8 bits.
struct BitByteWriter {
    out: Vec<u8>,
    cur_flag: u8,
    bit_count: u8,
    flag_index: usize,
    flag_open: bool,
}

impl BitByteWriter {
    fn new() -> BitByteWriter {
        BitByteWriter {
            out: Vec::new(),
            cur_flag: 0,
            bit_count: 0,
            flag_index: 0,
            flag_open: false,
        }
    }

    fn put_bit(&mut self, bit: bool) {
        if !self.flag_open {
            self.flag_index = self.out.len();
            self.out.push(0);
            self.cur_flag = 0;
            self.bit_count = 0;
            self.flag_open = true;
        }
        if bit {
            self.cur_flag |= 1 << self.bit_count;
        }
        self.bit_count += 1;
        if self.bit_count == 8 {
            self.out[self.flag_index] = self.cur_flag;
            self.flag_open = false;
        }
    }

    /// Emit the low `n` bits of `value`, most-significant first.
    fn put_bits_msb(&mut self, value: u32, n: u8) {
        for i in (0..n).rev() {
            self.put_bit((value >> i) & 1 == 1);
        }
    }

    fn put_byte(&mut self, byte: u8) {
        self.out.push(byte);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.flag_open {
            self.out[self.flag_index] = self.cur_flag;
        }
        self.out
    }
}

fn emit_len(w: &mut BitByteWriter, value: u32) {
    match value {
        2 => w.put_bit(false),
        3 => {
            w.put_bit(true);
            w.put_bit(false);
        }
        4 => {
            w.put_bit(true);
            w.put_bit(true);
            w.put_bit(false);
        }
        _ => {
            w.put_bit(true);
            w.put_bit(true);
            w.put_bit(true);
            let len = value - 1;
            let nr_bits = 31 - len.leading_zeros(); // floor(log2(len)), 2..=7
            for _ in 0..(nr_bits - 2) {
                w.put_bit(true);
            }
            if nr_bits != 7 {
                w.put_bit(false);
            }
            w.put_bits_msb(len, nr_bits as u8);
        }
    }
}

fn emit_dist(w: &mut BitByteWriter, dist: i32, huf: &mut Huffman) {
    let mut idx = 0;
    for i in 0..TBL_SIZE {
        if huf.cp_dist[i] <= dist {
            idx = i;
        } else {
            break;
        }
    }
    for bit in huf.code(idx) {
        w.put_bit(bit);
    }
    let ext = CPD_EXT[idx];
    let value = (dist - huf.cp_dist[idx]) as u32;
    if ext >= 8 {
        w.put_byte((value & 0xFF) as u8);
        w.put_bits_msb(value >> 8, ext - 8);
    } else {
        w.put_bits_msb(value, ext);
    }
    huf.note_use(idx);
}

fn hash(data: &[u8], i: usize) -> usize {
    let key = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8) | u32::from(data[i + 2]);
    (key.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
}

/// Find the longest back-reference match at `pos`, returning `(len, distance)`.
fn find_match(data: &[u8], pos: usize, head: &[i32], prev: &[i32]) -> (usize, usize) {
    let max_len = (data.len() - pos).min(MAX_MATCH);
    if max_len < MIN_MATCH {
        return (0, 0);
    }
    let mut best_len = 0;
    let mut best_dist = 0;
    let mut cand = head[hash(data, pos)];
    let mut chain = 0;
    while cand >= 0 {
        let c = cand as usize;
        if pos - c > WINDOW {
            break;
        }
        if data[c + best_len] == data[pos + best_len] {
            let mut l = 0;
            while l < max_len && data[c + l] == data[pos + l] {
                l += 1;
            }
            if l > best_len {
                best_len = l;
                best_dist = pos - c;
                if l == max_len {
                    break;
                }
            }
        }
        cand = prev[c];
        chain += 1;
        if chain >= MAX_CHAIN {
            break;
        }
    }
    if best_len >= MIN_MATCH {
        (best_len, best_dist)
    } else {
        (0, 0)
    }
}

/// Compress raw disk data into an XSA image.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    let comp_len_pos = out.len();
    out.extend_from_slice(&[0, 0, 0, 0]); // compressed length, patched below
    out.push(0); // empty filename

    let mut w = BitByteWriter::new();
    let mut huf = Huffman::new();

    let mut head = vec![-1i32; 1 << HASH_BITS];
    let mut prev = vec![-1i32; data.len().max(1)];
    let insert = |head: &mut [i32], prev: &mut [i32], i: usize| {
        if i + MIN_MATCH <= data.len() {
            let h = hash(data, i);
            prev[i] = head[h];
            head[h] = i as i32;
        }
    };

    let mut pos = 0;
    while pos < data.len() {
        let (mlen, mdist) = find_match(data, pos, &head, &prev);
        if mlen >= MIN_MATCH {
            w.put_bit(true);
            emit_len(&mut w, mlen as u32);
            emit_dist(&mut w, mdist as i32, &mut huf);
            for k in pos..pos + mlen {
                insert(&mut head, &mut prev, k);
            }
            pos += mlen;
        } else {
            w.put_bit(false);
            w.put_byte(data[pos]);
            insert(&mut head, &mut prev, pos);
            pos += 1;
        }
    }

    // END token: a match whose length code is MAX_STR_LEN + 1.
    w.put_bit(true);
    emit_len(&mut w, MAX_STR_LEN + 1);

    let body = w.finish();
    out.extend_from_slice(&body);
    let comp_len = body.len() as u32;
    out[comp_len_pos..comp_len_pos + 4].copy_from_slice(&comp_len.to_le_bytes());
    out
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

    #[test]
    fn decompresses_single_literal_and_end() {
        let mut input = Vec::new();
        input.extend_from_slice(MAGIC);
        input.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        input.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        input.push(0x00);
        input.extend_from_slice(&[0xFE, 0xAB, 0xFF, 0x00]);

        let out = decompress(&input).expect("decompress");
        assert_eq!(out.len(), 512);
        assert_eq!(out[0], 0xAB);
        assert!(out[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn huffman_table_builds() {
        let huf = Huffman::new();
        assert!(!huf.is_leaf(ROOT));
        assert_eq!(huf.cp_dist[0], 1);
        assert_eq!(huf.cp_dist[TBL_SIZE], WINDOW as i32 + 1);
        for i in 1..=TBL_SIZE {
            assert!(huf.cp_dist[i] > huf.cp_dist[i - 1]);
        }
        // Every distance code has a non-empty, unique-prefix path.
        for idx in 0..TBL_SIZE {
            assert!(!huf.code(idx).is_empty());
        }
    }

    fn roundtrip(data: &[u8]) {
        let xsa = compress(data);
        assert!(xsa.starts_with(MAGIC));
        let back = decompress(&xsa).expect("decompress");
        assert_eq!(&back[..data.len()], data, "round trip mismatch");
    }

    #[test]
    fn roundtrip_repetitive_compresses() {
        let data: Vec<u8> = (0..2048).map(|i| (i % 37) as u8).collect();
        roundtrip(&data);
        assert!(
            compress(&data).len() < data.len(),
            "repetitive data should shrink"
        );
    }

    #[test]
    fn roundtrip_pseudo_random() {
        let data: Vec<u8> = (0..4096u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        roundtrip(&data);
    }

    #[test]
    fn roundtrip_all_same_byte() {
        roundtrip(&vec![0x55u8; 4096]);
    }

    #[test]
    fn roundtrip_tiny() {
        roundtrip(&[0xDE, 0xAD, 0xBE, 0xEF]);
    }
}
