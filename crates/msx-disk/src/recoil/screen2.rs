//! MSX2 / MSX2+ / V9990 decoders, ported from RECOIL.
//!
//! Compressed variants that need a bitstream (RLE-packed `.SR*`/`.SCx` with a
//! `0xfd` header, packed `.G9B`) are not yet implemented and those files report
//! as unsupported; the common uncompressed `0xfe` images decode here.

use super::palette::{get_msx_header, MSX2_DEFAULT_PALETTE};
use super::{clamp_u5, get_nibble, Recoil, Resolution};

const G9B_YJK: i32 = 0;
const G9B_YUV: i32 = 1;

/// 128-byte-aligned image height from a SCREEN 5/6 BSAVE header.
fn get_msx128_height(content: &[u8], sr: bool) -> i32 {
    if content.len() < 135 || content[0] != 0xfe {
        return -1;
    }
    let header = get_msx_header(content);
    if header < 127 {
        return -1;
    }
    let height = (header + 1) >> 7;
    if content.len() < 7 + ((height as usize) << 7) {
        return -1;
    }
    if sr && height == 256 {
        return 256;
    }
    height.min(212)
}

fn is_msx_256_lines(content: &[u8]) -> bool {
    content.len() == 65543 && content[0] == 0xfe && get_msx_header(content) == 0xffff
}

/// Resolve the raw screen buffer for SR/SC8/SCC images. The RLE-packed `0xfd`
/// variant is not yet supported.
fn unpack_sr(content: &[u8]) -> Option<&[u8]> {
    if content.len() < 7 {
        return None;
    }
    match content[0] {
        0xfe if content.len() >= 54279 && get_msx_header(content) >= 0xd3ff => Some(content),
        _ => None,
    }
}

fn get_r8g8b8(content: &[u8], offset: usize) -> i32 {
    (content[offset] as i32) << 16 | (content[offset + 1] as i32) << 8 | content[offset + 2] as i32
}

impl Recoil<'_> {
    fn set_msx_companion_palette(&mut self, ext: &str) {
        match self.read_companion(ext) {
            Some(p) if p.len() >= 32 => self.set_msx_palette(&p, 0, 16),
            _ => self.set_msx_palette(&MSX2_DEFAULT_PALETTE, 0, 16),
        }
    }

    fn set_msx6_default_palette(&mut self) {
        self.content_palette[0] = 0x000000;
        self.content_palette[1] = 0x249224;
        self.content_palette[2] = 0x24db24;
        self.content_palette[3] = 0x6dff6d;
    }

    fn set_msx6_palette(&mut self) {
        match self.read_companion("pl6") {
            Some(p) if p.len() >= 8 => self.set_msx_palette(&p, 0, 4),
            _ => self.set_msx6_default_palette(),
        }
    }

    fn set_sc8_palette(&mut self) {
        const BLUES: [i32; 4] = [0, 2, 4, 7];
        for c in 0..256 {
            let ci = c as i32;
            let rgb = (ci & 0x1c) << 14 | (ci & 0xe0) << 3 | BLUES[c & 3];
            self.content_palette[c] = rgb << 5 | rgb << 2 | (rgb >> 1 & 0x030303);
        }
    }

    fn set_g9b_palette(&mut self, content: &[u8], colors: usize) -> bool {
        for c in 0..colors {
            let rgb = get_r8g8b8(content, 16 + c * 3);
            if (rgb & 0xe0e0e0) != 0 {
                return false;
            }
            self.content_palette[c] = rgb << 3 | (rgb >> 2 & 0x070707);
        }
        true
    }

    fn decode_msx6(&mut self, content: &[u8], offset: usize) {
        let height = self.get_original_height();
        for y in 0..height {
            for x in 0..self.width {
                let off = y * self.width + x;
                let b = content[offset + (off >> 2)] as i32;
                let idx = (b >> ((((!off) & 3) << 1) as u32)) & 3;
                let rgb = self.content_palette[idx as usize];
                self.set_scaled_pixel(x, y, rgb);
            }
        }
    }

    /// Decode one YJK (or palette) pixel; returns `0x00RRGGBB`.
    fn decode_msx_yjk(
        &self,
        content: &[u8],
        offset: usize,
        x: usize,
        width: usize,
        use_palette: bool,
    ) -> i32 {
        let y = (content[offset + x] >> 3) as i32;
        if use_palette && (y & 1) != 0 {
            return self.content_palette[(y >> 1) as usize];
        }
        let rgb = if (x | 3) >= width {
            y * 0x010101
        } else {
            let base = offset + (x & !3);
            let mut k = (content[base] & 7) as i32 | ((content[base + 1] & 7) as i32) << 3;
            let mut j = (content[base + 2] & 7) as i32 | ((content[base + 3] & 7) as i32) << 3;
            k -= (k & 0x20) << 1;
            j -= (j & 0x20) << 1;
            let r = clamp_u5(y + j);
            let g = clamp_u5(y + k);
            let b = clamp_u5((5 * y - 2 * j - k + 2) >> 2);
            r << 16 | g << 8 | b
        };
        rgb << 3 | (rgb >> 2 & 0x070707)
    }

    fn decode_msx_screen(
        &mut self,
        content: &[u8],
        offset: usize,
        interlace: Option<&[u8]>,
        height: usize,
        mode: i32,
        interlace_mask: usize,
    ) {
        if interlace_mask != 0 {
            let res = if mode >= 10 {
                Resolution::Msx2Plus2x1i
            } else if mode >> 1 == 3 {
                Resolution::Msx21x1i
            } else {
                Resolution::Msx22x1i
            };
            self.set_size(512, height << 1, res);
        } else if mode >> 1 == 3 {
            self.set_size(512, height << 1, Resolution::Msx21x2);
        } else {
            self.set_size(
                256,
                height,
                if mode >= 10 {
                    Resolution::Msx2Plus1x1
                } else {
                    Resolution::Msx21x1
                },
            );
        }

        for y in 0..self.height {
            let screen: &[u8] = if y & interlace_mask == 0 {
                content
            } else {
                interlace.unwrap_or(content)
            };
            for x in 0..self.width {
                let rgb = match mode {
                    5 => {
                        self.content_palette[get_nibble(
                            screen,
                            offset + ((y >> interlace_mask) << 7),
                            x >> interlace_mask,
                        ) as usize]
                    }
                    6 => {
                        let b = screen[offset + ((y >> 1) << 7) + (x >> 2)] as i32;
                        self.content_palette[(b >> ((((!x) & 3) << 1) as u32) & 3) as usize]
                    }
                    7 => {
                        self.content_palette
                            [get_nibble(screen, offset + ((y >> 1) << 8), x) as usize]
                    }
                    8 => {
                        self.content_palette[screen
                            [offset + ((y >> interlace_mask) << 8) + (x >> interlace_mask)]
                            as usize]
                    }
                    10 => self.decode_msx_yjk(
                        screen,
                        offset + ((y >> interlace_mask) << 8),
                        x >> interlace_mask,
                        256,
                        true,
                    ),
                    12 => self.decode_msx_yjk(
                        screen,
                        offset + ((y >> interlace_mask) << 8),
                        x >> interlace_mask,
                        256,
                        false,
                    ),
                    _ => 0,
                };
                self.pixels[y * self.width + x] = rgb;
            }
        }
    }

    fn decode_msx_sc(
        &mut self,
        content: &[u8],
        offset: usize,
        ext: &str,
        height: usize,
        mode: i32,
    ) -> bool {
        let interlace_length = 7 + (height << if mode <= 6 { 7 } else { 8 });
        if let Some(inter) = self.read_companion(ext) {
            if inter.len() >= interlace_length
                && inter[0] == 0xfe
                && get_msx_header(&inter) >= interlace_length as i32 - 8
            {
                self.decode_msx_screen(content, offset, Some(&inter), height, mode, 1);
                return true;
            }
        }
        self.decode_msx_screen(content, offset, None, height, mode, 0);
        false
    }

    pub(super) fn decode_sc5(&mut self, content: &[u8]) -> bool {
        let height = get_msx128_height(content, false);
        if height <= 0 {
            return false;
        }
        self.set_msx2_palette(content, 0x7687);
        if !self.decode_msx_sc(content, 7, "s15", height as usize, 5) && content.len() == 32775 {
            self.decode_msx_sprites(content, 5, 0x7607, 0x7807);
        }
        true
    }

    pub(super) fn decode_sc6(&mut self, content: &[u8]) -> bool {
        let height = get_msx128_height(content, false);
        if height <= 0 {
            return false;
        }
        if content.len() >= 30351 {
            self.set_msx_palette(content, 0x7687, 4);
        } else {
            self.set_msx6_default_palette();
        }
        if !self.decode_msx_sc(content, 7, "s16", height as usize, 6) && content.len() == 32775 {
            self.decode_msx_sprites(content, 6, 0x7607, 0x7807);
        }
        true
    }

    pub(super) fn decode_sc7(&mut self, content: &[u8]) -> bool {
        if content.len() < 54279 || content[0] != 0xfe || get_msx_header(content) < 0xd3ff {
            return false;
        }
        self.set_msx2_palette(content, 0xfa87);
        if !self.decode_msx_sc(content, 7, "s17", 212, 7) && content.len() == 64167 {
            self.decode_msx_sprites(content, 7, 0xfa07, 0xf007);
        }
        true
    }

    pub(super) fn decode_sc8(&mut self, content: &[u8]) -> bool {
        const SPRITE_PALETTE: [u8; 32] = [
            0x00, 0, 0x02, 0, 0x30, 0, 0x32, 0, 0x00, 3, 0x02, 3, 0x30, 3, 0x32, 3, 0x72, 4, 0x07,
            0, 0x70, 0, 0x77, 0, 0x00, 7, 0x07, 7, 0x70, 7, 0x77, 7,
        ];
        let (source, height): (&[u8], usize) = if is_msx_256_lines(content) {
            (content, 256)
        } else {
            match unpack_sr(content) {
                Some(c) => (c, 212),
                None => return false,
            }
        };
        self.set_sc8_palette();
        if !self.decode_msx_sc(source, 7, "s18", height, 8) && content.len() == 64167 {
            self.set_msx_palette(&SPRITE_PALETTE, 0, 16);
            self.decode_msx_sprites(content, 8, 0xfa07, 0xf007);
        }
        true
    }

    pub(super) fn decode_sr5(&mut self, content: &[u8]) -> bool {
        let height = get_msx128_height(content, true);
        if height <= 0 {
            return false;
        }
        self.set_msx_companion_palette("pl5");
        self.set_size(256, height as usize, Resolution::Msx21x1);
        self.decode_nibbles(content, 7, 128);
        true
    }

    pub(super) fn decode_sr6(&mut self, content: &[u8]) -> bool {
        let height = get_msx128_height(content, true);
        if height <= 0 {
            return false;
        }
        self.set_size(512, (height as usize) << 1, Resolution::Msx21x2);
        self.set_msx6_palette();
        self.decode_msx6(content, 7);
        true
    }

    pub(super) fn decode_sr7(&mut self, content: &[u8]) -> bool {
        let (source, height): (&[u8], usize) = if is_msx_256_lines(content) {
            (content, 256 * 2)
        } else {
            match unpack_sr(content) {
                Some(c) => (c, 212 * 2),
                None => return false,
            }
        };
        self.set_msx_companion_palette("pl7");
        self.set_size(512, height, Resolution::Msx21x2);
        self.decode_nibbles(source, 7, 256);
        true
    }

    pub(super) fn decode_sri(&mut self, content: &[u8]) -> bool {
        if content.len() != 108544 {
            return false;
        }
        self.set_msx_companion_palette("pl7");
        self.set_size(512, 424, Resolution::Msx21x1i);
        self.decode_nibbles(content, 0, 256);
        true
    }

    fn decode_gl16(&mut self, content: &[u8], resolution: Resolution, ext: &str) -> bool {
        if content.len() < 5 {
            return false;
        }
        let width = content[0] as usize | (content[1] as usize) << 8;
        let height = content[2] as usize | (content[3] as usize) << 8;
        if content.len() < 4 + ((width * height + 1) >> 1)
            || !self.set_scaled_size(width, height, resolution)
        {
            return false;
        }
        self.set_msx_companion_palette(ext);
        for y in 0..height {
            for x in 0..width {
                let rgb = self.content_palette[get_nibble(content, 4, y * width + x) as usize];
                self.set_scaled_pixel(x, y, rgb);
            }
        }
        true
    }

    pub(super) fn decode_gl5(&mut self, content: &[u8]) -> bool {
        self.decode_gl16(content, Resolution::Msx21x1, "pl5")
    }

    pub(super) fn decode_gl7(&mut self, content: &[u8]) -> bool {
        self.decode_gl16(content, Resolution::Msx21x2, "pl7")
    }

    pub(super) fn decode_gl6(&mut self, content: &[u8], is_gl6: bool) -> bool {
        if content.len() < 5 {
            return false;
        }
        let width = content[0] as usize | (content[1] as usize) << 8;
        let height = content[2] as usize | (content[3] as usize) << 8;
        if content.len() < 4 + ((width * height + 3) >> 2)
            || !self.set_size(width, height << 1, Resolution::Msx21x2)
        {
            return false;
        }
        if is_gl6 {
            self.set_msx6_palette();
        } else {
            // STP (Dynamic Publisher stamp): black on white.
            self.content_palette[0] = 0xffffff;
            self.content_palette[1] = 0;
            self.content_palette[2] = 0;
            self.content_palette[3] = 0;
        }
        self.decode_msx6(content, 4);
        true
    }

    pub(super) fn decode_gl8(&mut self, content: &[u8]) -> bool {
        if content.len() < 5 {
            return false;
        }
        let width = content[0] as usize | (content[1] as usize) << 8;
        let height = content[2] as usize | (content[3] as usize) << 8;
        if content.len() != 4 + width * height || !self.set_size(width, height, Resolution::Msx21x1)
        {
            return false;
        }
        self.set_sc8_palette();
        self.decode_bytes(content, 4);
        true
    }

    fn decode_msx_yjk_screen(&mut self, content: &[u8], offset: usize, use_palette: bool) {
        let width = self.get_original_width();
        for y in 0..self.height {
            for x in 0..width {
                let rgb = self.decode_msx_yjk(content, offset + y * width, x, width, use_palette);
                self.set_scaled_pixel(x, y, rgb);
            }
        }
    }

    pub(super) fn decode_glyjk(&mut self, content: &[u8], use_palette: bool) -> bool {
        if content.len() < 8 {
            return false;
        }
        let width = content[0] as usize | (content[1] as usize) << 8;
        let height = content[2] as usize | (content[3] as usize) << 8;
        if content.len() != 4 + width * height
            || !self.set_size(width, height, Resolution::Msx2Plus1x1)
        {
            return false;
        }
        if use_palette {
            self.set_msx_companion_palette("pla");
        }
        self.decode_msx_yjk_screen(content, 4, use_palette);
        true
    }

    fn decode_scc_sca(&mut self, content: &[u8], height: usize, use_palette: bool) {
        let (ext, mode) = if use_palette {
            ("s1a", 10)
        } else {
            ("s1c", 12)
        };
        if !self.decode_msx_sc(content, 7, ext, height, mode)
            && content.len() == 64167
            && content[0] == 0xfe
        {
            self.set_msx_palette(content, 0xfa87, 16);
            self.decode_msx_sprites(content, 12, 0xfa07, 0xf007);
        }
    }

    pub(super) fn decode_scc(&mut self, content: &[u8]) -> bool {
        let (source, height): (&[u8], usize) = if is_msx_256_lines(content) {
            (content, 256)
        } else if content.len() >= 49159 && content[0] == 0xfe && get_msx_header(content) == 0xbfff
        {
            (content, 192)
        } else {
            match unpack_sr(content) {
                Some(c) => (c, 212),
                None => return false,
            }
        };
        self.decode_scc_sca(source, height, false);
        true
    }

    pub(super) fn decode_sca(&mut self, content: &[u8]) -> bool {
        if content.len() < 64167 || content[0] != 0xfe || get_msx_header(content) < 0xd3ff {
            return false;
        }
        let height = if is_msx_256_lines(content) {
            self.set_msx_companion_palette("pla");
            256
        } else {
            self.set_msx_palette(content, 0xfa87, 16);
            212
        };
        self.decode_scc_sca(content, height, true);
        true
    }

    fn decode_g9b_unpacked(&mut self, content: &[u8], depth: i32, header_length: usize) {
        match depth {
            2 => self.decode_msx6(content, 16 + 4 * 3),
            4 => {
                let stride = (self.width + 1) >> 1;
                self.decode_nibbles(content, 16 + 16 * 3, stride);
            }
            8 => self.decode_bytes(content, header_length),
            G9B_YJK => self.decode_msx_yjk_screen(content, 16, false),
            G9B_YUV => {
                let pixels_length = self.width * self.height;
                for i in 0..pixels_length {
                    let y = (content[16 + i] >> 3) as i32;
                    let x = 16 + (i & !3);
                    let mut v = (content[x] & 7) as i32 | ((content[x + 1] & 7) as i32) << 3;
                    let mut u = (content[x + 2] & 7) as i32 | ((content[x + 3] & 7) as i32) << 3;
                    u -= (u & 0x20) << 1;
                    v -= (v & 0x20) << 1;
                    let r = clamp_u5(y + u);
                    let g = clamp_u5((((5 * y - v) >> 1) - u) >> 1);
                    let b = clamp_u5(y + v);
                    let rgb = r << 16 | g << 8 | b;
                    self.pixels[i] = rgb << 3 | (rgb >> 2 & 0x070707);
                }
            }
            16 => {
                let pixels_length = self.width * self.height;
                for i in 0..pixels_length {
                    let c = content[16 + (i << 1)] as i32 | (content[17 + (i << 1)] as i32) << 8;
                    let rgb = (c & 0x3e0) << 14 | (c & 0x7c00) << 1 | (c & 0x1f) << 3;
                    self.pixels[i] = rgb | (rgb >> 5 & 0x070707);
                }
            }
            _ => {}
        }
    }

    /// V9990 (GFX9000) `.G9B` image. Packed (`content[12] == 1`) images are not
    /// yet supported.
    pub(super) fn decode_g9b(&mut self, content: &[u8]) -> bool {
        if content.len() < 17
            || content[0] != b'G'
            || content[1] != b'9'
            || content[2] != b'B'
            || content[3] != 11
            || content[4] != 0
        {
            return false;
        }
        let mut depth = content[5] as i32;
        let header_length = 16 + content[7] as usize * 3;
        let width = content[8] as usize | (content[9] as usize) << 8;
        let height = content[10] as usize | (content[11] as usize) << 8;
        if content.len() <= header_length || !self.set_size(width, height, Resolution::MsxV99901x1)
        {
            return false;
        }
        let unpacked_length = header_length + ((width * depth as usize + 7) >> 3) * height;
        match depth {
            2 => {
                if content[7] != 4 || !self.set_g9b_palette(content, 4) {
                    return false;
                }
            }
            4 => {
                if content[7] != 16 || !self.set_g9b_palette(content, 16) {
                    return false;
                }
            }
            8 => match content[7] {
                0 => {
                    if width & 3 != 0 {
                        return false;
                    }
                    match content[6] {
                        0x40 => self.set_sc8_palette(),
                        0x80 => depth = G9B_YJK,
                        0xc0 => depth = G9B_YUV,
                        _ => return false,
                    }
                }
                64 => {
                    if !self.set_g9b_palette(content, 64) {
                        return false;
                    }
                    self.content_palette[64..256].fill(0);
                }
                _ => return false,
            },
            16 => {}
            _ => return false,
        }
        match content[12] {
            0 => {
                if content.len() != unpacked_length {
                    return false;
                }
                self.decode_g9b_unpacked(content, depth, header_length);
                true
            }
            _ => false, // packed (G9bStream) not yet supported
        }
    }
}
