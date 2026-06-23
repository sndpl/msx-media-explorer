//! MSX paint-tool formats, ported from RECOIL: Dynamic Publisher (`.PCT`/`.FNT`),
//! DD-Graph (`.CMP`), and MSX Interchange (`.MIF`/`.MIG`).

use super::bitstream::{unpack_mag, unpack_mif, CciStream, MigStream, ZimStream};
use super::{get32_le, is_string_at, Recoil, Resolution};

fn get_mig_mode(reg0: i32, reg1: i32, reg19: i32, length: i32) -> i32 {
    (reg0 & 0x0e) | (reg1 & 0x18) << 1 | (reg19 & 0x18) << 3 | length << 8
}

impl Recoil<'_> {
    /// MSX colour restriction for a palette entry (3 bits/component).
    fn restrict_platform_color(&self, rgb: i32) -> i32 {
        let rgb = rgb & 0xe0e0e0;
        rgb | rgb >> 3 | (rgb >> 6 & 0x030303)
    }

    /// Load a 3-byte-per-entry palette (`.PI`/`.MAG`); `r_offset` selects the
    /// R/G byte order.
    fn set_pi_palette(&mut self, content: &[u8], offset: usize, colors: usize, r_offset: usize) {
        for c in 0..colors {
            let o = offset + c * 3;
            let rgb = (content[o + r_offset] as i32) << 16
                | (content[o + 1 - r_offset] as i32) << 8
                | content[o + 2] as i32;
            self.content_palette[c] = self.restrict_platform_color(rgb);
        }
    }

    /// Maki-chan Graphics (`.MAG`/`.MKI`/`.MAX`) — MSX modes only.
    pub(super) fn decode_mag(&mut self, content: &[u8]) -> bool {
        if content.len() < 8 || !is_string_at(content, 0, b"MAKI02  ") {
            // MAKI01 is PC-98/X68000, not MSX.
            return false;
        }
        let mut header_offset = 0usize;
        loop {
            if header_offset >= content.len() {
                return false;
            }
            let c = content[header_offset];
            header_offset += 1;
            if c == 0x1a {
                break;
            }
        }
        if header_offset + (32 + 16 * 3) > content.len() || content[header_offset] != 0 {
            return false;
        }
        let left =
            content[header_offset + 4] as usize + ((content[header_offset + 5] as usize) << 8);
        let mut width =
            content[header_offset + 8] as usize + ((content[header_offset + 9] as usize) << 8) + 1;
        let bytes_per_line;
        let colors;
        if content[header_offset + 3] < 0x80 {
            width -= left & !7;
            bytes_per_line = (width + 1) >> 1;
            colors = 16;
        } else {
            if header_offset + (32 + 256 * 3) >= content.len() {
                return false;
            }
            width -= left & !3;
            bytes_per_line = width;
            colors = 256;
        }

        if content[header_offset + 1] != 0x03 {
            // Non-MSX platform (PC-88/PC-98/X68000/Mac) — out of scope.
            return false;
        }
        let msx_mode = (content[header_offset + 2] & 0xfc) as i32;
        let resolution = match msx_mode {
            0x00 | 0x14 | 0x54 => Resolution::Msx21x1,
            0x04 => Resolution::Msx21x2,
            0x10 | 0x50 => Resolution::Msx22x1i,
            0x20 | 0x40 => {
                if colors == 16 {
                    width >>= 1;
                }
                Resolution::Msx2Plus2x1i
            }
            0x24 | 0x44 => {
                if colors == 16 {
                    width >>= 1;
                }
                Resolution::Msx2Plus1x1
            }
            0x60 => {
                width = bytes_per_line << 2;
                Resolution::Msx21x1i
            }
            0x64 => {
                width = bytes_per_line << 2;
                Resolution::Msx21x2
            }
            _ => return false,
        };
        let height = content[header_offset + 10] as i32 - content[header_offset + 6] as i32
            + ((content[header_offset + 11] as i32 - content[header_offset + 7] as i32) << 8)
            + 1;
        if height < 1 {
            return false;
        }
        let height = height as usize;
        if !self.set_scaled_size(width, height, resolution) {
            return false;
        }
        let mut unpacked = vec![0u8; bytes_per_line * height];
        if !unpack_mag(
            content,
            header_offset,
            bytes_per_line,
            height,
            &mut unpacked,
        ) {
            return false;
        }
        self.set_pi_palette(content, header_offset + 32, colors, 1);
        match msx_mode {
            0x20 | 0x24 => self.decode_msx_yjk_screen(&unpacked, 0, true),
            0x40 | 0x44 => self.decode_msx_yjk_screen(&unpacked, 0, false),
            0x60 | 0x64 => self.decode_msx6(&unpacked, 0),
            _ => {
                if colors == 16 {
                    self.decode_nibbles(&unpacked, 0, bytes_per_line);
                } else {
                    self.decode_bytes(&unpacked, 0);
                }
            }
        }
        true
    }
    /// Dynamic Publisher screen (`.PCT`) or font (`.FNT`): 1-bit, RLE-compressed.
    pub(super) fn decode_pct(&mut self, content: &[u8]) -> bool {
        if content.len() < 384
            || (!is_string_at(content, 0, b"DYNAMIC") && !is_string_at(content, 0, b"E U R O"))
            || !is_string_at(content, 7, b" PUBLISHER ")
        {
            return false;
        }
        let (height, offset) = if is_string_at(content, 18, b"SCREEN") {
            (704usize, 0x180usize)
        } else if is_string_at(content, 18, b"FONT") {
            (160, 0x200)
        } else {
            return false;
        };
        self.set_size(512, height << 1, Resolution::Msx21x2);
        let mut rle = CciStream::new(content, offset);
        for y in 0..height {
            let mut b = 0i32;
            for x in 0..512usize {
                if x & 7 == 0 {
                    b = rle.read_rle();
                    if b < 0 {
                        return false;
                    }
                }
                let o = (y << 10) + x;
                let rgb = if (b >> (((x ^ 3) & 7) as u32)) & 1 == 0 {
                    0xffffff
                } else {
                    0
                };
                self.pixels[o] = rgb;
                self.pixels[o + 512] = rgb;
            }
        }
        true
    }

    /// DD-Graph (`.CMP`): SCREEN 5, delta + sparse-RLE compressed, `.PL5` palette.
    pub(super) fn decode_dd_graph(&mut self, content: &[u8]) -> bool {
        if content.len() < 4 {
            return false;
        }
        let width = content[0] as usize;
        if width == 0 || width > 128 {
            return false;
        }
        let height = content[1] as usize;
        if height == 0 || height > 212 {
            return false;
        }
        let verbatim_lines = content[2] as usize;
        if verbatim_lines == 0 || verbatim_lines > height {
            return false;
        }
        let mut stream = ZimStream::new(content, 3);
        let mut flags_offset = 64usize;
        let mut unpacked = vec![0u8; 212 * 128];
        let mut uo = 0usize;
        for y in 0..height {
            for _x in 0..width {
                if flags_offset >= 64 {
                    if !stream.unpack_flags2() {
                        return false;
                    }
                    flags_offset = 0;
                }
                let mut b = stream.read_unpacked(flags_offset);
                flags_offset += 1;
                if b < 0 {
                    return false;
                }
                if y >= verbatim_lines {
                    b ^= unpacked[uo - width] as i32;
                }
                unpacked[uo] = b as u8;
                uo += 1;
            }
        }
        if !stream.fully_consumed() {
            return false;
        }
        self.set_msx_companion_palette("pl5");
        self.set_size(width << 1, height, Resolution::Msx21x1);
        self.decode_nibbles(&unpacked, 0, width);
        true
    }

    /// MSX Interchange Format (`.MIF`): LZW-compressed SCREEN 2/3/5/6/7/8/10/12,
    /// optionally interlaced.
    pub(super) fn decode_mif(&mut self, content: &[u8]) -> bool {
        if content.len() < 35 {
            return false;
        }
        // (unpacked_length, has_palette)
        let plan: (usize, bool) = match content[0] {
            0 | 1 => (27136, true),
            2 | 4 => (54272, true),
            3 | 5 => (54272, false),
            8 => (14336, true),
            9 => (1536, true),
            0x10 | 0x11 => (2 * 27136, true),
            0x12 | 0x14 => (2 * 54272, true),
            0x13 | 0x15 => (2 * 54272, false),
            _ => return false,
        };
        let (unpacked_length, has_palette) = plan;
        let start = if has_palette { 34 } else { 2 };
        let Some(u) = unpack_mif(content, unpacked_length, start) else {
            return false;
        };
        if has_palette {
            self.set_msx_palette(content, 2, 16);
        }
        match content[0] {
            0 => {
                self.set_size(256, 212, Resolution::Msx21x1);
                self.decode_nibbles(&u, 0, 128);
            }
            1 => {
                self.set_size(512, 424, Resolution::Msx21x2);
                self.decode_msx6(&u, 0);
            }
            2 => {
                self.set_size(512, 424, Resolution::Msx21x2);
                self.decode_nibbles(&u, 0, 256);
            }
            3 => {
                self.set_sc8_palette();
                self.set_size(256, 212, Resolution::Msx21x1);
                self.decode_bytes(&u, 0);
            }
            4 => {
                self.set_size(256, 212, Resolution::Msx2Plus1x1);
                self.decode_msx_yjk_screen(&u, 0, true);
            }
            5 => {
                self.set_size(256, 212, Resolution::Msx2Plus1x1);
                self.decode_msx_yjk_screen(&u, 0, false);
            }
            8 => self.decode_sc2sc4(&u, 0, Resolution::Msx21x1),
            9 => self.decode_sc3_screen(&u, 0, false),
            0x10 => {
                self.set_size(512, 424, Resolution::Msx22x1i);
                self.decode_nibbles(&u, 0, 128);
            }
            0x11 => {
                self.set_size(512, 424, Resolution::Msx21x1i);
                self.decode_msx6(&u, 0);
            }
            0x12 => {
                self.set_size(512, 424, Resolution::Msx21x1i);
                self.decode_nibbles(&u, 0, 256);
            }
            0x13 => {
                self.set_sc8_palette();
                self.set_size(512, 424, Resolution::Msx22x1i);
                self.decode_bytes(&u, 0);
            }
            0x14 => {
                self.set_size(512, 424, Resolution::Msx2Plus2x1i);
                self.decode_msx_yjk_screen(&u, 0, true);
            }
            0x15 => {
                self.set_size(512, 424, Resolution::Msx2Plus2x1i);
                self.decode_msx_yjk_screen(&u, 0, false);
            }
            _ => return false,
        }
        true
    }

    /// MSX Interchange `.MIG`: an LZ stream of VDP register/palette/bitmap chunks.
    pub(super) fn decode_mig(&mut self, content: &[u8]) -> bool {
        if content.len() < 16
            || !is_string_at(content, 0, b"MSXMIG")
            || get32_le(content, 6) as usize != content.len() - 6
        {
            return false;
        }
        let mut unpacked = vec![0u8; MigStream::MAX_UNPACKED_LENGTH];
        let unpacked_length = MigStream::new(content).unpack(&mut unpacked);
        if unpacked_length < 0 {
            return false;
        }
        let unpacked_length = unpacked_length as usize;

        let mut colors = 0usize;
        let mut registers = [0u8; 256];
        let mut uo = 0usize;
        while uo < unpacked_length {
            match unpacked[uo] {
                0 => {
                    // VDP register writes
                    if uo + 1 >= unpacked_length {
                        return false;
                    }
                    let count = unpacked[uo + 1] as usize;
                    if uo + 2 + count * 3 > unpacked_length {
                        return false;
                    }
                    for i in 0..count {
                        let o = uo + 2 + i * 3;
                        let r = unpacked[o] as usize;
                        let m = unpacked[o + 2];
                        registers[r] = (registers[r] & !m) | (unpacked[o + 1] & m);
                    }
                    uo += 2 + count * 3;
                }
                1 => {
                    // palette
                    if uo + 2 >= unpacked_length || unpacked[uo + 1] != 0 {
                        return false;
                    }
                    colors = unpacked[uo + 2] as usize;
                    if uo + 3 + (colors << 1) > unpacked_length {
                        return false;
                    }
                    self.set_msx_palette(&unpacked, uo + 3, colors);
                    uo += 3 + (colors << 1);
                }
                2 => {
                    // bitmap
                    if uo + 7 >= unpacked_length
                        || unpacked[uo + 1] != 0
                        || unpacked[uo + 2] != 0
                        || unpacked[uo + 3] != 0
                        || unpacked[uo + 4] != 0
                        || unpacked[uo + 6] != 0
                    {
                        return false;
                    }
                    let length = unpacked[uo + 5] as usize;
                    uo += 7;
                    let interlace_mask = match registers[9] & 0x0c {
                        0x00 => {
                            if uo + (length << 8) + 1 != unpacked_length {
                                return false;
                            }
                            0
                        }
                        0x0c => {
                            let mid = uo + (length << 8);
                            if uo + (length << 9) + 7 + 1 != unpacked_length
                                || unpacked[mid] != 2
                                || unpacked[mid + 1] != 0
                                || unpacked[mid + 4] != 0
                                || unpacked[mid + 5] as usize != length
                                || unpacked[mid + 6] != 0
                            {
                                return false;
                            }
                            1
                        }
                        _ => return false,
                    };
                    let m = get_mig_mode(
                        registers[0] as i32,
                        registers[1] as i32,
                        registers[0x19] as i32,
                        length as i32,
                    );
                    let mode = if m == get_mig_mode(0x02, 0x00, 0x00, 0x38) {
                        if colors < 16 || interlace_mask != 0 {
                            return false;
                        }
                        self.decode_sc2sc4(&unpacked, uo, Resolution::Msx21x1);
                        return true;
                    } else if m == get_mig_mode(0x00, 0x08, 0x00, 0x06) {
                        if colors < 16 || interlace_mask != 0 {
                            return false;
                        }
                        self.decode_sc3_screen(&unpacked, uo, false);
                        return true;
                    } else if m == get_mig_mode(0x06, 0x00, 0x00, 0x6a) {
                        if colors < 16 {
                            return false;
                        }
                        5
                    } else if m == get_mig_mode(0x08, 0x00, 0x00, 0x6a) {
                        if colors < 4 {
                            return false;
                        }
                        6
                    } else if m == get_mig_mode(0x0a, 0x00, 0x00, 0xd4) {
                        if colors < 16 {
                            return false;
                        }
                        7
                    } else if m == get_mig_mode(0x0e, 0x00, 0x00, 0xd4) {
                        self.set_sc8_palette();
                        8
                    } else if m == get_mig_mode(0x0e, 0x00, 0x18, 0xd4) {
                        if colors < 16 {
                            return false;
                        }
                        10
                    } else if m == get_mig_mode(0x0e, 0x00, 0x08, 0xd4) {
                        12
                    } else {
                        return false;
                    };
                    self.decode_msx_screen(
                        &unpacked,
                        uo,
                        Some(&unpacked),
                        212,
                        mode,
                        interlace_mask,
                        true,
                    );
                    return true;
                }
                _ => return false,
            }
        }
        false
    }
}
