//! MSX / MSX2 / MSX2+ / V9990 graphics-format decoders.
//!
//! This is a Rust port of the MSX-family decoders from RECOIL (Retro Computer
//! Image Library) by Piotr Fusik, <https://recoil.sourceforge.net/>, which is
//! licensed under the GNU GPL v2 or later. The project adopts the same license
//! because of this port.
//!
//! The public entry point is [`decode`]. Formats that reference companion files
//! (a `.PLx` palette or a `.Sxx` interlace half) read them through a
//! [`CompanionFiles`] provider so the sibling files can come from the same disk.

mod bitstream;
mod packed;
mod palette;
mod screen;
mod screen2;

/// Maximum number of pixels we will allocate for a decoded image (guards against
/// hostile headers). 512x424 with 2 frames is the largest real MSX image.
const MAX_PIXELS: usize = 2_000_000;

/// A decoded image: row-major `0x00RRGGBB` pixels.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Image {
    /// Convert to tightly-packed RGBA8 (alpha = 0xFF).
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for &p in &self.pixels {
            rgba.push((p >> 16) as u8);
            rgba.push((p >> 8) as u8);
            rgba.push(p as u8);
            rgba.push(0xFF);
        }
        rgba
    }
}

/// Supplies companion files (palette / interlace halves) for the file being
/// decoded, looked up by extension within the same container.
pub trait CompanionFiles {
    /// Return the sibling file whose extension is `ext` (case-insensitive,
    /// without a dot), if present.
    fn read(&self, ext: &str) -> Option<Vec<u8>>;
}

/// A provider that has no companion files.
pub struct NoCompanions;

impl CompanionFiles for NoCompanions {
    fn read(&self, _ext: &str) -> Option<Vec<u8>> {
        None
    }
}

/// A [`CompanionFiles`] backed by a closure, so callers (GUI, CLI) can resolve
/// siblings however they like (e.g. from a mounted disk).
pub struct FnCompanions<F: Fn(&str) -> Option<Vec<u8>>>(pub F);

impl<F: Fn(&str) -> Option<Vec<u8>>> CompanionFiles for FnCompanions<F> {
    fn read(&self, ext: &str) -> Option<Vec<u8>> {
        (self.0)(ext)
    }
}

/// Logical resolution, mirroring the subset of RECOIL resolutions used by MSX.
/// It selects how [`Recoil::set_scaled_pixel`] expands a logical pixel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolution {
    Msx11x1,
    Msx14x4,
    Msx21x1,
    Msx21x2,
    Msx21x1i,
    Msx22x1i,
    Msx2Plus1x1,
    Msx2Plus2x1i,
    MsxV99901x1,
}

/// Decoder state: the framebuffer, the active palette, and companion files.
pub(crate) struct Recoil<'a> {
    width: usize,
    height: usize,
    resolution: Resolution,
    pixels: Vec<i32>,
    content_palette: [i32; 256],
    companions: &'a dyn CompanionFiles,
}

impl<'a> Recoil<'a> {
    fn new(companions: &'a dyn CompanionFiles) -> Recoil<'a> {
        Recoil {
            width: 0,
            height: 0,
            resolution: Resolution::Msx11x1,
            pixels: Vec::new(),
            content_palette: [0; 256],
            companions,
        }
    }

    /// Read a companion file by extension (case-insensitive, no dot).
    pub(crate) fn read_companion(&self, ext: &str) -> Option<Vec<u8>> {
        self.companions.read(ext)
    }
}

impl Recoil<'_> {
    /// Initialize the decoded image size and resolution. Returns false if the
    /// dimensions are implausible.
    pub(crate) fn set_size(&mut self, width: usize, height: usize, resolution: Resolution) -> bool {
        if width == 0 || height == 0 || width * height > MAX_PIXELS {
            return false;
        }
        self.width = width;
        self.height = height;
        self.resolution = resolution;
        self.pixels = vec![0; width * height];
        true
    }

    /// Like [`set_size`](Self::set_size) but doubles width/height first per the
    /// resolution (mirrors RECOIL's `SetScaledSize`).
    pub(crate) fn set_scaled_size(
        &mut self,
        mut width: usize,
        mut height: usize,
        resolution: Resolution,
    ) -> bool {
        match resolution {
            Resolution::Msx22x1i | Resolution::Msx2Plus2x1i => width <<= 1,
            Resolution::Msx21x2 => height <<= 1,
            _ => {}
        }
        self.set_size(width, height, resolution)
    }

    /// Write a logical pixel, expanding it per the resolution (mirrors RECOIL's
    /// `SetScaledPixel`).
    pub(crate) fn set_scaled_pixel(&mut self, x: usize, y: usize, rgb: i32) {
        match self.resolution {
            Resolution::Msx22x1i | Resolution::Msx2Plus2x1i => {
                let offset = y * self.width + (x << 1);
                self.pixels[offset] = rgb;
                self.pixels[offset + 1] = rgb;
            }
            Resolution::Msx21x2 => {
                let offset = ((y * self.width) << 1) + x;
                self.pixels[offset] = rgb;
                self.pixels[offset + self.width] = rgb;
            }
            _ => {
                self.pixels[y * self.width + x] = rgb;
            }
        }
    }

    /// Logical (pre-scaling) width — used by byte/nibble fillers.
    pub(crate) fn get_original_width(&self) -> usize {
        match self.resolution {
            Resolution::Msx22x1i | Resolution::Msx2Plus2x1i => self.width >> 1,
            Resolution::Msx14x4 => self.width >> 2,
            _ => self.width,
        }
    }

    /// Logical (pre-scaling) height.
    pub(crate) fn get_original_height(&self) -> usize {
        match self.resolution {
            Resolution::Msx21x2 => self.height >> 1,
            Resolution::Msx14x4 => self.height >> 2,
            _ => self.height,
        }
    }

    /// Fill the image from 8-bit-per-pixel palette indices.
    pub(crate) fn decode_bytes(&mut self, content: &[u8], offset: usize) {
        let width = self.get_original_width();
        let height = self.get_original_height();
        for y in 0..height {
            for x in 0..width {
                let rgb = self.content_palette[content[offset + y * width + x] as usize];
                self.set_scaled_pixel(x, y, rgb);
            }
        }
    }

    /// Fill the image from 4-bit-per-pixel palette indices with the given stride.
    pub(crate) fn decode_nibbles(&mut self, content: &[u8], offset: usize, stride: usize) {
        let width = self.get_original_width();
        let height = self.get_original_height();
        for y in 0..height {
            for x in 0..width {
                let rgb =
                    self.content_palette[get_nibble(content, offset + y * stride, x) as usize];
                self.set_scaled_pixel(x, y, rgb);
            }
        }
    }

    fn into_image(self) -> Image {
        Image {
            width: self.width,
            height: self.height,
            pixels: self
                .pixels
                .iter()
                .map(|&p| (p as u32) & 0x00FF_FFFF)
                .collect(),
        }
    }
}

/// The high or low nibble of `content[offset + index/2]`.
pub(crate) fn get_nibble(content: &[u8], offset: usize, index: usize) -> i32 {
    let b = content[offset + (index >> 1)] as i32;
    if index & 1 == 0 {
        b >> 4
    } else {
        b & 0xf
    }
}

pub(crate) fn clamp_u5(x: i32) -> i32 {
    x.clamp(0, 31)
}

/// Whether `s` appears at `offset` in `content`.
pub(crate) fn is_string_at(content: &[u8], offset: usize, s: &[u8]) -> bool {
    content.len() >= offset + s.len() && content[offset..offset + s.len()] == *s
}

/// Little-endian u32 at `offset`.
pub(crate) fn get32_le(content: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        content[offset],
        content[offset + 1],
        content[offset + 2],
        content[offset + 3],
    ])
}

/// Extract the lowercased file extension (without the dot).
fn extension(filename: &str) -> String {
    filename
        .rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether `filename`'s extension is a supported MSX graphics format.
pub fn is_supported(filename: &str) -> bool {
    matches!(
        extension(filename).as_str(),
        "sc2"
            | "grp"
            | "sc3"
            | "sc4"
            | "sc5"
            | "ge5"
            | "sc6"
            | "sc7"
            | "ge7"
            | "sc8"
            | "ge8"
            | "sr8"
            | "sca"
            | "scb"
            | "sra"
            | "scc"
            | "srs"
            | "yjk"
            | "shc"
            | "sr5"
            | "sr6"
            | "sr7"
            | "sri"
            | "gl5"
            | "sh5"
            | "gl6"
            | "sh6"
            | "gl7"
            | "sh7"
            | "gl8"
            | "sh8"
            | "gla"
            | "glb"
            | "sha"
            | "shb"
            | "glc"
            | "gls"
            | "g9b"
            | "stp"
            | "cmp"
            | "fnt"
            | "pct"
            | "mis"
            | "mif"
            | "mig"
            | "mag"
            | "mki"
            | "max"
    )
}

/// Decode an MSX-family graphics file into an [`Image`], or `None` if the file
/// is not a recognized/supported format.
pub fn decode(filename: &str, content: &[u8], companions: &dyn CompanionFiles) -> Option<Image> {
    let mut r = Recoil::new(companions);
    let ok = match extension(filename).as_str() {
        "sc2" | "grp" => r.decode_sc2(content),
        "sc3" => r.decode_sc3(content),
        "sc4" => r.decode_sc4(content),
        "sc5" | "ge5" => r.decode_sc5(content),
        "sc6" => r.decode_sc6(content),
        "sc7" | "ge7" => r.decode_sc7(content),
        "sc8" | "ge8" | "sr8" => r.decode_sc8(content),
        "sca" | "scb" | "sra" => r.decode_sca(content),
        "scc" | "srs" | "yjk" => r.decode_scc(content),
        "shc" => r.decode_glyjk(content, false),
        "sr5" => r.decode_sr5(content),
        "sr6" => r.decode_sr6(content),
        "sr7" => r.decode_sr7(content),
        "sri" => r.decode_sri(content),
        "gl5" | "sh5" => r.decode_gl5(content),
        "gl6" | "sh6" => r.decode_gl6(content, true),
        "gl7" | "sh7" => r.decode_gl7(content),
        "gl8" | "sh8" => r.decode_gl8(content),
        "gla" | "glb" | "sha" | "shb" => r.decode_glyjk(content, true),
        "glc" | "gls" => r.decode_glyjk(content, false),
        "g9b" => r.decode_g9b(content),
        "stp" => r.decode_gl6(content, false),
        "cmp" => r.decode_dd_graph(content),
        "fnt" | "pct" | "mis" => r.decode_pct(content),
        "mif" => r.decode_mif(content),
        "mig" => r.decode_mig(content),
        "mag" | "mki" | "max" => r.decode_mag(content),
        _ => false,
    };
    ok.then(|| r.into_image())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert!(is_supported("PIC.SC2"));
        assert!(is_supported("a.sc8"));
        assert!(is_supported("photo.G9B"));
        assert!(is_supported("img.gl5"));
        assert!(!is_supported("readme.txt"));
        assert!(!is_supported("noext"));
    }

    #[test]
    fn decodes_synthetic_sc2() {
        // Minimal SCREEN 2: BSAVE header, char 0 = a single top-left pixel,
        // colour fg=15 bg=1, name table all zero, no embedded palette.
        let mut buf = vec![0u8; 14343];
        buf[0] = 0xfe;
        buf[3] = 0xff; // header end = 0x37ff
        buf[4] = 0x37;
        buf[7] = 0x80; // pattern char 0, row 0: leftmost pixel set
        buf[0x2007] = 0xf1; // colour char 0, row 0: fg=15, bg=1

        let img = decode("pic.sc2", &buf, &NoCompanions).expect("decode");
        assert_eq!((img.width, img.height), (256, 192));
        // TMS9918 palette: index 15 (white) for the set pixel, index 1 for bg.
        assert_eq!(img.pixels[0], 0xfafff8);
        assert_eq!(img.pixels[1], 0x000400);
    }

    #[test]
    fn unrecognized_returns_none() {
        assert!(decode("x.sc2", &[0u8; 10], &NoCompanions).is_none());
    }
}
