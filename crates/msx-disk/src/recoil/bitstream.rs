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

/// MSX Interchange (`.MIF`) LZW-style decompressor. Returns the unpacked bytes
/// (length `unpacked_length`) starting the bitstream at `start_offset`.
pub(super) fn unpack_mif(
    content: &[u8],
    unpacked_length: usize,
    start_offset: usize,
) -> Option<Vec<u8>> {
    let mut s = BitStream::new(content, start_offset);
    let mut unpacked = vec![0u8; unpacked_length];
    let mut offsets = vec![0usize; 16384 / 3];
    let mut codes = 0usize;
    let mut code_bits = 2i32;
    let mut off = 0usize;
    while off < unpacked_length {
        if codes >= offsets.len() {
            return None;
        }
        offsets[codes] = off;
        codes += 1;
        if codes >> code_bits != 0 {
            code_bits += 1;
        }
        match s.read_bit() {
            0 => {
                let code = s.read_bits(code_bits);
                if code < 0 || code as usize >= codes {
                    return None;
                }
                let code = code as usize;
                if code == codes - 1 {
                    codes = 0;
                    code_bits = 2;
                    continue;
                }
                let mut src = offsets[code];
                let end = offsets[code + 1];
                if off + end - src >= unpacked_length {
                    return None;
                }
                loop {
                    unpacked[off] = unpacked[src];
                    off += 1;
                    src += 1;
                    if src > end {
                        break;
                    }
                }
            }
            1 => {
                let b = s.read_bits(8);
                if b < 0 {
                    return None;
                }
                unpacked[off] = b as u8;
                off += 1;
            }
            _ => return None,
        }
    }
    if s.read_bit() != 0 || s.read_bits(code_bits) <= codes as i32 || s.pos != content.len() {
        return None;
    }
    Some(unpacked)
}

/// Maki-chan Graphics (`.MAG`) delta + flag decompressor. Fills `unpacked`
/// (`bytes_per_line * height`). Returns false on error.
pub(super) fn unpack_mag(
    content: &[u8],
    header_offset: usize,
    bytes_per_line: usize,
    height: usize,
    unpacked: &mut [u8],
) -> bool {
    use super::get32_le;
    const DELTA_X: [usize; 16] = [0, 2, 4, 8, 0, 2, 0, 2, 4, 0, 2, 4, 0, 2, 4, 0];
    const DELTA_Y: [usize; 16] = [0, 0, 0, 0, 1, 1, 2, 2, 2, 4, 4, 4, 8, 8, 8, 16];

    let mut have_delta = BitStream::new(
        content,
        header_offset + get32_le(content, header_offset + 12) as usize,
    );
    let mut delta_offset = header_offset + get32_le(content, header_offset + 16) as usize;
    let mut color_offset = header_offset + get32_le(content, header_offset + 24) as usize;

    let dsize = (bytes_per_line + 3) >> 2;
    let mut deltas = vec![0u8; dsize];
    for y in 0..height {
        let mut delta = 0i32;
        for x in 0..bytes_per_line {
            if x & 1 == 0 {
                delta = deltas[x >> 2] as i32;
                if x & 2 == 0 {
                    match have_delta.read_bit() {
                        0 => {}
                        1 => {
                            if delta_offset >= content.len() {
                                return false;
                            }
                            delta ^= content[delta_offset] as i32;
                            delta_offset += 1;
                            deltas[x >> 2] = delta as u8;
                        }
                        _ => return false,
                    }
                    delta >>= 4;
                } else {
                    delta &= 0xf;
                }
            }
            if delta == 0 {
                if color_offset >= content.len() {
                    return false;
                }
                unpacked[y * bytes_per_line + x] = content[color_offset];
                color_offset += 1;
            } else {
                let sx = x as i32 - DELTA_X[delta as usize] as i32;
                let sy = y as i32 - DELTA_Y[delta as usize] as i32;
                if sx < 0 || sy < 0 {
                    return false;
                }
                unpacked[y * bytes_per_line + x] =
                    unpacked[sy as usize * bytes_per_line + sx as usize];
            }
        }
        if bytes_per_line & 1 != 0 && delta == 0 {
            color_offset += 1;
        }
        if (bytes_per_line + 1) & 2 != 0
            && deltas.get(bytes_per_line >> 2).copied().unwrap_or(0) & 0xf == 0
        {
            color_offset += 2;
        }
    }
    true
}

/// Dynamic Publisher byte RLE (used by `.PCT`/`.FNT`).
pub(super) struct CciStream<'a> {
    bs: BitStream<'a>,
    repeat_count: i32,
    repeat_value: i32,
}

impl<'a> CciStream<'a> {
    pub(super) fn new(content: &'a [u8], offset: usize) -> CciStream<'a> {
        CciStream {
            bs: BitStream::new(content, offset),
            repeat_count: 0,
            repeat_value: 0,
        }
    }

    fn read_command(&mut self) -> bool {
        let b = self.bs.read_byte();
        if b < 0 {
            return false;
        }
        if b < 128 {
            self.repeat_count = b + 1;
            self.repeat_value = -1;
        } else {
            self.repeat_count = b - 127;
            self.repeat_value = self.bs.read_byte();
        }
        true
    }

    pub(super) fn read_rle(&mut self) -> i32 {
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
}

/// MSX Interchange (`.MIG`) LZ packer.
pub(super) struct MigStream<'a> {
    bs: BitStream<'a>,
}

impl<'a> MigStream<'a> {
    pub(super) const MAX_UNPACKED_LENGTH: usize = 108_800;

    pub(super) fn new(content: &'a [u8]) -> MigStream<'a> {
        MigStream {
            bs: BitStream::new(content, 0),
        }
    }

    /// Returns the unpacked length, or -1 on error.
    pub(super) fn unpack(&mut self, unpacked: &mut [u8]) -> i32 {
        self.bs.pos = 11 + 4;
        let mut off = 0usize;
        while off < Self::MAX_UNPACKED_LENGTH {
            let c = self.bs.read_bit();
            if c < 0 {
                return -1;
            }
            let mut b = self.bs.read_byte();
            if b < 0 {
                return -1;
            }
            if c == 0 {
                unpacked[off] = b as u8;
                off += 1;
                continue;
            }
            if b >= 128 {
                let c4 = self.bs.read_bits(4);
                if c4 < 0 {
                    return -1;
                }
                b += (c4 - 1) << 7;
            }
            let distance = (b + 1) as usize;
            if off < distance {
                return -1;
            }
            let mut bits = -1i32;
            loop {
                let bit = self.bs.read_bit();
                if bit < 0 {
                    return -1;
                }
                bits += 1;
                if bit == 0 {
                    break;
                }
            }
            let mut length = self.bs.read_bits(bits);
            if length < 0 {
                return -1;
            }
            if bits >= 16 {
                self.bs.pos += 4; // skip block lengths
                if self.bs.pos >= self.bs.content.len() {
                    return off as i32;
                }
                self.bs.bits = 0;
            } else {
                length += (1 << bits) + 1;
                if off + length as usize > Self::MAX_UNPACKED_LENGTH {
                    return -1;
                }
                loop {
                    unpacked[off] = unpacked[off - distance];
                    off += 1;
                    length -= 1;
                    if length <= 0 {
                        break;
                    }
                }
            }
        }
        -1
    }
}

/// DD-Graph (`.CMP`) two-level sparse stream.
pub(super) struct ZimStream<'a> {
    content: &'a [u8],
    pos: usize,
    flags2: [u8; 8],
}

impl<'a> ZimStream<'a> {
    pub(super) fn new(content: &'a [u8], offset: usize) -> ZimStream<'a> {
        ZimStream {
            content,
            pos: offset,
            flags2: [0; 8],
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

    pub(super) fn fully_consumed(&self) -> bool {
        self.pos == self.content.len()
    }

    /// Read the next 8 presence-flag bytes into `flags2`. Returns false on EOF.
    pub(super) fn unpack_flags2(&mut self) -> bool {
        let group = self.read_byte();
        if group < 0 {
            return false;
        }
        for i in 0..8usize {
            let present = (group >> ((!i & 7) as u32)) & 1 != 0;
            self.flags2[i] = if present {
                let v = self.read_byte();
                if v < 0 {
                    return false;
                }
                v as u8
            } else {
                0
            };
        }
        true
    }

    /// Read one unpacked byte for position `flags_offset` (0..64), or -1 on EOF.
    pub(super) fn read_unpacked(&mut self, flags_offset: usize) -> i32 {
        let present = (self.flags2[flags_offset >> 3] >> ((!flags_offset & 7) as u32)) & 1 != 0;
        if present {
            self.read_byte()
        } else {
            0
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
