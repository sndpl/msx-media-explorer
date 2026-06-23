//! MSX SCREEN-mode decoders, ported from RECOIL.

use super::palette::{get_msx_header, is_msx_palette, MSX2_DEFAULT_PALETTE};
use super::{Recoil, Resolution};

impl Recoil<'_> {
    /// SCREEN 2 / SCREEN 4 bitmap (pattern + colour tables).
    pub(super) fn decode_sc2sc4(&mut self, content: &[u8], offset: usize, resolution: Resolution) {
        self.set_size(256, 192, resolution);
        for y in 0..192usize {
            let font_offset = offset + ((y & 0xc0) << 5) + (y & 7);
            for x in 0..256usize {
                let name = content[offset + 0x1800 + ((y & !7) << 2) + (x >> 3)] as usize;
                let b = font_offset + (name << 3);
                let c = content[0x2000 + b] as i32;
                let bit = (content[b] as i32 >> ((!x & 7) as u32)) & 1;
                let idx = if bit == 0 { c & 0xf } else { c >> 4 };
                self.pixels[(y << 8) + x] = self.content_palette[idx as usize];
            }
        }
    }

    /// Overlay MSX hardware sprites (modes 1 and 2) on the decoded screen.
    pub(super) fn decode_msx_sprites(
        &mut self,
        content: &[u8],
        mode: i32,
        attributes_offset: usize,
        patterns_offset: usize,
    ) {
        let rd = |i: usize| -> i32 { content.get(i).copied().unwrap_or(0) as i32 };
        let height = if mode <= 4 { 192 } else { 212 };
        for y in 0..height {
            for x in 0..256i32 {
                let mut color = 0i32;
                let mut enable_or = false;
                let mut sprites_per_line: i32 = if mode >= 4 { 8 } else { 4 };
                for sprite in 0..32i32 {
                    let attribute_offset = attributes_offset + ((sprite as usize) << 2);
                    let sprite_y = rd(attribute_offset);
                    if sprite_y == if mode >= 4 { 216 } else { 208 } {
                        break;
                    }
                    let row = (y - sprite_y - 1) & 0xff;
                    if row >= 16 {
                        continue;
                    }
                    sprites_per_line -= 1;
                    if sprites_per_line < 0 {
                        break;
                    }
                    let c = if mode >= 4 {
                        rd((attributes_offset as i32 - 0x200 + (sprite << 4) + row) as usize)
                    } else {
                        rd(attribute_offset + 3)
                    };
                    if mode < 4 || (c & 0x40) == 0 {
                        if color != 0 {
                            break; // already have a sprite pixel here; paint it
                        }
                        enable_or = true;
                    } else if !enable_or {
                        continue;
                    }
                    let mut column = x - rd(attribute_offset + 1);
                    if c >= 0x80 {
                        column += 32;
                    }
                    if !(0..16).contains(&column) {
                        continue;
                    }
                    let pattern =
                        patterns_offset as i32 + ((rd(attribute_offset + 2) & 0xfc) << 3) + row;
                    if (rd((pattern + ((column & 8) << 1)) as usize) >> ((!column & 7) as u32)) & 1
                        == 0
                    {
                        continue;
                    }
                    color |= c;
                    if mode >= 4 {
                        color |= 0x10; // make c==0 non-transparent
                    }
                }
                if color == 0 {
                    continue;
                }
                let x = x as usize;
                let y = y as usize;
                match mode {
                    6 => {
                        let offset = (y << 10) + (x << 1);
                        let hi = self.content_palette[(color >> 2 & 3) as usize];
                        let lo = self.content_palette[(color & 3) as usize];
                        self.pixels[offset] = hi;
                        self.pixels[offset + 512] = hi;
                        self.pixels[offset + 1] = lo;
                        self.pixels[offset + 513] = lo;
                    }
                    7 => {
                        let offset = (y << 10) + (x << 1);
                        let rgb = self.content_palette[(color & 0xf) as usize];
                        self.pixels[offset] = rgb;
                        self.pixels[offset + 1] = rgb;
                        self.pixels[offset + 512] = rgb;
                        self.pixels[offset + 513] = rgb;
                    }
                    _ => {
                        self.pixels[(y << 8) + x] = self.content_palette[(color & 0xf) as usize];
                    }
                }
            }
        }
    }

    pub(super) fn decode_sc2(&mut self, content: &[u8]) -> bool {
        if content.len() < 14343 || content[0] != 0xfe || get_msx_header(content) < 0x37ff {
            return false;
        }
        if is_msx_palette(content, 0x1b87) {
            self.set_msx_palette(content, 0x1b87, 16);
            self.decode_sc2sc4(content, 7, Resolution::Msx21x1);
        } else {
            self.set_msx1_palette();
            self.decode_sc2sc4(content, 7, Resolution::Msx11x1);
        }
        if content.len() == 16391 {
            self.decode_msx_sprites(content, 2, 0x1b07, 0x3807);
        }
        true
    }

    fn decode_sc3_screen(&mut self, content: &[u8], offset: usize, is_long: bool) {
        self.set_size(256, 192, Resolution::Msx14x4);
        for y in 0..192usize {
            for x in 0..256usize {
                let mut c = if is_long {
                    content[0x807 + ((y & !7) << 2) + (x >> 3)] as usize
                } else {
                    (y & 0xe0) + (x >> 3)
                };
                c = (content[offset + (c << 3) + (y >> 2 & 7)] as usize >> (!x & 4)) & 0xf;
                self.pixels[(y << 8) + x] = self.content_palette[c];
            }
        }
    }

    pub(super) fn decode_sc3(&mut self, content: &[u8]) -> bool {
        if content.len() < 1543 || content[0] != 0xfe || get_msx_header(content) < 0x5ff {
            return false;
        }
        if content.len() >= 0x2047 && is_msx_palette(content, 0x2027) {
            self.set_msx_palette(content, 0x2027, 16);
        } else {
            self.set_msx1_palette();
        }
        self.decode_sc3_screen(content, 7, content.len() >= 0xb07);
        if content.len() == 16391 {
            self.decode_msx_sprites(content, 3, 0x1b07, 0x3807);
        }
        true
    }

    pub(super) fn decode_sc4(&mut self, content: &[u8]) -> bool {
        if content.len() < 14343 || content[0] != 0xfe || get_msx_header(content) < 0x37ff {
            return false;
        }
        if is_msx_palette(content, 0x1b87) {
            self.set_msx_palette(content, 0x1b87, 16);
        } else {
            self.set_msx_palette(&MSX2_DEFAULT_PALETTE, 0, 16);
        }
        self.decode_sc2sc4(content, 7, Resolution::Msx21x1);
        if content.len() >= 16391 {
            self.decode_msx_sprites(content, 4, 0x1e07, 0x3807);
        }
        true
    }
}
