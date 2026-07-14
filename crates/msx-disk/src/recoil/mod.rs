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

/// A decodable MSX-family graphics format. Each variant maps to exactly one
/// decoder path; several file extensions may map to the same variant.
///
/// Used both to drive extension-based decoding ([`decode`]) and to force a
/// format when a file's extension is unknown or wrong ([`decode_as`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ImageFormat {
    Screen2,
    Screen3,
    Screen4,
    Screen5,
    Screen6,
    Screen7,
    Screen8,
    Screen10_11,
    Screen12,
    GraphSaurus5,
    GraphSaurus6,
    GraphSaurus7,
    GraphSaurusInterlace,
    Gl5,
    Gl6,
    Gl7,
    Gl8,
    GlYjk,
    GlYjkInterlace,
    Stp,
    G9b,
    DdGraph,
    DynamicPublisher,
    Mif,
    Mig,
    MakiChan,
    Pi,
}

impl ImageFormat {
    /// Every format, in a sensible order for a UI picker: standard SCREEN modes
    /// first, then compressed/container formats.
    pub fn all() -> &'static [ImageFormat] {
        use ImageFormat::*;
        &[
            Screen2,
            Screen3,
            Screen4,
            Screen5,
            Screen6,
            Screen7,
            Screen8,
            Screen10_11,
            Screen12,
            GraphSaurus5,
            GraphSaurus6,
            GraphSaurus7,
            GraphSaurusInterlace,
            Gl5,
            Gl6,
            Gl7,
            Gl8,
            GlYjk,
            GlYjkInterlace,
            Stp,
            G9b,
            DdGraph,
            DynamicPublisher,
            Mif,
            Mig,
            MakiChan,
            Pi,
        ]
    }

    /// Human-readable label for a UI picker.
    pub fn label(self) -> &'static str {
        use ImageFormat::*;
        match self {
            Screen2 => "SCREEN 2",
            Screen3 => "SCREEN 3",
            Screen4 => "SCREEN 4",
            Screen5 => "SCREEN 5",
            Screen6 => "SCREEN 6",
            Screen7 => "SCREEN 7",
            Screen8 => "SCREEN 8",
            Screen10_11 => "SCREEN 10/11",
            Screen12 => "SCREEN 12 (YJK)",
            GraphSaurus5 => "Graph Saurus RLE (SCREEN 5)",
            GraphSaurus6 => "Graph Saurus RLE (SCREEN 6)",
            GraphSaurus7 => "Graph Saurus RLE (SCREEN 7)",
            GraphSaurusInterlace => "Graph Saurus RLE (interlaced)",
            Gl5 => "GL shape (SCREEN 5)",
            Gl6 => "GL shape (SCREEN 6)",
            Gl7 => "GL shape (SCREEN 7)",
            Gl8 => "GL shape (SCREEN 8)",
            GlYjk => "GL shape (YJK)",
            GlYjkInterlace => "GL shape (YJK interlaced)",
            Stp => "Stamp (STP)",
            G9b => "GFX9000 (G9B)",
            DdGraph => "DD-Graph (CMP)",
            DynamicPublisher => "Dynamic Publisher (FNT/PCT)",
            Mif => "MIF",
            Mig => "MIG",
            MakiChan => "Maki-chan (MAG/MKI/MAX)",
            Pi => "Yanagisawa (PI)",
        }
    }
}

/// Map a lowercased, dot-stripped extension to the format that decodes it.
/// This is the single source of truth shared by [`is_supported`] and [`decode`].
fn format_for_extension(ext: &str) -> Option<ImageFormat> {
    use ImageFormat::*;
    let format = match ext {
        "sc2" | "grp" => Screen2,
        "sc3" => Screen3,
        "sc4" => Screen4,
        "sc5" | "ge5" => Screen5,
        "sc6" => Screen6,
        "sc7" | "ge7" => Screen7,
        "sc8" | "ge8" | "sr8" | "pic" => Screen8,
        "sca" | "scb" | "sra" => Screen10_11,
        "scc" | "s12" | "srs" | "yjk" => Screen12,
        "sr5" => GraphSaurus5,
        "sr6" => GraphSaurus6,
        "sr7" => GraphSaurus7,
        "sri" => GraphSaurusInterlace,
        "gl5" | "sh5" => Gl5,
        "gl6" | "sh6" => Gl6,
        "gl7" | "sh7" => Gl7,
        "gl8" | "sh8" => Gl8,
        "shc" | "glc" | "gls" => GlYjk,
        "gla" | "glb" | "sha" | "shb" => GlYjkInterlace,
        "stp" => Stp,
        "g9b" => G9b,
        "cmp" => DdGraph,
        "fnt" | "pct" | "mis" => DynamicPublisher,
        "mif" => Mif,
        "mig" => Mig,
        "mag" | "mki" | "max" => MakiChan,
        "pi" => Pi,
        _ => return None,
    };
    Some(format)
}

/// Whether `filename`'s extension is a supported MSX graphics format.
pub fn is_supported(filename: &str) -> bool {
    format_for_extension(&extension(filename)).is_some()
}

/// Detect a self-describing MSX graphics format from its bytes alone, for files
/// whose extension is missing or misleading (e.g. a GL "shape" SCREEN 5 image
/// saved as `.PIC`, which the extension map treats as SCREEN 8).
///
/// Only formats with a distinctive magic (`G9B`/`MAG`/`PI`) or an exact,
/// dimension-checked payload length (the GL shapes) are sniffed. The plain BSAVE
/// screen dumps share a generic `0xfe` header and cannot be told apart from one
/// another without the extension, so they are never guessed here.
fn sniff(content: &[u8]) -> Option<ImageFormat> {
    sniff_magic(content).or_else(|| sniff_gl(content))
}

/// Formats identified by a leading magic signature.
fn sniff_magic(content: &[u8]) -> Option<ImageFormat> {
    use ImageFormat::*;
    if is_string_at(content, 0, b"G9B") && content.len() > 4 && content[3] == 11 && content[4] == 0
    {
        Some(G9b)
    } else if is_string_at(content, 0, b"MAKI02  ") {
        Some(MakiChan)
    } else if is_string_at(content, 0, b"Pi") {
        Some(Pi)
    } else {
        None
    }
}

/// A GL "shape" image: a 4-byte little-endian (width, height) header followed by
/// tightly-packed pixel data whose length pins down the bit depth. Width picks
/// the SCREEN mode (256 = SCREEN 5/8, 512 = SCREEN 6/7); at most one bit depth
/// matches the payload length, so the classification is unambiguous. A YJK GL
/// shape shares the 8bpp layout, so an 8bpp match resolves to SCREEN 8 (the far
/// more common case) — a YJK shape with a misleading extension must be forced.
fn sniff_gl(content: &[u8]) -> Option<ImageFormat> {
    use ImageFormat::*;
    if content.len() < 5 {
        return None;
    }
    let width = content[0] as usize | (content[1] as usize) << 8;
    let height = content[2] as usize | (content[3] as usize) << 8;
    if !matches!(width, 256 | 512) || height == 0 || height > 512 {
        return None;
    }
    let pixels = width * height;
    match content.len() - 4 {
        n if n == pixels && width == 256 => Some(Gl8), // 8 bpp
        n if n == (pixels + 1) >> 1 => Some(if width == 256 { Gl5 } else { Gl7 }), // 4 bpp
        n if n == (pixels + 3) >> 2 && width == 512 => Some(Gl6), // 2 bpp
        _ => None,
    }
}

/// Decode an MSX-family graphics file into an [`Image`], or `None` if the file
/// is not a recognized/supported format. The format is chosen by extension; if
/// the extension is unknown or its bytes do not decode under it, self-describing
/// formats are detected from the content (see [`sniff`]). Use [`decode_as`] to
/// force a specific format regardless of extension.
pub fn decode(filename: &str, content: &[u8], companions: &dyn CompanionFiles) -> Option<Image> {
    if let Some(format) = format_for_extension(&extension(filename)) {
        if let Some(image) = decode_as(format, content, companions) {
            return Some(image);
        }
    }
    // The extension was unknown, or its format could not decode these bytes.
    // Fall back to content-based detection so e.g. a GL SCREEN 5 image saved as
    // `.PIC` (an extension that otherwise means SCREEN 8) still renders.
    decode_as(sniff(content)?, content, companions)
}

/// Decode `content` as an explicit [`ImageFormat`], ignoring the file's
/// extension. Returns `None` if the bytes do not satisfy that format's header.
///
/// For the plain BSAVE screen modes, an image `BLOAD`'d to a non-zero VRAM
/// address (a partial screen / tile sheet) is rejected by the strict decoder.
/// When that happens, the data is placed into a full screen page at its VRAM
/// offset (the rest left black) and decoding is retried, so forcing the format
/// renders such images instead of showing nothing.
pub fn decode_as(
    format: ImageFormat,
    content: &[u8],
    companions: &dyn CompanionFiles,
) -> Option<Image> {
    if let Some(image) = decode_screen_format(format, content, companions) {
        return Some(image);
    }
    let rebuilt = rebuild_offset_bsave(format, content)?;
    decode_screen_format(format, &rebuilt, companions)
}

/// Dispatch a single [`ImageFormat`] to its decoder, with no offset rebuild.
fn decode_screen_format(
    format: ImageFormat,
    content: &[u8],
    companions: &dyn CompanionFiles,
) -> Option<Image> {
    use ImageFormat::*;
    let mut r = Recoil::new(companions);
    let ok = match format {
        Screen2 => r.decode_sc2(content),
        Screen3 => r.decode_sc3(content),
        Screen4 => r.decode_sc4(content),
        Screen5 => r.decode_sc5(content),
        Screen6 => r.decode_sc6(content),
        Screen7 => r.decode_sc7(content),
        Screen8 => r.decode_sc8(content),
        Screen10_11 => r.decode_sca(content),
        Screen12 => r.decode_scc(content),
        GraphSaurus5 => r.decode_sr5(content),
        GraphSaurus6 => r.decode_sr6(content),
        GraphSaurus7 => r.decode_sr7(content),
        GraphSaurusInterlace => r.decode_sri(content),
        Gl5 => r.decode_gl5(content),
        Gl6 => r.decode_gl6(content, true),
        Gl7 => r.decode_gl7(content),
        Gl8 => r.decode_gl8(content),
        GlYjk => r.decode_glyjk(content, false),
        GlYjkInterlace => r.decode_glyjk(content, true),
        Stp => r.decode_gl6(content, false),
        G9b => r.decode_g9b(content),
        DdGraph => r.decode_dd_graph(content),
        DynamicPublisher => r.decode_pct(content),
        Mif => r.decode_mif(content),
        Mig => r.decode_mig(content),
        MakiChan => r.decode_mag(content),
        Pi => r.decode_pi(content),
    };
    ok.then(|| r.into_image())
}

/// VRAM bytes per scanline for the plain BSAVE screen modes, or `None` for
/// formats that are not a from-VRAM BSAVE dump (Graph Saurus, GL, MAG, ...).
fn screen_bytes_per_line(format: ImageFormat) -> Option<usize> {
    use ImageFormat::*;
    match format {
        Screen5 | Screen6 => Some(128),
        Screen7 | Screen8 | Screen10_11 | Screen12 => Some(256),
        _ => None,
    }
}

/// If `content` is a BSAVE (`0xfe`) image loaded to a non-zero VRAM start
/// address, rebuild a full 212-line screen page with the data placed at that
/// offset and the rest left black. Returns `None` when no rebuild applies:
/// not a plain screen mode, not a BSAVE, a from-VRAM-0 image, or a start
/// address beyond the page.
fn rebuild_offset_bsave(format: ImageFormat, content: &[u8]) -> Option<Vec<u8>> {
    let bytes_per_line = screen_bytes_per_line(format)?;
    if content.len() < 8 || content[0] != 0xfe {
        return None;
    }
    let start = content[1] as usize | (content[2] as usize) << 8;
    if start == 0 {
        return None; // already a from-top image; the strict path handles it
    }
    const HEIGHT: usize = 212;
    let page = bytes_per_line * HEIGHT;
    if start >= page {
        return None;
    }
    let mut buf = vec![0u8; 7 + page];
    buf[0] = 0xfe;
    let end = page - 1; // start/exec stay zero, so this is a from-VRAM-0 page
    buf[3] = (end & 0xff) as u8;
    buf[4] = ((end >> 8) & 0xff) as u8;
    let data = &content[7..];
    let n = data.len().min(page - start);
    buf[7 + start..7 + start + n].copy_from_slice(&data[..n]);
    Some(buf)
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
        assert!(is_supported("photo.s12"));
        assert!(is_supported("PHOTO.S12"));
        assert!(!is_supported("readme.txt"));
        assert!(!is_supported("noext"));
    }

    #[test]
    fn s12_is_scc_alias() {
        // Minimal valid SCREEN 12 (SCC) BSAVE buffer: marker 0xfe, start = 0,
        // end-address header = 0xbfff (192-line image), exec = 0. The size is
        // exactly 7 + (192 << 8) so the YJK pixel data fills the buffer.
        let mut buf = vec![0u8; 49159];
        buf[0] = 0xfe;
        buf[3] = 0xff;
        buf[4] = 0xbf;

        let via_scc = decode("img.scc", &buf, &NoCompanions).expect("scc decodes");
        let via_s12 = decode("img.s12", &buf, &NoCompanions).expect("s12 decodes");
        assert_eq!(
            (via_scc.width, via_scc.height),
            (via_s12.width, via_s12.height)
        );
        assert_eq!(via_scc.pixels, via_s12.pixels);
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

    /// A GL "shape" SCREEN 5 image (Graph Saurus / PEACH UP `.PIC`) has a 4-byte
    /// little-endian (width, height) header, 4bpp pixel data, and no BSAVE
    /// marker. Saved with a `.PIC` extension (which maps to SCREEN 8, and cannot
    /// decode it), it must still be auto-detected by content and rendered.
    #[test]
    fn decode_sniffs_gl5_from_pic_extension() {
        let (w, h) = (256usize, 212usize);
        let mut buf = vec![0u8; 4 + ((w * h + 1) >> 1)];
        buf[0] = (w & 0xff) as u8;
        buf[1] = (w >> 8) as u8;
        buf[2] = (h & 0xff) as u8;
        buf[3] = (h >> 8) as u8;
        let img = decode("PHOTO.PIC", &buf, &NoCompanions).expect("sniff GL5");
        assert_eq!((img.width, img.height), (256, 212));
    }

    /// The sniff is a fallback only: a real SCREEN 8 `.PIC` (a `0xfe` BSAVE dump)
    /// must keep decoding via its extension and not be shadowed by content
    /// detection.
    #[test]
    fn decode_pic_still_prefers_screen8_bsave() {
        // 256x212 SCREEN 8 BSAVE: marker 0xfe, end address = 256*212-1 = 0xd3ff.
        let mut buf = vec![0u8; 7 + 256 * 212];
        buf[0] = 0xfe;
        buf[3] = 0xff;
        buf[4] = 0xd3;
        let img = decode("PHOTO.PIC", &buf, &NoCompanions).expect("screen8");
        assert_eq!((img.width, img.height), (256, 212));
    }

    /// A self-describing image carrying an extension recoil cannot classify at
    /// all is still decoded from its content.
    #[test]
    fn decode_sniffs_unknown_extension() {
        let (w, h) = (256usize, 212usize);
        let mut buf = vec![0u8; 4 + ((w * h + 1) >> 1)];
        buf[0] = (w & 0xff) as u8;
        buf[1] = (w >> 8) as u8;
        buf[2] = (h & 0xff) as u8;
        buf[3] = (h >> 8) as u8;
        let img = decode("PHOTO.XYZ", &buf, &NoCompanions).expect("sniff by content");
        assert_eq!((img.width, img.height), (256, 212));
    }

    /// Build a GL shape buffer: 4-byte (width, height) header + `payload` bytes.
    fn gl_buf(width: usize, height: usize, payload: usize) -> Vec<u8> {
        let mut buf = vec![0u8; 4 + payload];
        buf[0] = (width & 0xff) as u8;
        buf[1] = (width >> 8) as u8;
        buf[2] = (height & 0xff) as u8;
        buf[3] = (height >> 8) as u8;
        buf
    }

    /// Each GL bit depth and width resolves to exactly one format. Note GL5
    /// (256 wide, 4bpp) and GL6 (512 wide, 2bpp) have the same payload size but
    /// are told apart by the header width.
    #[test]
    fn sniff_classifies_gl_family_by_dimensions() {
        let (w5, w6, h) = (256usize, 512usize, 212usize);
        // GL5: 256 wide, 4 bpp.
        assert_eq!(
            sniff(&gl_buf(w5, h, (w5 * h + 1) >> 1)),
            Some(ImageFormat::Gl5)
        );
        // GL7: 512 wide, 4 bpp.
        assert_eq!(
            sniff(&gl_buf(w6, h, (w6 * h + 1) >> 1)),
            Some(ImageFormat::Gl7)
        );
        // GL6: 512 wide, 2 bpp (same payload size as GL5, disambiguated by width).
        assert_eq!(
            sniff(&gl_buf(w6, h, (w6 * h + 3) >> 2)),
            Some(ImageFormat::Gl6)
        );
        // GL8: 256 wide, 8 bpp.
        assert_eq!(sniff(&gl_buf(w5, h, w5 * h)), Some(ImageFormat::Gl8));
    }

    /// The GL sniff rejects implausible dimensions and inexact payload lengths,
    /// and never mistakes an ordinary BSAVE dump or arbitrary bytes for an image.
    #[test]
    fn sniff_rejects_non_shapes() {
        // Wrong width (not a SCREEN page).
        assert!(sniff(&gl_buf(320, 200, 320 * 200)).is_none());
        // Right dimensions, payload one byte short of any bit depth.
        assert!(sniff(&gl_buf(256, 212, ((256 * 212 + 1) >> 1) - 1)).is_none());
        // A plain SCREEN 8 BSAVE dump (0xfe header) is not a GL shape.
        let mut bsave = vec![0u8; 7 + 256 * 212];
        bsave[0] = 0xfe;
        bsave[3] = 0xff;
        bsave[4] = 0xd3;
        assert!(sniff(&bsave).is_none());
        // Too small, and unrelated bytes.
        assert!(sniff(&[0u8; 4]).is_none());
        assert!(sniff(b"just some text, not an image at all").is_none());
    }

    /// Magic-signature formats are recognized by their header regardless of
    /// extension.
    #[test]
    fn sniff_classifies_magic_formats() {
        assert_eq!(
            sniff(b"MAKI02  rest of header"),
            Some(ImageFormat::MakiChan)
        );
        let mut g9b = vec![0u8; 20];
        g9b[0..3].copy_from_slice(b"G9B");
        g9b[3] = 11;
        g9b[4] = 0;
        assert_eq!(sniff(&g9b), Some(ImageFormat::G9b));
        assert_eq!(sniff(b"Pi\x1a comment"), Some(ImageFormat::Pi));
    }

    /// Same synthetic SCREEN 2 buffer as [`decodes_synthetic_sc2`], but carrying
    /// an extension recoil cannot classify. The extension path must fail, while
    /// forcing the format must decode it identically to the `.sc2` path.
    #[test]
    fn decode_as_forces_format_regardless_of_extension() {
        let mut buf = vec![0u8; 14343];
        buf[0] = 0xfe;
        buf[3] = 0xff;
        buf[4] = 0x37;
        buf[7] = 0x80;
        buf[0x2007] = 0xf1;

        // Unknown extension: the normal path can't classify it.
        assert!(decode("pic.dat", &buf, &NoCompanions).is_none());

        // Forcing SCREEN 2 decodes it the same as the .sc2 extension would.
        let forced = decode_as(ImageFormat::Screen2, &buf, &NoCompanions).expect("forced decode");
        let via_ext = decode("pic.sc2", &buf, &NoCompanions).expect("ext decode");
        assert_eq!(
            (forced.width, forced.height),
            (via_ext.width, via_ext.height)
        );
        assert_eq!(forced.pixels, via_ext.pixels);
    }

    /// Forcing a format whose header the bytes do not satisfy must fail cleanly
    /// rather than panic or return garbage.
    #[test]
    fn decode_as_mismatched_format_returns_none() {
        assert!(decode_as(ImageFormat::Screen8, &[0u8; 10], &NoCompanions).is_none());
    }

    /// A SCREEN 5 image BLOAD'd to a non-zero VRAM address (start=0x4600), like
    /// Dark Castle's `ENEMY*.PIC`. RECOIL's strict path rejects a non-zero start
    /// address; forcing SCREEN 5 must still render it by placing the data into a
    /// full screen page (the rest left black).
    #[test]
    fn decode_as_rebuilds_offset_bsave_screen5() {
        let start = 0x4600usize;
        let end = 0x6000usize;
        let data_len = end - start + 1;
        let mut buf = vec![0u8; 7 + data_len];
        buf[0] = 0xfe;
        buf[1] = (start & 0xff) as u8;
        buf[2] = (start >> 8) as u8;
        buf[3] = (end & 0xff) as u8;
        buf[4] = (end >> 8) as u8;
        buf[7..].fill(0x12); // arbitrary non-zero pixel data

        let img = decode_as(ImageFormat::Screen5, &buf, &NoCompanions).expect("rebuilt decode");
        assert_eq!((img.width, img.height), (256, 212));
    }

    /// A from-VRAM-0 image must decode at its natural height and must NOT be
    /// rebuilt into a full 212-line page (which would pad it with black).
    #[test]
    fn decode_as_keeps_zero_start_natural_height() {
        // end = 0x4600 -> height (0x4600+1)>>7 = 140 lines.
        let end = 0x4600usize;
        let data_len = end + 1;
        let mut buf = vec![0u8; 7 + data_len];
        buf[0] = 0xfe;
        buf[3] = (end & 0xff) as u8;
        buf[4] = (end >> 8) as u8;
        buf[7..].fill(0x12);

        let img = decode_as(ImageFormat::Screen5, &buf, &NoCompanions).expect("decode");
        assert_eq!((img.width, img.height), (256, 140));
    }

    /// The dropdown is populated from `ImageFormat::all()`; it must enumerate the
    /// formats with unique, non-empty labels.
    #[test]
    fn image_format_all_has_unique_nonempty_labels() {
        let all = ImageFormat::all();
        assert!(all.contains(&ImageFormat::Screen2));
        assert!(all.contains(&ImageFormat::Screen8));
        assert!(all.contains(&ImageFormat::Screen12));
        assert!(all.iter().all(|f| !f.label().is_empty()));

        let mut labels: Vec<&str> = all.iter().map(|f| f.label()).collect();
        let count = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), count, "labels must be unique");
    }
}
