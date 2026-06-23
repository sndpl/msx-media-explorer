//! Minimal CLI to list the contents of an MSX disk image.
//!
//! Usage: `cargo run -p msx-disk --example lsdsk -- <image-file>`

use msx_disk::{DiskFs, DiskImage};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: lsdsk <image-file>");
        std::process::exit(2);
    };

    let image = match DiskImage::open(&path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("failed to open image: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "format={:?} geometry={:?} sectors={}",
        image.format(),
        image.geometry(),
        image.sector_count()
    );

    let fs = match DiskFs::from_image(&image) {
        Ok(fs) => fs,
        Err(e) => {
            eprintln!("failed to mount filesystem: {e}");
            std::process::exit(1);
        }
    };
    if let Some(label) = fs.volume_label() {
        println!("volume label: {label}");
    }
    match fs.tree() {
        Ok(tree) => {
            for entry in &tree {
                print_entry(entry, 0);
            }
        }
        Err(e) => {
            eprintln!("failed to read directory: {e}");
            std::process::exit(1);
        }
    }
}

fn print_entry(entry: &msx_disk::DirEntry, depth: usize) {
    let indent = "  ".repeat(depth);
    if entry.is_dir {
        println!("{indent}{}/", entry.name);
        for child in &entry.children {
            print_entry(child, depth + 1);
        }
    } else {
        println!("{indent}{:<14} {:>8} bytes", entry.name, entry.size);
    }
}
