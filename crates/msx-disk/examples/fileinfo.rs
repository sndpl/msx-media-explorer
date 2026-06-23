//! Print the content-derived info for one or more files.
//!
//! Usage: `cargo run -p msx-disk --example fileinfo -- <file> [<file> ...]`
//!
//! Each argument is a host file; its bytes are read and described via
//! [`msx_disk::fileinfo::describe`] (the same logic the GUI's Info pane uses).

use msx_disk::fileinfo;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: fileinfo <file> [<file> ...]");
        std::process::exit(2);
    }

    for path in &args {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let info = fileinfo::describe(path, &bytes);
        println!("== {path} ({} bytes) ==", bytes.len());
        if let Some(desc) = info.description {
            println!("  Description: {desc}");
        }
        if let Some(b) = info.bload {
            println!(
                "  BSAVE: start=0x{:04X} end=0x{:04X} exec=0x{:04X} ({} bytes)",
                b.start,
                b.end,
                b.exec,
                b.data_len()
            );
        }
        if let Some(g) = info.graphics {
            println!("  Graphics: {}", g.label);
        }
        if let Some(m) = info.music {
            println!("  Music: {}", m.format);
            if let Some(t) = m.title {
                println!("    Title: {t}");
            }
            if let Some(a) = m.author {
                println!("    Author: {a}");
            }
            if let Some(p) = m.positions {
                println!("    Positions: {p}");
            }
            if let Some(c) = m.channels {
                println!("    Channels: {c}");
            }
            for (k, v) in m.extra {
                println!("    {k}: {v}");
            }
        }
        println!();
    }
}
