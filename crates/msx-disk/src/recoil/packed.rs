//! MSX paint-tool formats, ported from RECOIL: Dynamic Publisher (`.PCT`/`.FNT`),
//! DD-Graph (`.CMP`), and MSX Interchange (`.MIF`/`.MIG`).

use super::bitstream::{unpack_mif, CciStream, MigStream, ZimStream};
use super::{get32_le, is_string_at, Recoil, Resolution};

fn get_mig_mode(reg0: i32, reg1: i32, reg19: i32, length: i32) -> i32 {
    (reg0 & 0x0e) | (reg1 & 0x18) << 1 | (reg19 & 0x18) << 3 | length << 8
}

impl Recoil<'_> {
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
