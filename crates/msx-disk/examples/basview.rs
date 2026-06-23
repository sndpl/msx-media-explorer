//! Detokenize a `.BAS` file inside a disk image.
//!
//! Usage: `cargo run -p msx-disk --example basview -- <image> <file-path>`

use msx_disk::{view::basic, DiskFs, DiskImage, MsxCharset};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(image), Some(file)) = (args.next(), args.next()) else {
        eprintln!("usage: basview <image> <file-path>");
        std::process::exit(2);
    };
    let disk = DiskImage::open(&image).expect("open image");
    let fs = DiskFs::from_image(&disk).expect("mount");
    let bytes = fs.read_file(&file).expect("read file");
    print!("{}", basic::detokenize(&bytes, MsxCharset::International));
}
