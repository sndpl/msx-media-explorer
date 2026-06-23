//! Render an MSX screen file from a disk image to a PNG (for verification).
//!
//! Usage: `cargo run -p msx-disk --example screenpng -- <image> <file> <out.png>`

use std::collections::HashSet;

use msx_disk::view::screen::{self, ScreenMode};
use msx_disk::{DiskFs, DiskImage};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(image), Some(file), Some(out)) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: screenpng <image> <file> <out.png>");
        std::process::exit(2);
    };
    let mode = ScreenMode::from_filename(&file).expect("unrecognized screen extension");
    let disk = DiskImage::open(&image).expect("open image");
    let fs = DiskFs::from_image(&disk).expect("mount");
    let bytes = fs.read_file(&file).expect("read file");

    let bmp = screen::render(mode, &bytes);

    // Quick decode sanity stats.
    let distinct: HashSet<[u8; 3]> = bmp
        .rgba
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    let non_black = bmp
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count();
    let total = bmp.width * bmp.height;
    eprintln!(
        "{mode:?} {}x{} | distinct colors: {} | non-black: {}/{} ({:.0}%)",
        bmp.width,
        bmp.height,
        distinct.len(),
        non_black,
        total,
        100.0 * non_black as f64 / total as f64
    );

    let img = image::RgbaImage::from_raw(bmp.width as u32, bmp.height as u32, bmp.rgba)
        .expect("buffer size");
    img.save(&out).expect("save png");
    eprintln!("wrote {out}");
}
