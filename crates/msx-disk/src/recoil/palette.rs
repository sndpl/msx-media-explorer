//! MSX palette handling, ported from RECOIL.

use super::Recoil;

/// V9938 default 16-colour palette (`0xRB 0x0G`, 3 bits per component).
pub(super) const MSX2_DEFAULT_PALETTE: [u8; 32] = [
    0x00, 0, 0x00, 0, 0x11, 6, 0x33, 7, 0x17, 1, 0x27, 3, 0x51, 1, 0x27, 6, 0x71, 1, 0x73, 3, 0x61,
    6, 0x64, 6, 0x11, 4, 0x65, 2, 0x55, 5, 0x77, 7,
];

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
pub(super) fn is_msx_palette(content: &[u8], offset: usize) -> bool {
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
        const PAL: [i32; 16] = [
            0x000800, 0x000400, 0x3abb43, 0x70d377, 0x5459d7, 0x7b7be8, 0xb3634b, 0x61dfe7,
            0xd46a53, 0xf88e77, 0xc7c759, 0xd9d481, 0x36a53b, 0xb06bae, 0xc7d0c5, 0xfafff8,
        ];
        self.content_palette[..16].copy_from_slice(&PAL);
    }

    /// Decode `colors` palette entries (`0xRB 0x0G`, 3 bits/component) into the
    /// active palette.
    pub(super) fn set_msx_palette(&mut self, content: &[u8], offset: usize, colors: usize) {
        for i in 0..colors {
            let rb = content[offset + (i << 1)] as i32;
            let g = content[offset + (i << 1) + 1] as i32;
            let rgb = (rb & 0x70) << 12 | (rb & 7) | (g & 7) << 8;
            self.content_palette[i] = rgb << 5 | rgb << 2 | (rgb >> 1 & 0x030303);
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
