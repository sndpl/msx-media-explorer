//! Render a raw byte window of any file as a bitmap, the way the GUI's hex
//! graphics preview does. Reproduces raw-layout bugs without the GUI.
//!
//! Usage: `cargo run -p msx-disk --example rawpng -- <file> <format> <width> <offset> <out.png>`
//!
//! `<format>` is a [`RawFormat`] id (`bpp4`, `tile8x8`, `sc2pattern`, ...) and
//! `<offset>` is decimal or `0x`-prefixed hex. Pass no arguments to list the ids.

use msx_disk::view::bitmap::{self, RawFormat, RawView};

/// Rows rendered by default: enough to see a whole SCREEN page and then some.
const ROWS: usize = 512;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file, format, width, offset, out] = args.as_slice() else {
        eprintln!("usage: rawpng <file> <format> <width> <offset> <out.png>");
        eprintln!("formats:");
        for f in RawFormat::all() {
            eprintln!("  {:<12} {}", f.id(), f.label());
        }
        std::process::exit(2);
    };

    let Some(format) = RawFormat::from_id(format) else {
        eprintln!("unknown format id: {format}");
        std::process::exit(2);
    };
    let width: usize = width.parse().expect("width");
    let offset = parse_offset(offset).expect("offset");
    let bytes = std::fs::read(file).expect("read file");

    let img = bitmap::render(
        &bytes,
        &RawView {
            format,
            palette: format.default_palette(),
            width,
            offset,
            max_rows: ROWS,
        },
    );
    if img.pixels.is_empty() {
        eprintln!(
            "nothing to render at offset {offset} of {} bytes",
            bytes.len()
        );
        std::process::exit(1);
    }
    eprintln!(
        "{} @ {:#x}: {}x{}",
        format.label(),
        offset,
        img.width,
        img.height
    );

    image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.to_rgba())
        .expect("buffer")
        .save(out)
        .expect("save");
    eprintln!("wrote {out}");
}

/// A decimal or `0x`-prefixed hexadecimal byte offset.
fn parse_offset(s: &str) -> Option<usize> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => usize::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}
