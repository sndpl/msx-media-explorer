//! Bit/byte stream decompressors, ported from RECOIL.
//!
//! [`SrStream`] is the Graph Saurus byte RLE (`0xfd`-headed `.SR*`/`.SCx`),
//! [`G9bStream`] the V9990 `.G9B` LZ-style packer.

/// A byte stream with MSB-first bit reading (`Bits` is 8 bits sliding left with
/// a trailing 1 sentinel, matching RECOIL).
struct BitStream<'a> {
    content: &'a [u8],
    pos: usize,
    bits: i32,
}

impl<'a> BitStream<'a> {
    fn new(content: &'a [u8], offset: usize) -> BitStream<'a> {
        BitStream {
            content,
            pos: offset,
            bits: 0,
        }
    }

    fn read_byte(&mut self) -> i32 {
        if self.pos >= self.content.len() {
            -1
        } else {
            let b = self.content[self.pos] as i32;
            self.pos += 1;
            b
        }
    }

    fn read_bit(&mut self) -> i32 {
        if self.bits & 0x7f == 0 {
            if self.pos >= self.content.len() {
                return -1;
            }
            self.bits = (self.content[self.pos] as i32) << 1 | 1;
            self.pos += 1;
        } else {
            self.bits <<= 1;
        }
        self.bits >> 8 & 1
    }

    fn read_bits(&mut self, count: i32) -> i32 {
        let mut result = 0;
        for _ in 0..count {
            let bit = self.read_bit();
            if bit < 0 {
                return -1;
            }
            result = result << 1 | bit;
        }
        result
    }
}

/// Graph Saurus byte RLE.
pub(super) struct SrStream<'a> {
    bs: BitStream<'a>,
    repeat_count: i32,
    repeat_value: i32,
}

impl<'a> SrStream<'a> {
    pub(super) fn new(content: &'a [u8], offset: usize) -> SrStream<'a> {
        SrStream {
            bs: BitStream::new(content, offset),
            repeat_count: 0,
            repeat_value: 0,
        }
    }

    fn read_command(&mut self) -> bool {
        let b = self.bs.read_byte();
        match b {
            -1 => false,
            0 => {
                let mut count = self.bs.read_byte();
                if count == 0 {
                    count = 256;
                }
                self.repeat_count = count;
                self.repeat_value = self.bs.read_byte();
                true
            }
            1..=15 => {
                self.repeat_count = b;
                self.repeat_value = self.bs.read_byte();
                true
            }
            _ => {
                self.repeat_count = 1;
                self.repeat_value = b;
                true
            }
        }
    }

    fn read_rle(&mut self) -> i32 {
        while self.repeat_count == 0 {
            if !self.read_command() {
                return -1;
            }
        }
        self.repeat_count -= 1;
        if self.repeat_value >= 0 {
            self.repeat_value
        } else {
            self.bs.read_byte()
        }
    }

    /// Unpack into `unpacked[offset..end]` (stride 1). Truncated input stops
    /// early, matching RECOIL's tolerance for clipped Graph Saurus files.
    pub(super) fn unpack(&mut self, unpacked: &mut [u8], offset: usize, end: usize) {
        let mut o = offset;
        while o < end {
            let b = self.read_rle();
            if b < 0 {
                return;
            }
            unpacked[o] = b as u8;
            o += 1;
        }
    }
}

/// V9990 `.G9B` LZ packer.
pub(super) struct G9bStream<'a> {
    bs: BitStream<'a>,
}

const BLOCK_END: i32 = -2;

impl<'a> G9bStream<'a> {
    pub(super) fn new(content: &'a [u8]) -> G9bStream<'a> {
        G9bStream {
            bs: BitStream::new(content, 0),
        }
    }

    fn read_length(&mut self) -> i32 {
        let mut length = 1i32;
        while length < 1 << 16 {
            match self.bs.read_bit() {
                0 => return length + 1,
                1 => {}
                _ => return -1,
            }
            length <<= 1;
            match self.bs.read_bit() {
                0 => {}
                1 => length += 1,
                _ => return -1,
            }
        }
        BLOCK_END
    }

    pub(super) fn unpack(
        &mut self,
        unpacked: &mut [u8],
        header_length: usize,
        unpacked_length: usize,
    ) -> bool {
        self.bs.pos = header_length + 3;
        let mut off = header_length;
        while off < unpacked_length {
            match self.bs.read_bit() {
                0 => {
                    let b = self.bs.read_byte();
                    if b < 0 {
                        return false;
                    }
                    unpacked[off] = b as u8;
                    off += 1;
                }
                1 => {
                    let mut length = self.read_length();
                    if length == BLOCK_END {
                        self.bs.pos += 2; // skip block length
                        self.bs.bits = 0; // reset bit buffer
                        continue;
                    }
                    if length < 0 || off + length as usize > unpacked_length {
                        return false;
                    }
                    let mut distance = self.bs.read_byte();
                    if distance < 0 {
                        return false;
                    }
                    if distance >= 128 {
                        let b = self.bs.read_bits(4);
                        if b < 0 {
                            return false;
                        }
                        distance += (b - 1) << 7;
                    }
                    distance += 1;
                    let dist = distance as usize;
                    if off < header_length + dist {
                        return false;
                    }
                    loop {
                        unpacked[off] = unpacked[off - dist];
                        off += 1;
                        length -= 1;
                        if length <= 0 {
                            break;
                        }
                    }
                }
                _ => return false,
            }
        }
        true
    }
}
