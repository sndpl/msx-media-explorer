//! Decode an MSX graphics file from a disk image to PNG via the RECOIL port.
//!
//! Usage: `cargo run -p msx-disk --example recoilpng -- <image> <file> <out.png>`

use std::collections::HashSet;

use msx_disk::recoil::{self, FnCompanions};
use msx_disk::{DiskFs, DiskImage};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(image), Some(file), Some(out)) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: recoilpng <image> <file> <out.png>");
        std::process::exit(2);
    };
    let fs = DiskFs::from_image(&DiskImage::open(&image).expect("open")).expect("mount");
    let tree = fs.tree().expect("tree");
    let bytes = fs.read_file(&file).expect("read file");

    // Resolve companion files (.PLx / .Sxx) by matching siblings in the same
    // directory, case-insensitively on the extension.
    let dir = file.rsplit_once('/').map(|(d, _)| d.to_string());
    let stem = file
        .rsplit('/')
        .next()
        .and_then(|n| n.rsplit_once('.'))
        .map(|(s, _)| s.to_string())
        .unwrap_or_default();
    let companions = FnCompanions(|ext: &str| {
        let target = format!("{stem}.{ext}").to_ascii_uppercase();
        for e in tree.iter().flat_map(|e| e.walk()) {
            if e.is_dir {
                continue;
            }
            let same_dir = e.path.rsplit_once('/').map(|(d, _)| d.to_string()) == dir;
            if same_dir && e.name.to_ascii_uppercase() == target {
                return fs.read_file(&e.path).ok();
            }
        }
        None
    });

    let Some(img) = recoil::decode(&file, &bytes, &companions) else {
        eprintln!("unsupported or undecodable: {file}");
        std::process::exit(1);
    };

    let distinct: HashSet<u32> = img.pixels.iter().copied().collect();
    eprintln!(
        "{}x{} | distinct colors: {}",
        img.width,
        img.height,
        distinct.len()
    );

    let rgba = img.to_rgba();
    image::RgbaImage::from_raw(img.width as u32, img.height as u32, rgba)
        .expect("buffer")
        .save(&out)
        .expect("save");
    eprintln!("wrote {out}");
}
