//! Raw byte-run to bitmap interpretation, for the hex views' graphics preview.
//!
//! Where [`crate::recoil`] decodes a *file* whose format is known from its
//! extension or header, this module interprets an arbitrary window of bytes as
//! pixels under a layout the user picks. That is what finds uncompressed
//! graphics buried inside game data: sprite sheets, fonts, tile banks, and
//! screen dumps saved without a BSAVE header.
//!
//! [`render`] is total: bytes past the end of the buffer come out as colour 0,
//! so scrolling to the tail of a file can never panic or blank the view.

use crate::recoil::palette;
use crate::recoil::{Image, MAX_PIXELS};

/// How a run of bytes is unpacked into pixels.
///
/// The MSX entries mirror the bitmap formats of Binary Editor Bz for MSX;
/// [`Sprite16`](RawFormat::Sprite16) and [`Sc2Pattern`](RawFormat::Sc2Pattern)
/// are additions that Bz has no equivalent for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum RawFormat {
    /// 1 bit per pixel, read as continuous scanlines.
    Mono,
    /// 1bpp 8x8 tiles in a grid: SCREEN 0/1/2/4 patterns, fonts, 8x8 sprites.
    MonoTile8x8,
    /// 1bpp 8x16 tiles in a grid.
    MonoTile8x16,
    /// 1bpp 16x16 tiles in a grid, rows stored left byte then right byte:
    /// the kanji ROM layout.
    MonoTile16x16,
    /// 1bpp 16x16 MSX sprites: four 8x8 quadrants stored column-major
    /// (top-left, bottom-left, top-right, bottom-right).
    Sprite16,
    /// 2 bits per pixel: SCREEN 6/9.
    Bpp2,
    /// 4 bits per pixel, high nibble leftmost: SCREEN 5 (256 wide) / 7 (512).
    Bpp4,
    /// 8 bits per pixel through the fixed GRB332 palette: SCREEN 8.
    Bpp8,
    /// 8bpp YJK with the odd-luminance RGB escape: SCREEN 10/11.
    YjkRgb,
    /// 8bpp YJK with no escape: SCREEN 12.
    Yjk,
    /// SCREEN 2/4 pattern table combined with the colour table at +0x2000, so
    /// character sets render in their real foreground/background colours.
    Sc2Pattern,
}

impl RawFormat {
    /// Every format, in picker order: linear depths, then tile layouts.
    pub fn all() -> &'static [RawFormat] {
        use RawFormat::*;
        &[
            Mono,
            MonoTile8x8,
            MonoTile8x16,
            MonoTile16x16,
            Sprite16,
            Sc2Pattern,
            Bpp2,
            Bpp4,
            Bpp8,
            YjkRgb,
            Yjk,
        ]
    }

    /// Human-readable label for a UI picker.
    pub fn label(self) -> &'static str {
        use RawFormat::*;
        match self {
            Mono => "1bpp linear",
            MonoTile8x8 => "1bpp 8x8 tiles (patterns, fonts)",
            MonoTile8x16 => "1bpp 8x16 tiles",
            MonoTile16x16 => "1bpp 16x16 tiles (kanji ROM)",
            Sprite16 => "16x16 sprites",
            Sc2Pattern => "SCREEN 2/4 pattern + colour",
            Bpp2 => "2bpp (SCREEN 6/9)",
            Bpp4 => "4bpp (SCREEN 5/7)",
            Bpp8 => "8bpp (SCREEN 8)",
            YjkRgb => "8bpp YJK+RGB (SCREEN 10/11)",
            Yjk => "8bpp YJK (SCREEN 12)",
        }
    }

    /// A stable identifier for settings persistence, independent of the order
    /// of the enum. See [`RawFormat::from_id`].
    pub fn id(self) -> &'static str {
        use RawFormat::*;
        match self {
            Mono => "mono",
            MonoTile8x8 => "tile8x8",
            MonoTile8x16 => "tile8x16",
            MonoTile16x16 => "tile16x16",
            Sprite16 => "sprite16",
            Sc2Pattern => "sc2pattern",
            Bpp2 => "bpp2",
            Bpp4 => "bpp4",
            Bpp8 => "bpp8",
            YjkRgb => "yjkrgb",
            Yjk => "yjk",
        }
    }

    /// The format with this [`id`](RawFormat::id), if any.
    pub fn from_id(id: &str) -> Option<RawFormat> {
        RawFormat::all().iter().copied().find(|f| f.id() == id)
    }

    /// Bits per pixel in the packed data.
    fn bpp(self) -> usize {
        use RawFormat::*;
        match self {
            Mono | MonoTile8x8 | MonoTile8x16 | MonoTile16x16 | Sprite16 | Sc2Pattern => 1,
            Bpp2 => 2,
            Bpp4 => 4,
            Bpp8 | YjkRgb | Yjk => 8,
        }
    }

    /// Tile dimensions in pixels, for the tiled layouts.
    fn tile_size(self) -> Option<(usize, usize)> {
        use RawFormat::*;
        match self {
            MonoTile8x8 | Sc2Pattern => Some((8, 8)),
            MonoTile8x16 => Some((8, 16)),
            MonoTile16x16 | Sprite16 => Some((16, 16)),
            _ => None,
        }
    }

    /// The width a row must be a multiple of for this format to tile evenly.
    fn width_step(self) -> usize {
        use RawFormat::*;
        if let Some((tw, _)) = self.tile_size() {
            return tw;
        }
        match self {
            // A YJK quadruple spans four horizontal pixels.
            Yjk | YjkRgb => 4,
            _ => 8 / self.bpp(),
        }
    }

    /// The palette this format is normally viewed under, used to keep the
    /// picker sensible when the format changes.
    pub fn default_palette(self) -> PaletteId {
        use RawFormat::*;
        match self {
            Mono | MonoTile8x8 | MonoTile8x16 | MonoTile16x16 | Sprite16 | Yjk => PaletteId::Mono,
            Sc2Pattern => PaletteId::Msx1,
            Bpp2 => PaletteId::Screen6,
            Bpp4 | YjkRgb => PaletteId::Msx2Default,
            Bpp8 => PaletteId::Sc8,
        }
    }

    /// The width this format is normally viewed at, in pixels.
    pub fn default_width(self) -> usize {
        match self {
            RawFormat::Bpp2 => 512,
            _ => 256,
        }
    }
}

/// Where the preview's colours come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PaletteId {
    /// Black and white.
    Mono,
    /// The V9938 SCREEN 6 default four colours.
    Screen6,
    /// The TMS9928 fixed 16 colours (MSX1 screen modes).
    Msx1,
    /// The V9938 default 16 colours.
    Msx2Default,
    /// The fixed 256-colour GRB332 palette (SCREEN 8).
    Sc8,
    /// 16 colours read from the viewed buffer itself, at a byte offset.
    FromFile { offset: usize },
}

impl PaletteId {
    /// Every palette, in picker order. `FromFile` appears with a zero offset;
    /// the caller supplies the real one.
    pub fn all() -> &'static [PaletteId] {
        &[
            PaletteId::Mono,
            PaletteId::Screen6,
            PaletteId::Msx1,
            PaletteId::Msx2Default,
            PaletteId::Sc8,
            PaletteId::FromFile { offset: 0 },
        ]
    }

    /// Human-readable label for a UI picker.
    pub fn label(self) -> &'static str {
        match self {
            PaletteId::Mono => "Monochrome",
            PaletteId::Screen6 => "SCREEN 6 default (4)",
            PaletteId::Msx1 => "MSX1 / TMS9928 (16)",
            PaletteId::Msx2Default => "MSX2 default (16)",
            PaletteId::Sc8 => "SCREEN 8 GRB332 (256)",
            PaletteId::FromFile { .. } => "From file",
        }
    }

    /// A stable identifier for settings persistence. The `FromFile` offset is
    /// not part of it; persist that separately.
    pub fn id(self) -> &'static str {
        match self {
            PaletteId::Mono => "mono",
            PaletteId::Screen6 => "screen6",
            PaletteId::Msx1 => "msx1",
            PaletteId::Msx2Default => "msx2",
            PaletteId::Sc8 => "sc8",
            PaletteId::FromFile { .. } => "file",
        }
    }

    /// The palette with this [`id`](PaletteId::id), if any. `"file"` yields a
    /// zero offset.
    pub fn from_id(id: &str) -> Option<PaletteId> {
        PaletteId::all().iter().copied().find(|p| p.id() == id)
    }

    /// Whether this palette reads its colours out of the viewed buffer.
    pub fn is_from_file(self) -> bool {
        matches!(self, PaletteId::FromFile { .. })
    }

    /// The colour table, as `0x00RRGGBB` entries. Indices past its end render
    /// as black.
    fn table(self, bytes: &[u8]) -> Vec<u32> {
        match self {
            PaletteId::Mono => vec![0x000000, 0xffffff],
            PaletteId::Screen6 => palette::MSX6_DEFAULT_RGB.to_vec(),
            PaletteId::Msx1 => palette::MSX1_RGB.to_vec(),
            PaletteId::Msx2Default => palette::msx2_default_rgb().to_vec(),
            PaletteId::Sc8 => palette::sc8_rgb().to_vec(),
            PaletteId::FromFile { offset } => palette::msx_palette_rgb(bytes, offset, 16),
        }
    }
}

/// What to draw: a layout, a palette, and the byte window to read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RawView {
    pub format: RawFormat,
    pub palette: PaletteId,
    /// Pixels across. Rounded down to whatever the format tiles evenly at.
    pub width: usize,
    /// Byte offset the top-left pixel is read from.
    pub offset: usize,
    /// Upper bound on the pixel rows produced, so the caller can render only
    /// what its panel can show.
    pub max_rows: usize,
}

/// The SCREEN 2/4 colour table sits one 8 kB VRAM bank after the pattern table.
const SC2_COLOUR_TABLE: usize = 0x2000;

/// A byte of the buffer, or 0 past its end. This is what makes [`render`] total.
fn at(bytes: &[u8], i: usize) -> u8 {
    bytes.get(i).copied().unwrap_or(0)
}

/// Interpret the bytes of `bytes` from `view.offset` as an image.
///
/// Always succeeds. An empty result (`width` or `height` of 0) means there was
/// nothing to draw — an empty buffer, a zero width, or an offset past the end.
pub fn render(bytes: &[u8], view: &RawView) -> Image {
    let format = view.format;
    let step = format.width_step();
    let width = view.width / step * step;
    if width == 0 || view.max_rows == 0 || view.offset >= bytes.len() {
        return Image {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
    }

    let available = bytes.len() - view.offset;
    let height = plan_height(format, width, available, view.max_rows);
    if height == 0 {
        return Image {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
    }

    let colors = view.palette.table(bytes);
    let color = |i: usize| colors.get(i).copied().unwrap_or(0);
    let mut pixels = vec![0u32; width * height];
    let geometry = Geometry {
        offset: view.offset,
        width,
        height,
    };

    match format {
        RawFormat::Yjk | RawFormat::YjkRgb => {
            draw_yjk(bytes, geometry, &colors, &mut pixels, format)
        }
        RawFormat::Sc2Pattern => draw_sc2_pattern(bytes, geometry, &mut pixels, &color),
        _ => match format.tile_size() {
            Some(tile) => draw_tiles(bytes, geometry, tile, format, &mut pixels, &color),
            None => draw_linear(bytes, geometry, format.bpp(), &mut pixels, &color),
        },
    }

    Image {
        width,
        height,
        pixels,
    }
}

/// Where a painter reads from and how big the target is.
#[derive(Clone, Copy)]
struct Geometry {
    offset: usize,
    width: usize,
    height: usize,
}

/// How many pixel rows to produce: enough to show `available` bytes, capped by
/// the caller's `max_rows` and by [`MAX_PIXELS`]. Tiled layouts render whole
/// tile rows only; a partial tile at the end is padded with colour 0.
fn plan_height(format: RawFormat, width: usize, available: usize, max_rows: usize) -> usize {
    let cap = max_rows.min(MAX_PIXELS / width);
    match format.tile_size() {
        Some((tw, th)) => {
            let across = (width / tw).max(1);
            let bytes_per_tile = tw * th * format.bpp() / 8;
            let rows_of_tiles = available.div_ceil(bytes_per_tile).div_ceil(across).max(1);
            // Always at least one tile row: a panel too short for a whole tile
            // should scroll, not render nothing.
            rows_of_tiles.min((cap / th).max(1)) * th
        }
        None => {
            let stride = (width * format.bpp() / 8).max(1);
            available.div_ceil(stride).min(cap)
        }
    }
}

/// Packed pixels read as continuous scanlines, most significant bit first.
fn draw_linear(
    bytes: &[u8],
    geom: Geometry,
    bpp: usize,
    pixels: &mut [u32],
    color: &dyn Fn(usize) -> u32,
) {
    let Geometry {
        offset,
        width,
        height,
    } = geom;
    let stride = width * bpp / 8;
    let per_byte = 8 / bpp;
    let mask = (1usize << bpp) - 1;
    for y in 0..height {
        let row = offset + y * stride;
        for x in 0..width {
            let b = at(bytes, row + x / per_byte) as usize;
            // Shift so the leftmost pixel of a byte is its high bits.
            let shift = (per_byte - 1 - x % per_byte) * bpp;
            pixels[y * width + x] = color((b >> shift) & mask);
        }
    }
}

/// Rectangular tiles laid out left to right, then top to bottom. Tile bytes are
/// row-major; a 16-pixel-wide tile stores the left byte of a row before the
/// right one. [`RawFormat::Sprite16`] instead uses the MSX sprite quadrant
/// order, so it is handled separately below.
fn draw_tiles(
    bytes: &[u8],
    geom: Geometry,
    (tw, th): (usize, usize),
    format: RawFormat,
    pixels: &mut [u32],
    color: &dyn Fn(usize) -> u32,
) {
    let Geometry {
        offset,
        width,
        height,
    } = geom;
    let across = (width / tw).max(1);
    let bytes_per_tile = tw * th / 8; // every tiled format here is 1bpp
    let sprite = format == RawFormat::Sprite16;
    for y in 0..height {
        for x in 0..width {
            let tile = (y / th) * across + x / tw;
            let (ix, iy) = (x % tw, y % th);
            let byte = if sprite {
                // Four 8x8 quadrants stored column-major.
                (ix / 8) * 16 + (iy / 8) * 8 + iy % 8
            } else {
                iy * (tw / 8) + ix / 8
            };
            let b = at(bytes, offset + tile * bytes_per_tile + byte);
            let bit = (b >> (7 - ix % 8)) & 1;
            pixels[y * width + x] = color(bit as usize);
        }
    }
}

/// SCREEN 2/4 character cells: the pattern table at `offset` supplies the bits,
/// the colour table one 8 kB bank later supplies foreground/background per row.
/// Without a colour table in range the cell falls back to white on black, so
/// pattern data at the very end of a buffer still reads.
fn draw_sc2_pattern(
    bytes: &[u8],
    geom: Geometry,
    pixels: &mut [u32],
    color: &dyn Fn(usize) -> u32,
) {
    let Geometry {
        offset,
        width,
        height,
    } = geom;
    let across = (width / 8).max(1);
    for y in 0..height {
        for x in 0..width {
            let cell = (y / 8) * across + x / 8;
            let index = cell * 8 + y % 8;
            let pattern = at(bytes, offset + index);
            let attr = bytes
                .get(offset + SC2_COLOUR_TABLE + index)
                .copied()
                .unwrap_or(0xf0);
            let lit = (pattern >> (7 - x % 8)) & 1 == 1;
            let idx = if lit { attr >> 4 } else { attr & 0x0f };
            pixels[y * width + x] = color(idx as usize);
        }
    }
}

/// YJK pixels: each horizontal group of four bytes carries one luminance per
/// pixel plus a shared J/K chroma pair. In the SCREEN 10/11 variant an odd
/// luminance escapes to a palette colour instead.
fn draw_yjk(bytes: &[u8], geom: Geometry, colors: &[u32], pixels: &mut [u32], format: RawFormat) {
    let Geometry {
        offset,
        width,
        height,
    } = geom;
    let use_palette = format == RawFormat::YjkRgb;
    for y in 0..height {
        let row = offset + y * width;
        for x in 0..width {
            let luma = (at(bytes, row + x) >> 3) as i32;
            if use_palette && (luma & 1) != 0 {
                pixels[y * width + x] = colors.get((luma >> 1) as usize).copied().unwrap_or(0);
                continue;
            }
            let base = row + (x & !3);
            let k = (at(bytes, base) & 7) as i32 | ((at(bytes, base + 1) & 7) as i32) << 3;
            let j = (at(bytes, base + 2) & 7) as i32 | ((at(bytes, base + 3) & 7) as i32) << 3;
            pixels[y * width + x] =
                palette::yjk_to_rgb(luma, palette::yjk_signed(j), palette::yjk_signed(k));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view over `format` at its natural width, reading from byte 0.
    fn view(format: RawFormat, rows: usize) -> RawView {
        RawView {
            format,
            palette: format.default_palette(),
            width: format.default_width(),
            offset: 0,
            max_rows: rows,
        }
    }

    /// The colour at `(x, y)` of a rendered image.
    fn px(img: &Image, x: usize, y: usize) -> u32 {
        img.pixels[y * img.width + x]
    }

    #[test]
    fn mono_linear_lights_the_high_bit_first() {
        let mut buf = vec![0u8; 256];
        buf[0] = 0x80; // leftmost pixel of the first row
        let img = render(&buf, &view(RawFormat::Mono, 8));
        assert_eq!(img.width, 256);
        assert_eq!(px(&img, 0, 0), 0xffffff);
        assert_eq!(px(&img, 1, 0), 0x000000);
    }

    #[test]
    fn bpp2_reads_two_bits_per_pixel_high_first() {
        // 0b11_10_01_00: pixel 0 = 3, pixel 1 = 2, pixel 2 = 1, pixel 3 = 0.
        let buf = vec![0b1110_0100u8; 512];
        let img = render(&buf, &view(RawFormat::Bpp2, 4));
        assert_eq!(img.width, 512);
        let pal = PaletteId::Screen6.table(&buf);
        assert_eq!(px(&img, 0, 0), pal[3]);
        assert_eq!(px(&img, 1, 0), pal[2]);
        assert_eq!(px(&img, 2, 0), pal[1]);
        assert_eq!(px(&img, 3, 0), pal[0]);
    }

    #[test]
    fn bpp4_puts_the_high_nibble_on_the_left() {
        let buf = vec![0x1fu8; 512];
        let img = render(&buf, &view(RawFormat::Bpp4, 4));
        let pal = PaletteId::Msx2Default.table(&buf);
        assert_eq!(px(&img, 0, 0), pal[1]);
        assert_eq!(px(&img, 1, 0), pal[15]);
    }

    #[test]
    fn bpp8_indexes_the_grb332_palette_directly() {
        let mut buf = vec![0u8; 512];
        buf[3] = 255;
        let img = render(&buf, &view(RawFormat::Bpp8, 2));
        assert_eq!(px(&img, 3, 0), 0xffffff);
        assert_eq!(px(&img, 2, 0), 0x000000);
    }

    #[test]
    fn tiles_fill_across_before_down() {
        // Tile 1 (the second tile across) has its top-left pixel set.
        let mut buf = vec![0u8; 64 * 8];
        buf[8] = 0x80;
        let img = render(&buf, &view(RawFormat::MonoTile8x8, 8));
        assert_eq!(px(&img, 8, 0), 0xffffff);
        assert_eq!(px(&img, 0, 0), 0x000000);
    }

    #[test]
    fn tall_tiles_use_their_full_height() {
        // Byte 15 is the last row of the first 8x16 tile.
        let mut buf = vec![0u8; 64 * 16];
        buf[15] = 0x80;
        let img = render(&buf, &view(RawFormat::MonoTile8x16, 16));
        assert_eq!(px(&img, 0, 15), 0xffffff);
    }

    #[test]
    fn wide_tiles_store_the_left_byte_of_a_row_first() {
        // Kanji layout: byte 1 is the right half of the first tile's first row.
        let mut buf = vec![0u8; 32 * 16];
        buf[1] = 0x80;
        let img = render(&buf, &view(RawFormat::MonoTile16x16, 16));
        assert_eq!(px(&img, 8, 0), 0xffffff);
        assert_eq!(px(&img, 0, 0), 0x000000);
    }

    /// The MSX sprite quadrant order is what distinguishes this from a plain
    /// 16x16 tile: byte 8 is the *bottom*-left quadrant, not the second row.
    #[test]
    fn sprite16_uses_msx_quadrant_order() {
        let mut buf = vec![0u8; 32 * 16];
        buf[8] = 0x80; // bottom-left quadrant, first row -> y = 8
        buf[16] = 0x80; // top-right quadrant, first row -> x = 8, y = 0
        let img = render(&buf, &view(RawFormat::Sprite16, 16));
        assert_eq!(px(&img, 0, 8), 0xffffff);
        assert_eq!(px(&img, 8, 0), 0xffffff);
        assert_eq!(px(&img, 0, 1), 0x000000);
    }

    #[test]
    fn sc2_pattern_colours_from_the_table_at_0x2000() {
        let mut buf = vec![0u8; 0x2000 + 64];
        buf[0] = 0x80; // first pixel of cell 0 lit
        buf[0x2000] = 0x2f; // fg = 2, bg = 15
        let img = render(&buf, &view(RawFormat::Sc2Pattern, 8));
        let pal = PaletteId::Msx1.table(&buf);
        assert_eq!(px(&img, 0, 0), pal[2]);
        assert_eq!(px(&img, 1, 0), pal[15]);
    }

    #[test]
    fn sc2_pattern_falls_back_to_white_on_black_without_a_colour_table() {
        let mut buf = vec![0u8; 64];
        buf[0] = 0x80;
        let img = render(&buf, &view(RawFormat::Sc2Pattern, 8));
        let pal = PaletteId::Msx1.table(&buf);
        assert_eq!(px(&img, 0, 0), pal[15]);
        assert_eq!(px(&img, 1, 0), pal[0]);
    }

    /// A flat YJK buffer has zero chroma, so every pixel is the grey ramp of
    /// its luminance — and must agree with the shared conversion.
    #[test]
    fn yjk_renders_grey_for_zero_chroma() {
        let buf = vec![0xf8u8; 1024]; // luminance 31, J = K = 0
        let img = render(&buf, &view(RawFormat::Yjk, 2));
        assert_eq!(px(&img, 0, 0), palette::yjk_to_rgb(31, 0, 0));
    }

    #[test]
    fn yjkrgb_escapes_odd_luminance_to_the_palette() {
        // 0x0f >> 3 = 1: odd, so pixel 0 takes palette entry 0.
        let mut buf = vec![0u8; 1024];
        buf[0] = 0x0f;
        let img = render(&buf, &view(RawFormat::YjkRgb, 2));
        let pal = PaletteId::Msx2Default.table(&buf);
        assert_eq!(px(&img, 0, 0), pal[0]);
    }

    #[test]
    fn short_buffer_pads_the_last_row_instead_of_panicking() {
        let buf = vec![0xffu8; 3];
        let img = render(&buf, &view(RawFormat::Mono, 8));
        assert_eq!(img.width, 256);
        assert_eq!(img.height, 1);
        assert_eq!(px(&img, 0, 0), 0xffffff);
        // Byte 3 onwards does not exist, so those pixels are colour 0.
        assert_eq!(px(&img, 24, 0), 0x000000);
    }

    #[test]
    fn empty_results_for_degenerate_inputs() {
        let buf = vec![0xffu8; 1024];
        let empty = |v: RawView| render(&buf, &v).pixels.is_empty();
        assert!(empty(RawView {
            width: 0,
            ..view(RawFormat::Bpp4, 8)
        }));
        assert!(empty(RawView {
            max_rows: 0,
            ..view(RawFormat::Bpp4, 8)
        }));
        assert!(empty(RawView {
            offset: 4096,
            ..view(RawFormat::Bpp4, 8)
        }));
        assert!(render(&[], &view(RawFormat::Bpp4, 8)).pixels.is_empty());
    }

    /// A caller asking for an absurd number of rows must be clamped, not
    /// allowed to allocate without bound.
    #[test]
    fn height_is_capped_by_max_pixels() {
        let buf = vec![0u8; 1 << 20];
        let img = render(
            &buf,
            &RawView {
                max_rows: usize::MAX,
                ..view(RawFormat::Bpp8, 0)
            },
        );
        assert!(img.width * img.height <= MAX_PIXELS);
        assert!(img.height > 0);
    }

    /// Widths that do not tile evenly are rounded down rather than producing a
    /// torn last column.
    #[test]
    fn width_is_rounded_down_to_the_format_step() {
        let buf = vec![0u8; 4096];
        let odd = |format: RawFormat, width: usize| {
            render(
                &buf,
                &RawView {
                    width,
                    ..view(format, 8)
                },
            )
            .width
        };
        assert_eq!(odd(RawFormat::Mono, 260), 256);
        assert_eq!(odd(RawFormat::Bpp4, 255), 254);
        assert_eq!(odd(RawFormat::MonoTile16x16, 260), 256);
        assert_eq!(odd(RawFormat::Yjk, 254), 252);
    }

    /// A panel with room for fewer rows than one tile is tall still gets a
    /// whole tile row to scroll, rather than an empty image.
    #[test]
    fn a_short_panel_still_gets_one_whole_tile_row() {
        let buf = vec![0u8; 4096];
        let img = render(&buf, &view(RawFormat::MonoTile16x16, 4));
        assert_eq!(img.height, 16);
    }

    #[test]
    fn formats_and_palettes_have_unique_labels_and_round_trip_by_id() {
        let formats = RawFormat::all();
        assert!(formats.iter().all(|f| !f.label().is_empty()));
        assert!(formats
            .iter()
            .all(|&f| RawFormat::from_id(f.id()) == Some(f)));
        let mut ids: Vec<&str> = formats.iter().map(|f| f.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "format ids must be unique");

        let palettes = PaletteId::all();
        assert!(palettes.iter().all(|p| !p.label().is_empty()));
        assert!(palettes
            .iter()
            .all(|&p| PaletteId::from_id(p.id()) == Some(p)));
        assert_eq!(RawFormat::from_id("nope"), None);
        assert_eq!(PaletteId::from_id("nope"), None);
    }

    /// The `FromFile` palette reads the buffer being viewed, which is what lets
    /// a screen dump be previewed under its own embedded colours.
    #[test]
    fn from_file_palette_reads_the_buffer() {
        let mut buf = vec![0u8; 512];
        buf[0] = 0x11; // entry 0: some non-black colour
        buf[1] = 0x01;
        let table = PaletteId::FromFile { offset: 0 }.table(&buf);
        assert_ne!(table[0], 0);
        assert_eq!(table.len(), 16);
    }
}
