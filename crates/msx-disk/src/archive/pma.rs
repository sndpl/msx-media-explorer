//! PMarc `-pm1-` / `-pm2-` decompression.
//!
//! `delharc` recognizes these method tags but has no decoder for them, yet they
//! are the methods most real `.pma` files use. The decoders here are a faithful
//! Rust port of Simon Howard's `lhasa` (`lib/pm1_decoder.c`,
//! `lib/pm2_decoder.c`, `lib/pma_common.c`, `lib/tree_decode.c`,
//! `lib/bit_stream_reader.c`), which is ISC-licensed.
//!
//! Both formats are LZSS variants over an 8 KiB (pm2) or 16 KiB (pm1) ring
//! buffer pre-filled with spaces, with byte values encoded against a
//! move-to-front "history" linked list and static/adaptive Huffman-style codes.

use crate::error::{Error, Result};

/// Whether this module can decompress the given method tag.
pub(super) fn supports(method: &[u8; 5]) -> bool {
    matches!(method, b"-pm1-" | b"-pm2-")
}

/// Decompress a PMarc member. `original_size` is the expected output length.
pub(super) fn decompress(method: &[u8; 5], comp: &[u8], original_size: usize) -> Result<Vec<u8>> {
    let decoded = match method {
        b"-pm1-" => decompress_pm1(comp, original_size),
        b"-pm2-" => decompress_pm2(comp, original_size),
        _ => None,
    };
    match decoded {
        Some(mut out) => {
            out.truncate(original_size);
            Ok(out)
        }
        None => Err(Error::Malformed(format!(
            "could not decode PMarc {} stream",
            std::str::from_utf8(method).unwrap_or("?????")
        ))),
    }
}

// ---------------------------------------------------------------------------
// Bit stream reader (port of bit_stream_reader.c)
// ---------------------------------------------------------------------------

/// MSB-first bit reader over an in-memory slice. With `pad_zeros` set (used by
/// pm1), reads past the end yield zero bits rather than signalling end-of-file
/// — some archives rely on reading slightly beyond the compressed data.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buffer: u32,
    bits: u32,
    pad_zeros: bool,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], pad_zeros: bool) -> Self {
        BitReader {
            data,
            pos: 0,
            bit_buffer: 0,
            bits: 0,
            pad_zeros,
        }
    }

    fn peek_bits(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        while self.bits < n {
            let fill_bytes = (32 - self.bits) / 8;
            let mut got = 0u32;
            for _ in 0..fill_bytes {
                let byte = if self.pos < self.data.len() {
                    let b = self.data[self.pos];
                    self.pos += 1;
                    b
                } else if self.pad_zeros {
                    0
                } else {
                    break;
                };
                self.bit_buffer |= (byte as u32) << (24 - self.bits);
                self.bits += 8;
                got += 1;
            }
            if got == 0 {
                return None;
            }
        }
        Some(self.bit_buffer >> (32 - n))
    }

    fn read_bits(&mut self, n: u32) -> Option<u32> {
        let result = self.peek_bits(n)?;
        self.bit_buffer <<= n;
        self.bits -= n;
        Some(result)
    }

    fn read_bit(&mut self) -> Option<u32> {
        self.read_bits(1)
    }
}

// ---------------------------------------------------------------------------
// Common PMarc structures (port of pma_common.c)
// ---------------------------------------------------------------------------

/// A variable-length code: `offset + read_bits(bits)`.
type Vlt = (u32, u32);

fn decode_variable_length(reader: &mut BitReader, table: &[Vlt], header: usize) -> Option<u32> {
    let (offset, bits) = table[header];
    Some(offset + reader.read_bits(bits)?)
}

#[derive(Clone, Copy)]
struct HistoryNode {
    prev: u8,
    next: u8,
}

/// Move-to-front linked list over all 256 byte values. Codes in the stream are
/// distances back along this list rather than literal bytes, so recently used
/// bytes encode in fewer bits.
struct HistoryList {
    history: [HistoryNode; 256],
    head: u8,
}

impl HistoryList {
    fn new() -> Self {
        let mut history = [HistoryNode { prev: 0, next: 0 }; 256];
        for (i, node) in history.iter_mut().enumerate() {
            node.prev = ((i + 1) % 256) as u8;
            node.next = ((i + 255) % 256) as u8;
        }
        let mut list = HistoryList { history, head: 0x20 };

        // The chain is cut into groups so that printable ASCII is closest to
        // the start, followed by control characters, then other groups.
        list.history[0x7f].prev = 0x00;
        list.history[0x00].next = 0x7f;
        list.history[0x1f].prev = 0xa0;
        list.history[0xa0].next = 0x1f;
        list.history[0xdf].prev = 0x80;
        list.history[0x80].next = 0xdf;
        list.history[0x9f].prev = 0xe0;
        list.history[0xe0].next = 0x9f;
        list.history[0xff].prev = 0x20;
        list.history[0x20].next = 0xff;
        list
    }

    fn find(&self, count: u8) -> u8 {
        let mut code = self.head;
        if count < 128 {
            for _ in 0..count {
                code = self.history[code as usize].prev;
            }
        } else {
            for _ in 0..(256 - count as u32) {
                code = self.history[code as usize].next;
            }
        }
        code
    }

    fn update(&mut self, b: u8) {
        if self.head == b {
            return;
        }
        // Unhook from current position.
        let (prev, next) = {
            let node = self.history[b as usize];
            (node.prev, node.next)
        };
        self.history[next as usize].prev = prev;
        self.history[prev as usize].next = next;

        // Hook in between the old head and old_head.next.
        let old_head = self.head;
        let old_next = self.history[old_head as usize].next;
        self.history[b as usize].prev = old_head;
        self.history[b as usize].next = old_next;
        self.history[old_next as usize].prev = b;
        self.history[old_head as usize].next = b;
        self.head = b;
    }
}

// ---------------------------------------------------------------------------
// Tree decoder (port of tree_decode.c, with TreeElement = u8)
// ---------------------------------------------------------------------------

const TREE_NODE_LEAF: u8 = 1 << 7;

fn init_tree(tree: &mut [u8]) {
    for e in tree.iter_mut() {
        *e = TREE_NODE_LEAF;
    }
}

fn set_tree_single(tree: &mut [u8], code: u8) {
    tree[0] = code | TREE_NODE_LEAF;
}

fn build_tree(tree: &mut [u8], code_lengths: &[u8]) {
    let tree_len = tree.len();
    let mut next_entry = 0usize;
    let mut tree_allocated = 1usize;
    let mut code_len = 0u32;

    loop {
        // expand_queue: add a level to the tree.
        let new_nodes = (tree_allocated - next_entry) * 2;
        if tree_allocated + new_nodes <= tree_len {
            let end_offset = tree_allocated;
            while next_entry < end_offset {
                tree[next_entry] = tree_allocated as u8;
                tree_allocated += 2;
                next_entry += 1;
            }
        }
        code_len += 1;

        // add_codes_with_length.
        let mut codes_remaining = false;
        for (i, &cl) in code_lengths.iter().enumerate() {
            if cl as u32 == code_len {
                // read_next_entry (returns 0 on overflow, matching lhasa).
                let node = if next_entry < tree_allocated {
                    let r = next_entry;
                    next_entry += 1;
                    r
                } else {
                    0
                };
                tree[node] = (i as u8) | TREE_NODE_LEAF;
            } else if cl as u32 > code_len {
                codes_remaining = true;
            }
        }

        if !codes_remaining {
            break;
        }
    }
}

fn read_from_tree(reader: &mut BitReader, tree: &[u8]) -> Option<u32> {
    let mut code = tree[0];
    while code & TREE_NODE_LEAF == 0 {
        let bit = reader.read_bit()?;
        code = tree[code as usize + bit as usize];
    }
    Some((code & !TREE_NODE_LEAF) as u32)
}

// ---------------------------------------------------------------------------
// pm2 decoder (port of pm2_decoder.c)
// ---------------------------------------------------------------------------

const PM2_RING: usize = 8192;
const PM2_CODE_TREE_ELEMENTS: usize = 65;
const PM2_OFFSET_TREE_ELEMENTS: usize = 17;

#[derive(Clone, Copy, PartialEq)]
enum Pm2Rebuild {
    Unbuilt,
    Build1,
    Build2,
    Build3,
    Continuing,
}

const PM2_HISTORY_DECODE: &[Vlt] = &[
    (0, 3),
    (8, 3),
    (16, 4),
    (32, 5),
    (64, 5),
    (96, 5),
    (128, 6),
    (192, 6),
];

const PM2_COPY_DECODE: &[Vlt] = &[(17, 3), (25, 3), (33, 5), (65, 6), (129, 7), (256, 0)];

struct Pm2<'a> {
    reader: BitReader<'a>,
    tree_state: Pm2Rebuild,
    tree_rebuild_remaining: usize,
    ringbuf: [u8; PM2_RING],
    ringbuf_pos: u32,
    history: HistoryList,
    code_tree: [u8; PM2_CODE_TREE_ELEMENTS],
    need_offset_tree: bool,
    offset_tree: [u8; PM2_OFFSET_TREE_ELEMENTS],
}

impl<'a> Pm2<'a> {
    fn new(comp: &'a [u8]) -> Self {
        let mut d = Pm2 {
            reader: BitReader::new(comp, false),
            tree_state: Pm2Rebuild::Unbuilt,
            tree_rebuild_remaining: 0,
            ringbuf: [b' '; PM2_RING],
            ringbuf_pos: 0,
            history: HistoryList::new(),
            code_tree: [0; PM2_CODE_TREE_ELEMENTS],
            need_offset_tree: false,
            offset_tree: [0; PM2_OFFSET_TREE_ELEMENTS],
        };
        init_tree(&mut d.code_tree);
        init_tree(&mut d.offset_tree);
        d
    }

    fn read_code_tree(&mut self) -> Option<()> {
        let num_codes = self.reader.read_bits(5)? as i32;
        let min_code_length = self.reader.read_bits(3)? as i32;

        // Reject overlong code counts (GHSA-j2m3-h278-rrg9).
        if num_codes > 29 {
            return Some(());
        }

        self.need_offset_tree =
            num_codes >= 10 && !(num_codes == 29 && min_code_length == 0);

        if min_code_length == 0 {
            set_tree_single(&mut self.code_tree, (num_codes - 1) as u8);
            return Some(());
        }

        let length_bits = self.reader.read_bits(3)?;
        let mut code_lengths = [0u8; 31];
        for slot in code_lengths.iter_mut().take(num_codes as usize) {
            let val = self.reader.read_bits(length_bits)?;
            *slot = if val == 0 {
                0
            } else {
                (min_code_length as u32 + val - 1) as u8
            };
        }
        build_tree(&mut self.code_tree, &code_lengths[..num_codes as usize]);
        Some(())
    }

    fn read_offset_tree(&mut self, num_offsets: usize) -> Option<()> {
        if !self.need_offset_tree {
            return Some(());
        }
        let mut offset_lengths = [0u8; 8];
        let mut num_codes = 0u32;
        let mut single_offset = 0usize;
        for (off, slot) in offset_lengths.iter_mut().enumerate().take(num_offsets) {
            let len = self.reader.read_bits(3)?;
            *slot = len as u8;
            if len != 0 {
                single_offset = off;
                num_codes += 1;
            }
        }
        if num_codes == 1 {
            set_tree_single(&mut self.offset_tree, single_offset as u8);
            return Some(());
        }
        build_tree(&mut self.offset_tree, &offset_lengths[..num_offsets]);
        Some(())
    }

    fn rebuild_tree(&mut self) {
        match self.tree_state {
            Pm2Rebuild::Unbuilt => {
                let _ = self.read_code_tree();
                let _ = self.read_offset_tree(5);
                self.tree_state = Pm2Rebuild::Build1;
                self.tree_rebuild_remaining = 1024;
            }
            Pm2Rebuild::Build1 => {
                let _ = self.read_offset_tree(6);
                self.tree_state = Pm2Rebuild::Build2;
                self.tree_rebuild_remaining = 1024;
            }
            Pm2Rebuild::Build2 => {
                let _ = self.read_offset_tree(7);
                self.tree_state = Pm2Rebuild::Build3;
                self.tree_rebuild_remaining = 2048;
            }
            Pm2Rebuild::Build3 => {
                if self.reader.read_bit() == Some(1) {
                    let _ = self.read_code_tree();
                }
                let _ = self.read_offset_tree(8);
                self.tree_state = Pm2Rebuild::Continuing;
                self.tree_rebuild_remaining = 4096;
            }
            Pm2Rebuild::Continuing => {
                if self.reader.read_bit() == Some(1) {
                    let _ = self.read_code_tree();
                    let _ = self.read_offset_tree(8);
                }
                self.tree_rebuild_remaining = 4096;
            }
        }
    }

    fn output(&mut self, out: &mut Vec<u8>, b: u8) {
        self.ringbuf[self.ringbuf_pos as usize] = b;
        self.ringbuf_pos = (self.ringbuf_pos + 1) % PM2_RING as u32;
        out.push(b);
        self.history.update(b);
        self.tree_rebuild_remaining -= 1;
        if self.tree_rebuild_remaining == 0 {
            self.rebuild_tree();
        }
    }

    fn read_single_byte(&mut self, code: usize, out: &mut Vec<u8>) {
        if let Some(offset) = decode_variable_length(&mut self.reader, PM2_HISTORY_DECODE, code) {
            let b = self.history.find(offset as u8);
            self.output(out, b);
        }
    }

    fn history_get_count(&mut self, code: u32) -> Option<u32> {
        if code < 15 {
            Some(code + 2)
        } else {
            decode_variable_length(&mut self.reader, PM2_COPY_DECODE, (code - 15) as usize)
        }
    }

    fn history_get_offset(&mut self, code: u32) -> Option<u32> {
        let mut result = 0u32;
        let bits;
        if code == 0 {
            bits = 6;
        } else if code < 20 {
            let val = read_from_tree(&mut self.reader, &self.offset_tree)?;
            if val == 0 {
                bits = 6;
            } else {
                bits = val + 5;
                result = 1 << bits;
            }
        } else {
            return Some(0);
        }
        result += self.reader.read_bits(bits)?;
        Some(result)
    }

    fn copy_from_history(&mut self, code: u32, out: &mut Vec<u8>) {
        let (to_copy, offset) = match (self.history_get_count(code), self.history_get_offset(code)) {
            (Some(c), Some(o)) => (c, o),
            _ => return,
        };
        if to_copy > 256 {
            return;
        }
        let start = self
            .ringbuf_pos
            .wrapping_add(PM2_RING as u32)
            .wrapping_sub(1)
            .wrapping_sub(offset);
        for i in 0..to_copy {
            let pos = (start.wrapping_add(i) % PM2_RING as u32) as usize;
            let b = self.ringbuf[pos];
            self.output(out, b);
        }
    }
}

fn decompress_pm2(comp: &[u8], original_size: usize) -> Option<Vec<u8>> {
    let mut d = Pm2::new(comp);
    let mut out = Vec::with_capacity(original_size);
    if original_size == 0 {
        return Some(out);
    }

    // First bit of the stream is discarded, then the initial trees are built.
    d.reader.read_bit();
    d.rebuild_tree();

    while out.len() < original_size {
        let before = out.len();
        let code = read_from_tree(&mut d.reader, &d.code_tree)?;
        if code < 8 {
            d.read_single_byte(code as usize, &mut out);
        } else {
            d.copy_from_history(code - 8, &mut out);
        }
        if out.len() == before {
            return None;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// pm1 decoder (port of pm1_decoder.c)
// ---------------------------------------------------------------------------

const PM1_RING: usize = 16384;
const PM1_MAX_BYTE_BLOCK_LEN: usize = 216;

const PM1_COPY_RANGES: &[Vlt] = &[
    (0, 6),
    (64, 8),
    (0, 6),
    (64, 9),
    (576, 11),
    (2624, 13),
    // Limited early-stream ranges, redirected to by output position:
    (64, 8),
    (576, 8),
    (576, 9),
    (576, 10),
    (2624, 8),
    (2624, 9),
    (2624, 10),
    (2624, 11),
    (2624, 12),
];

const PM1_BYTE_RANGES: &[Vlt] = &[(0, 4), (16, 4), (32, 5), (64, 6), (128, 6), (192, 6)];

// Each row is a mini binary tree over byte_ranges indices. Each nibble is
// either a leaf value (0x0a..0x0f -> 0..5) or a byte offset to a child node.
const PM1_BYTE_DECODE_TREES: [[u8; 5]; 32] = [
    [0x12, 0x2d, 0xef, 0x1c, 0xab],
    [0x12, 0x23, 0xde, 0xab, 0xcf],
    [0x12, 0x2c, 0xd2, 0xab, 0xef],
    [0x12, 0xa2, 0xd2, 0xbc, 0xef],
    [0x12, 0xa2, 0xc2, 0xbd, 0xef],
    [0x12, 0xa2, 0xcd, 0xb1, 0xef],
    [0x12, 0xab, 0x12, 0xcd, 0xef],
    [0x12, 0xab, 0x1d, 0xc1, 0xef],
    [0x12, 0xab, 0xc1, 0xd1, 0xef],
    [0xa1, 0x12, 0x2c, 0xde, 0xbf],
    [0xa1, 0x1d, 0x1c, 0xb1, 0xef],
    [0xa1, 0x12, 0x2d, 0xef, 0xbc],
    [0xa1, 0x12, 0xb2, 0xde, 0xcf],
    [0xa1, 0x12, 0xbc, 0xd1, 0xef],
    [0xa1, 0x1c, 0xb1, 0xd1, 0xef],
    [0xa1, 0xb1, 0x12, 0xcd, 0xef],
    [0xa1, 0xb1, 0xc1, 0xd1, 0xef],
    [0x12, 0x1c, 0xde, 0xab, 0x00],
    [0x12, 0xa2, 0xcd, 0xbe, 0x00],
    [0x12, 0xab, 0xc1, 0xde, 0x00],
    [0xa1, 0x1d, 0x1c, 0xbe, 0x00],
    [0xa1, 0x12, 0xbc, 0xde, 0x00],
    [0xa1, 0x1c, 0xb1, 0xde, 0x00],
    [0xa1, 0xb1, 0xc1, 0xde, 0x00],
    [0x1d, 0x1c, 0xab, 0x00, 0x00],
    [0x1c, 0xa1, 0xbd, 0x00, 0x00],
    [0x12, 0xab, 0xcd, 0x00, 0x00],
    [0xa1, 0x1c, 0xbd, 0x00, 0x00],
    [0xa1, 0xb1, 0xcd, 0x00, 0x00],
    [0xa1, 0xbc, 0x00, 0x00, 0x00],
    [0xab, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00, 0x00, 0x00],
];

struct Pm1<'a> {
    reader: BitReader<'a>,
    output_stream_pos: u32,
    byte_decode_tree: Option<[u8; 5]>,
    ringbuf: [u8; PM1_RING],
    ringbuf_pos: u32,
    history: HistoryList,
}

impl<'a> Pm1<'a> {
    fn new(comp: &'a [u8]) -> Self {
        Pm1 {
            reader: BitReader::new(comp, true),
            output_stream_pos: 0,
            byte_decode_tree: None,
            ringbuf: [b' '; PM1_RING],
            ringbuf_pos: 0,
            history: HistoryList::new(),
        }
    }

    fn read_start_header(&mut self) -> Option<()> {
        let index = self.reader.read_bits(5)? as usize;
        self.byte_decode_tree = Some(PM1_BYTE_DECODE_TREES[index]);
        Some(())
    }

    fn output(&mut self, out: &mut Vec<u8>, b: u8) {
        self.ringbuf[self.ringbuf_pos as usize] = b;
        self.ringbuf_pos = (self.ringbuf_pos + 1) % PM1_RING as u32;
        self.history.update(b);
        self.output_stream_pos += 1;
        out.push(b);
    }

    fn read_copy_byte_count(&mut self) -> Option<u32> {
        let x = self.reader.read_bits(2)?;
        if x < 3 {
            return Some(x + 3);
        }
        let x = self.reader.read_bits(3)?;
        if x < 5 {
            Some(x + 6)
        } else if x == 5 {
            Some(self.reader.read_bits(2)? + 11)
        } else if x == 6 {
            Some(self.reader.read_bits(3)? + 15)
        } else {
            let x = self.reader.read_bits(6)?;
            if x < 62 {
                Some(x + 23)
            } else if x == 62 {
                Some(self.reader.read_bits(5)? + 85)
            } else {
                Some(self.reader.read_bits(7)? + 117)
            }
        }
    }

    fn read_bit_after_threshold(&mut self, threshold: u32, def: u32) -> Option<u32> {
        if self.output_stream_pos >= threshold {
            self.reader.read_bit()
        } else {
            Some(def)
        }
    }

    fn read_copy_type_range(&mut self) -> Option<u32> {
        let x = self.reader.read_bit()?;
        if x == 0 {
            let y = self.read_bit_after_threshold(576, 0)?;
            if y != 0 {
                Some(4)
            } else {
                self.read_bit_after_threshold(64, 0)
            }
        } else {
            let y = self.read_bit_after_threshold(64, 1)?;
            if y == 0 {
                return Some(3);
            }
            let z = self.read_bit_after_threshold(2624, 1)?;
            if z != 0 {
                Some(2)
            } else {
                Some(5)
            }
        }
    }

    fn read_copy_command(&mut self, out: &mut Vec<u8>) {
        let range_index = match self.read_copy_type_range() {
            Some(r) => r,
            None => return,
        };

        let count = if range_index < 2 {
            2
        } else {
            match self.read_copy_byte_count() {
                Some(c) => c,
                None => return,
            }
        };

        // Early in the stream, some history ranges are inaccessible, so a
        // narrower table entry (fewer bits) is used instead.
        let mut range_index = range_index as usize;
        if range_index == 3 {
            if self.output_stream_pos < 320 {
                range_index = 6;
            }
        } else if range_index == 4 {
            if self.output_stream_pos < 832 {
                range_index = 7;
            } else if self.output_stream_pos < 1088 {
                range_index = 8;
            } else if self.output_stream_pos < 1600 {
                range_index = 9;
            }
        } else if range_index == 5 {
            if self.output_stream_pos < 2880 {
                range_index = 10;
            } else if self.output_stream_pos < 3136 {
                range_index = 11;
            } else if self.output_stream_pos < 3648 {
                range_index = 12;
            } else if self.output_stream_pos < 4672 {
                range_index = 13;
            } else if self.output_stream_pos < 6720 {
                range_index = 14;
            }
        }

        let history_distance =
            match decode_variable_length(&mut self.reader, PM1_COPY_RANGES, range_index) {
                Some(d) => d,
                None => return,
            };
        if history_distance >= self.output_stream_pos {
            return;
        }

        let mut copy_index = self
            .ringbuf_pos
            .wrapping_add(PM1_RING as u32)
            .wrapping_sub(history_distance)
            .wrapping_sub(1)
            % PM1_RING as u32;
        for _ in 0..count {
            let b = self.ringbuf[copy_index as usize];
            self.output(out, b);
            copy_index = (copy_index + 1) % PM1_RING as u32;
        }
    }

    fn read_byte_decode_index(&mut self) -> Option<u32> {
        let tree = self.byte_decode_tree.as_ref().unwrap();
        if tree[0] == 0 {
            return Some(0);
        }
        let mut ptr = 0usize;
        loop {
            let bit = self.reader.read_bit()?;
            let child = if bit == 0 {
                (tree[ptr] >> 4) & 0x0f
            } else {
                tree[ptr] & 0x0f
            };
            if child >= 10 {
                return Some((child - 10) as u32);
            }
            ptr += child as usize;
        }
    }

    fn read_byte(&mut self) -> Option<u8> {
        let index = self.read_byte_decode_index()? as usize;
        let count = decode_variable_length(&mut self.reader, PM1_BYTE_RANGES, index)?;
        Some(self.history.find(count as u8))
    }

    fn read_byte_block_count(&mut self) -> Option<u32> {
        let x = self.reader.read_bits(2)?;
        if x < 3 {
            return Some(x + 1);
        }
        let x = self.reader.read_bits(3)?;
        if x < 7 {
            return Some(x + 4);
        }
        let x = self.reader.read_bits(4)?;
        if x < 14 {
            Some(x + 11)
        } else if x == 14 {
            Some(self.reader.read_bits(6)? + 25)
        } else {
            Some(self.reader.read_bits(7)? + 89)
        }
    }

    fn read_byte_block(&mut self, out: &mut Vec<u8>) {
        let block_len = match self.read_byte_block_count() {
            Some(n) => n as usize,
            None => return,
        };
        for _ in 0..block_len {
            match self.read_byte() {
                Some(b) => self.output(out, b),
                None => return,
            }
        }
        // A byte block is normally followed by a copy command, unless the block
        // hit the maximum length (then it ended only because it could not grow).
        if block_len == PM1_MAX_BYTE_BLOCK_LEN {
            return;
        }
        self.read_copy_command(out);
    }
}

fn decompress_pm1(comp: &[u8], original_size: usize) -> Option<Vec<u8>> {
    let mut d = Pm1::new(comp);
    let mut out = Vec::with_capacity(original_size);
    if original_size == 0 {
        return Some(out);
    }
    d.read_start_header()?;

    while out.len() < original_size {
        let before = out.len();
        let command_type = d.reader.read_bit()?;
        if command_type == 0 {
            d.read_copy_command(&mut out);
        } else {
            d.read_byte_block(&mut out);
        }
        if out.len() == before {
            return None;
        }
    }
    Some(out)
}
