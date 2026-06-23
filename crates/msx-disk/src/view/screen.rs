//! MSX screen-image rendering.
//!
//! Decodes BSAVEd VRAM dumps of SCREEN 2/5/7/8/12 into RGBA bitmaps. Palette
//! values and the YJK conversion are taken from openMSX / msxconverter. As the
//! original DskExplorer notes, palettes are not always 100% faithful.

use crate::view::bsave;

/// 3-bit colour component to 8-bit (round(n * 255 / 7)).
const TBL3: [u8; 8] = [0x00, 0x24, 0x49, 0x6D, 0x92, 0xB6, 0xDB, 0xFF];
/// 2-bit colour component to 8-bit.
const TBL2: [u8; 4] = [0x00, 0x55, 0xAA, 0xFF];

/// TMS9918 fixed 16-colour palette (RGB888), used by SCREEN 2. Index 0 is
/// transparent; rendered as black.
const TMS9918: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0x00, 0x00, 0x00),
    (0x0A, 0xAD, 0x1E),
    (0x34, 0xC8, 0x4C),
    (0x2B, 0x2D, 0xE3),
    (0x51, 0x4B, 0xFB),
    (0xBD, 0x29, 0x25),
    (0x1E, 0xE2, 0xEF),
    (0xFB, 0x2C, 0x2B),
    (0xFF, 0x5F, 0x4C),
    (0xBD, 0xA2, 0x2B),
    (0xD7, 0xB4, 0x54),
    (0x0A, 0x8C, 0x18),
    (0xAF, 0x32, 0x9A),
    (0xB2, 0xB2, 0xB2),
    (0xFF, 0xFF, 0xFF),
];

/// V9938 default 16-colour palette as (green, red, blue) 3-bit digits.
const V9938_GRB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (0, 0, 0),
    (6, 1, 1),
    (7, 3, 3),
    (1, 1, 7),
    (3, 2, 7),
    (1, 5, 1),
    (6, 2, 7),
    (1, 7, 1),
    (3, 7, 3),
    (6, 6, 1),
    (6, 6, 4),
    (4, 1, 1),
    (2, 6, 5),
    (5, 5, 5),
    (7, 7, 7),
];

/// A decoded RGBA8 image (top-left origin, 4 bytes per pixel).
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// The MSX screen modes this renderer supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Sc2,
    Sc5,
    Sc7,
    Sc8,
    Sc12,
}

impl ScreenMode {
    /// Map a file extension (no dot, any case) to a screen mode.
    pub fn from_extension(ext: &str) -> Option<ScreenMode> {
        match ext.to_ascii_lowercase().as_str() {
            "sc2" => Some(ScreenMode::Sc2),
            "sc5" => Some(ScreenMode::Sc5),
            "sc7" => Some(ScreenMode::Sc7),
            "sc8" => Some(ScreenMode::Sc8),
            "scc" | "sc12" => Some(ScreenMode::Sc12),
            _ => None,
        }
    }

    /// Detect a screen mode from a file name's extension.
    pub fn from_filename(name: &str) -> Option<ScreenMode> {
        name.rsplit('.').next().and_then(ScreenMode::from_extension)
    }

    /// Pixel dimensions of the mode.
    pub fn dimensions(self) -> (usize, usize) {
        match self {
            ScreenMode::Sc2 => (256, 192),
            ScreenMode::Sc5 | ScreenMode::Sc8 | ScreenMode::Sc12 => (256, 212),
            ScreenMode::Sc7 => (512, 212),
        }
    }
}

/// Render a screen file (with or without a BSAVE header) into an RGBA bitmap.
pub fn render(mode: ScreenMode, file_bytes: &[u8]) -> Bitmap {
    let vram = bsave::payload(file_bytes);
    match mode {
        ScreenMode::Sc2 => render_sc2(vram),
        ScreenMode::Sc5 => render_4bpp(vram, 256, 212, 128, palette_sc5(vram)),
        ScreenMode::Sc7 => render_4bpp(vram, 512, 212, 256, v9938_default()),
        ScreenMode::Sc8 => render_sc8(vram),
        ScreenMode::Sc12 => render_sc12(vram),
    }
}

fn v9938_default() -> [(u8, u8, u8); 16] {
    let mut pal = [(0u8, 0u8, 0u8); 16];
    for (i, &(g, r, b)) in V9938_GRB.iter().enumerate() {
        pal[i] = (TBL3[r as usize], TBL3[g as usize], TBL3[b as usize]);
    }
    pal
}

/// SCREEN 5 palette: an embedded 32-byte VDP palette block (at VRAM 0x7680) if
/// the dump is long enough to contain one, otherwise the V9938 default.
fn palette_sc5(vram: &[u8]) -> [(u8, u8, u8); 16] {
    const OFF: usize = 0x7680;
    if vram.len() < OFF + 32 {
        return v9938_default();
    }
    let mut pal = [(0u8, 0u8, 0u8); 16];
    for (i, slot) in pal.iter_mut().enumerate() {
        let b0 = vram[OFF + i * 2]; // 0RRR0BBB
        let b1 = vram[OFF + i * 2 + 1]; // 00000GGG
        let r = (b0 >> 4) & 7;
        let b = b0 & 7;
        let g = b1 & 7;
        *slot = (TBL3[r as usize], TBL3[g as usize], TBL3[b as usize]);
    }
    pal
}

fn px(vram: &[u8], i: usize) -> u8 {
    vram.get(i).copied().unwrap_or(0)
}

fn put(rgba: &mut [u8], width: usize, x: usize, y: usize, (r, g, b): (u8, u8, u8)) {
    let i = (y * width + x) * 4;
    rgba[i] = r;
    rgba[i + 1] = g;
    rgba[i + 2] = b;
    rgba[i + 3] = 0xFF;
}

fn render_sc2(vram: &[u8]) -> Bitmap {
    let (w, h) = (256usize, 192usize);
    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        let cell_row = row / 8;
        let line = row % 8;
        let bank = cell_row / 8; // 0,1,2 for the three screen thirds
        for col in 0..32 {
            let cell = cell_row * 32 + col;
            let name = px(vram, 0x1800 + cell) as usize;
            let pattern = px(vram, bank * 0x800 + name * 8 + line);
            let color = px(vram, 0x2000 + bank * 0x800 + name * 8 + line);
            let fg = (color >> 4) & 0x0F;
            let bg = color & 0x0F;
            for bit in 0..8 {
                let on = (pattern >> (7 - bit)) & 1 == 1;
                let idx = if on { fg } else { bg } as usize;
                put(&mut rgba, w, col * 8 + bit, row, TMS9918[idx]);
            }
        }
    }
    Bitmap {
        width: w,
        height: h,
        rgba,
    }
}

fn render_4bpp(
    vram: &[u8],
    w: usize,
    h: usize,
    stride: usize,
    palette: [(u8, u8, u8); 16],
) -> Bitmap {
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let byte = px(vram, y * stride + x / 2);
            let nibble = if x % 2 == 0 { byte >> 4 } else { byte & 0x0F };
            put(&mut rgba, w, x, y, palette[nibble as usize]);
        }
    }
    Bitmap {
        width: w,
        height: h,
        rgba,
    }
}

fn render_sc8(vram: &[u8]) -> Bitmap {
    let (w, h) = (256usize, 212usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let byte = px(vram, y * w + x); // GGGRRRBB
            let g = (byte >> 5) & 7;
            let r = (byte >> 2) & 7;
            let b = byte & 3;
            put(
                &mut rgba,
                w,
                x,
                y,
                (TBL3[r as usize], TBL3[g as usize], TBL2[b as usize]),
            );
        }
    }
    Bitmap {
        width: w,
        height: h,
        rgba,
    }
}

fn render_sc12(vram: &[u8]) -> Bitmap {
    let (w, h) = (256usize, 212usize);
    let mut rgba = vec![0u8; w * h * 4];
    let expand5 = |v: i32| -> u8 { ((v << 3) | (v >> 2)) as u8 };
    for y in 0..h {
        let mut x = 0;
        while x < w {
            let base = y * w + x;
            let p = [
                px(vram, base),
                px(vram, base + 1),
                px(vram, base + 2),
                px(vram, base + 3),
            ];
            let mut k = ((p[0] & 7) | ((p[1] & 7) << 3)) as i32;
            if k >= 32 {
                k -= 64;
            }
            let mut j = ((p[2] & 7) | ((p[3] & 7) << 3)) as i32;
            if j >= 32 {
                j -= 64;
            }
            for (n, &byte) in p.iter().enumerate() {
                let yv = (byte >> 3) as i32;
                let r = (yv + j).clamp(0, 31);
                let g = (yv + k).clamp(0, 31);
                let b = ((5 * yv - 2 * j - k + 2) / 4).clamp(0, 31);
                put(&mut rgba, w, x + n, y, (expand5(r), expand5(g), expand5(b)));
            }
            x += 4;
        }
    }
    Bitmap {
        width: w,
        height: h,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(bmp: &Bitmap, x: usize, y: usize) -> (u8, u8, u8) {
        let i = (y * bmp.width + x) * 4;
        (bmp.rgba[i], bmp.rgba[i + 1], bmp.rgba[i + 2])
    }

    #[test]
    fn extension_detection() {
        assert_eq!(ScreenMode::from_filename("PIC.SC5"), Some(ScreenMode::Sc5));
        assert_eq!(ScreenMode::from_filename("a.scc"), Some(ScreenMode::Sc12));
        assert_eq!(ScreenMode::from_filename("readme.txt"), None);
    }

    #[test]
    fn sc2_pattern_and_color() {
        let mut vram = vec![0u8; 0x3800];
        vram[0] = 0x80; // pattern for char 0, line 0: leftmost pixel set
        vram[0x1800] = 0; // name table cell 0 -> pattern 0
        vram[0x2000] = 0xF1; // colour: fg=15 (white), bg=1 (black)
        let bmp = render(ScreenMode::Sc2, &vram);
        assert_eq!(pixel(&bmp, 0, 0), (0xFF, 0xFF, 0xFF)); // fg
        assert_eq!(pixel(&bmp, 1, 0), (0x00, 0x00, 0x00)); // bg
    }

    #[test]
    fn sc5_nibble_order_and_default_palette() {
        let mut vram = vec![0u8; 27136];
        vram[0] = 0x2F; // x=0 -> nibble 2, x=1 -> nibble 15
        let bmp = render(ScreenMode::Sc5, &vram);
        // palette[2] = GRB (6,1,1) -> (R=tbl3[1], G=tbl3[6], B=tbl3[1])
        assert_eq!(pixel(&bmp, 0, 0), (TBL3[1], TBL3[6], TBL3[1]));
        // palette[15] = (7,7,7) -> white
        assert_eq!(pixel(&bmp, 1, 0), (0xFF, 0xFF, 0xFF));
    }

    #[test]
    fn sc8_grb332() {
        let mut vram = vec![0u8; 256 * 212];
        vram[0] = 0b1110_0000; // GGGRRRBB = g7 r0 b0 -> green
        let bmp = render(ScreenMode::Sc8, &vram);
        assert_eq!(pixel(&bmp, 0, 0), (0x00, 0xFF, 0x00));
    }

    #[test]
    fn sc12_yjk_grayscale() {
        // All four bytes = 0xF8: Y=31, J=K=0 -> white for all four pixels.
        let mut vram = vec![0u8; 256 * 212];
        vram[..4].copy_from_slice(&[0xF8, 0xF8, 0xF8, 0xF8]);
        let bmp = render(ScreenMode::Sc12, &vram);
        for x in 0..4 {
            assert_eq!(pixel(&bmp, x, 0), (0xFF, 0xFF, 0xFF));
        }
    }
}
