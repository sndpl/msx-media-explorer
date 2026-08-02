//! MSX palette handling, ported from RECOIL.
//!
//! The colour tables live here as plain data (`MSX1_RGB`, [`msx2_default_rgb`],
//! [`sc8_rgb`], [`msx_palette_rgb`]) and the [`Recoil`] setters are thin wrappers
//! over them, so the raw-bitmap viewer in [`crate::view::bitmap`] can reuse the
//! exact same colours the file decoders use.

use super::{clamp_u5, Recoil};

/// V9938 default 16-colour palette (`0xRB 0x0G`, 3 bits per component).
pub(super) const MSX2_DEFAULT_PALETTE: [u8; 32] = [
    0x00, 0, 0x00, 0, 0x11, 6, 0x33, 7, 0x17, 1, 0x27, 3, 0x51, 1, 0x27, 6, 0x71, 1, 0x73, 3, 0x61,
    6, 0x64, 6, 0x11, 4, 0x65, 2, 0x55, 5, 0x77, 7,
];

/// TMS9928/9929 fixed palette (50% saturation variant, by Fabio Schmidlin), as
/// `0x00RRGGBB`. Used by the MSX1 screen modes.
pub const MSX1_RGB: [u32; 16] = [
    0x000800, 0x000400, 0x3abb43, 0x70d377, 0x5459d7, 0x7b7be8, 0xb3634b, 0x61dfe7, 0xd46a53,
    0xf88e77, 0xc7c759, 0xd9d481, 0x36a53b, 0xb06bae, 0xc7d0c5, 0xfafff8,
];

/// V9938 default SCREEN 6 4-colour palette, as `0x00RRGGBB`.
pub const MSX6_DEFAULT_RGB: [u32; 4] = [0x000000, 0x249224, 0x24db24, 0x6dff6d];

/// Expand a value whose R, G and B components are 3 bits wide into full 8-bit
/// components (the V9938 palette expansion).
pub fn expand_rgb333(rgb: i32) -> u32 {
    (rgb << 5 | rgb << 2 | (rgb >> 1 & 0x03_0303)) as u32
}

/// Expand a value whose R, G and B components are 5 bits wide into full 8-bit
/// components (the YJK / G9B expansion).
pub fn expand_rgb555(rgb: i32) -> u32 {
    (rgb << 3 | (rgb >> 2 & 0x07_0707)) as u32
}

/// Sign-extend a raw 6-bit YJK J or K component.
pub fn yjk_signed(v: i32) -> i32 {
    v - ((v & 0x20) << 1)
}

/// Convert one YJK triple (5-bit luminance, sign-extended J and K) to
/// `0x00RRGGBB`.
pub fn yjk_to_rgb(y: i32, j: i32, k: i32) -> u32 {
    let r = clamp_u5(y + j);
    let g = clamp_u5(y + k);
    let b = clamp_u5((5 * y - 2 * j - k + 2) >> 2);
    expand_rgb555(r << 16 | g << 8 | b)
}

/// The V9938 default 16-colour palette as `0x00RRGGBB` entries.
pub fn msx2_default_rgb() -> [u32; 16] {
    let mut out = [0u32; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = msx_palette_entry(&MSX2_DEFAULT_PALETTE, 0, i);
    }
    out
}

/// The fixed SCREEN 8 GRB332 palette as `0x00RRGGBB` entries.
pub fn sc8_rgb() -> [u32; 256] {
    const BLUES: [i32; 4] = [0, 2, 4, 7];
    let mut out = [0u32; 256];
    for (c, slot) in out.iter_mut().enumerate() {
        let ci = c as i32;
        *slot = expand_rgb333((ci & 0x1c) << 14 | (ci & 0xe0) << 3 | BLUES[c & 3]);
    }
    out
}

/// Decode entry `index` of an MSX palette stored as `0xRB 0x0G` pairs at
/// `offset`, as `0x00RRGGBB`.
fn msx_palette_entry(content: &[u8], offset: usize, index: usize) -> u32 {
    let rb = content[offset + (index << 1)] as i32;
    let g = content[offset + (index << 1) + 1] as i32;
    expand_rgb333((rb & 0x70) << 12 | (rb & 7) | (g & 7) << 8)
}

/// Decode `colors` MSX palette entries (`0xRB 0x0G`, 3 bits per component) at
/// `offset` into `0x00RRGGBB` values. Entries that would read past the end of
/// `content` are returned as black.
pub fn msx_palette_rgb(content: &[u8], offset: usize, colors: usize) -> Vec<u32> {
    (0..colors)
        .map(|i| {
            if offset + (i << 1) + 1 < content.len() {
                msx_palette_entry(content, offset, i)
            } else {
                0
            }
        })
        .collect()
}

/// Read the 16-bit value at offsets 3..4 of a BSAVE header (end address),
/// requiring the other header words to be zero. Returns -1 if not a header.
pub(super) fn get_msx_header(content: &[u8]) -> i32 {
    if content.len() < 7 {
        return -1;
    }
    if content[1] != 0 || content[2] != 0 || content[5] != 0 || content[6] != 0 {
        return -1;
    }
    content[3] as i32 | (content[4] as i32) << 8
}

/// Whether the 32 bytes at `offset` look like a valid embedded MSX palette.
pub fn is_msx_palette(content: &[u8], offset: usize) -> bool {
    if offset + 32 > content.len() {
        return false;
    }
    let mut ored = 0;
    for i in 0..16 {
        let rb = content[offset + (i << 1)];
        let g = content[offset + (i << 1) + 1];
        if (rb & 0x88) != 0 || (g & 0xf8) != 0 {
            return false;
        }
        ored |= rb | g;
    }
    ored != 0
}

impl Recoil<'_> {
    /// TMS9928/9929 fixed palette (50% saturation variant, by Fabio Schmidlin).
    pub(super) fn set_msx1_palette(&mut self) {
        for (slot, &rgb) in self.content_palette.iter_mut().zip(MSX1_RGB.iter()) {
            *slot = rgb as i32;
        }
    }

    /// Decode `colors` palette entries (`0xRB 0x0G`, 3 bits/component) into the
    /// active palette.
    pub(super) fn set_msx_palette(&mut self, content: &[u8], offset: usize, colors: usize) {
        for i in 0..colors {
            self.content_palette[i] = msx_palette_entry(content, offset, i) as i32;
        }
    }

    /// Use an embedded 16-colour palette if present, else the V9938 default.
    pub(super) fn set_msx2_palette(&mut self, content: &[u8], offset: usize) {
        if content.len() >= offset + 32 {
            self.set_msx_palette(content, offset, 16);
        } else {
            self.set_msx_palette(&MSX2_DEFAULT_PALETTE, 0, 16);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The extracted tables must reproduce exactly what the decoder setters
    /// install, so the raw-bitmap viewer and the file decoders agree on colour.
    #[test]
    fn msx2_default_rgb_matches_the_encoded_table() {
        let rgb = msx2_default_rgb();
        assert_eq!(rgb.len(), 16);
        // Entry 0 is transparent/black; entry 15 is white (0x77, 7 -> full).
        assert_eq!(rgb[0], 0x000000);
        assert_eq!(rgb[15], 0xffffff);
    }

    #[test]
    fn sc8_rgb_is_grb332() {
        let pal = sc8_rgb();
        assert_eq!(pal.len(), 256);
        assert_eq!(pal[0], 0x000000);
        assert_eq!(pal[255], 0xffffff);
    }

    #[test]
    fn msx_palette_rgb_pads_short_input_with_black() {
        // Two valid entries followed by nothing: the rest must be black, not a panic.
        let bytes = [0x77u8, 7, 0x00, 0];
        let pal = msx_palette_rgb(&bytes, 0, 16);
        assert_eq!(pal[0], 0xffffff);
        assert_eq!(pal[1], 0x000000);
        assert!(pal[2..].iter().all(|&c| c == 0));
    }

    #[test]
    fn yjk_signed_sign_extends_six_bits() {
        assert_eq!(yjk_signed(0), 0);
        assert_eq!(yjk_signed(31), 31);
        assert_eq!(yjk_signed(32), -32);
        assert_eq!(yjk_signed(63), -1);
    }

    #[test]
    fn yjk_grey_when_chroma_is_zero() {
        // J = K = 0 gives R = G = Y and B = (5Y + 2) >> 2, i.e. near-grey.
        let rgb = yjk_to_rgb(31, 0, 0);
        assert_eq!(rgb, 0xffffff);
        assert_eq!(yjk_to_rgb(0, 0, 0), 0x000000);
    }
}
