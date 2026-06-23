//! List the contents of an MSX `.cas` tape image.
//!
//! Usage: `cargo run -p msx-disk --example lscas -- <file.cas>`

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: lscas <file.cas>");
        std::process::exit(2);
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read error: {e}");
            std::process::exit(1);
        }
    };
    let files = msx_disk::cas::list(&bytes);
    if files.is_empty() {
        println!("no files found");
        return;
    }
    for f in &files {
        println!(
            "{:<8} {:<7} {:>8} bytes",
            f.name,
            f.kind.label(),
            f.data.len()
        );
    }
}
