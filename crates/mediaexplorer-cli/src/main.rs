//! Thin I/O shell: parse arguments, run the pure command functions, write
//! results back. Exit codes: 0 success, 1 operational error, 2 usage error
//! (from clap).

mod cli;
mod commands;
mod disk;

use std::path::Path;

use clap::Parser;
use msx_disk::charset::MsxCharset;
use msx_disk::fs::partition;
use msx_disk::image::{DiskImage, ImageFormat};

use cli::{Cli, Command};

fn main() {
    if let Err(msg) = run(Cli::parse()) {
        eprintln!("mediaexplorer-cli: {msg}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Ls(args) => {
            let image = open_readable(&args.image)?;
            let opts = commands::ls::Opts {
                long: args.long,
                recursive: args.recursive,
                charset: args.charset.map(MsxCharset::from),
            };
            let listing = commands::ls::render(image.data(), args.path.as_deref(), &opts)?;
            print!("{listing}");
            Ok(())
        }
        Command::Add(args) => {
            let image = open_editable(&args.image)?;
            if args.as_name.is_some() && args.files.len() != 1 {
                return Err("--as requires exactly one host file".into());
            }
            let mut files = Vec::with_capacity(args.files.len());
            for host in &args.files {
                let name = match &args.as_name {
                    Some(n) => n.clone(),
                    None => host
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| format!("{}: not a usable file name", host.display()))?
                        .to_string(),
                };
                let bytes = std::fs::read(host).map_err(|e| format!("{}: {e}", host.display()))?;
                files.push((name, bytes));
            }
            let updated = commands::add::apply(
                image.data(),
                &files,
                args.dest.as_deref(),
                args.charset.map(MsxCharset::from),
                args.force,
            )?;
            write_back(&image, &args.image, &updated)?;
            println!("added {} file(s) to {}", files.len(), args.image.display());
            Ok(())
        }
        Command::Extract(args) => {
            let image = open_readable(&args.image)?;
            let planned = commands::extract::plan(
                image.data(),
                args.path.as_deref(),
                args.recursive,
                args.charset.map(MsxCharset::from),
            )?;
            for (rel, bytes, modified) in &planned {
                let target = args
                    .out
                    .join(rel.split('/').collect::<std::path::PathBuf>());
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                // Two byte-identical entries (crafted "fake" files) share a name;
                // suffix the later one so it does not overwrite the first.
                let target = msx_disk::hostname::free_target(&target);
                disk::write_extracted(&target, bytes, *modified)
                    .map_err(|e| format!("{}: {e}", target.display()))?;
            }
            println!(
                "extracted {} file(s) to {}",
                planned.len(),
                args.out.display()
            );
            Ok(())
        }
        Command::Rm(args) => {
            let image = open_editable(&args.image)?;
            let updated =
                commands::rm::apply(image.data(), &args.paths, args.recursive, args.force, None)?;
            write_back(&image, &args.image, &updated)?;
            println!("removed from {}", args.image.display());
            Ok(())
        }
        Command::Mv(args) => {
            let image = open_editable(&args.image)?;
            let updated =
                commands::mv::apply(image.data(), &args.from, &args.to, args.force, None)?;
            write_back(&image, &args.image, &updated)?;
            println!("renamed '{}' to '{}'", args.from, args.to);
            Ok(())
        }
        Command::Mkdir(args) => {
            let image = open_editable(&args.image)?;
            let updated = commands::mkdir::apply(image.data(), &args.paths, args.parents, None)?;
            write_back(&image, &args.image, &updated)?;
            println!("created {} director(y/ies)", args.paths.len());
            Ok(())
        }
        Command::New(args) => {
            match extension_format(&args.image) {
                Some(ImageFormat::Dsk) => {}
                _ => {
                    return Err(format!(
                        "{}: new writes raw disk images; use a .dsk extension",
                        args.image.display()
                    ))
                }
            }
            refuse_existing(&args.image, args.force)?;
            let bytes = commands::new::build(args.format.into(), args.dos.into(), volume_serial())?;
            disk::write_atomic(&args.image, &bytes)
                .map_err(|e| format!("{}: {e}", args.image.display()))?;
            println!(
                "created {} ({} KB)",
                args.image.display(),
                bytes.len() / 1024
            );
            Ok(())
        }
        Command::Bootsector(args) => {
            let image = open_readable(&args.image)?;
            match args.dos {
                None => {
                    println!("{}", commands::bootsector::describe(image.data())?);
                    Ok(())
                }
                Some(dos) => {
                    ensure_editable(&image, &args.image)?;
                    let updated =
                        commands::bootsector::apply(image.data(), dos.into(), volume_serial())?;
                    write_back(&image, &args.image, &updated)?;
                    println!("{}", commands::bootsector::describe(&updated)?);
                    Ok(())
                }
            }
        }
        Command::Convert(args) => {
            let image = open_readable(&args.input)?;
            let format = extension_format(&args.output).ok_or_else(|| {
                format!(
                    "{}: unknown output format; use a .dsk, .xsa, or .sav extension",
                    args.output.display()
                )
            })?;
            refuse_existing(&args.output, args.force)?;
            let bytes = commands::convert::encode(image.data(), format)?;
            disk::write_atomic(&args.output, &bytes)
                .map_err(|e| format!("{}: {e}", args.output.display()))?;
            println!(
                "converted {} ({:?}) to {} ({format:?})",
                args.input.display(),
                image.format(),
                args.output.display()
            );
            Ok(())
        }
    }
}

/// Open an image for reading, rejecting partitioned hard-disk images (they
/// hold multiple volumes; browse those in the GUI).
fn open_readable(path: &Path) -> Result<DiskImage, String> {
    let image = DiskImage::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if partition::is_partitioned(image.data()) {
        return Err(format!(
            "{}: partitioned hard-disk image; browse it with the MSX Media Explorer GUI",
            path.display()
        ));
    }
    Ok(image)
}

/// Open an image that is about to be modified.
fn open_editable(path: &Path) -> Result<DiskImage, String> {
    let image = open_readable(path)?;
    ensure_editable(&image, path)?;
    Ok(image)
}

fn ensure_editable(image: &DiskImage, path: &Path) -> Result<(), String> {
    if !image.is_writable() {
        return Err(format!(
            "{}: {:?} images are read-only; `convert` it to a .dsk first",
            path.display(),
            image.format()
        ));
    }
    Ok(())
}

/// Re-wrap updated sector data in the original container and save atomically.
fn write_back(image: &DiskImage, path: &Path, updated: &[u8]) -> Result<(), String> {
    let bytes = image.reencode(updated).map_err(|e| e.to_string())?;
    disk::write_atomic(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn extension_format(path: &Path) -> Option<ImageFormat> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(ImageFormat::from_extension)
}

fn refuse_existing(path: &Path, force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!(
            "{}: already exists; use --force to overwrite",
            path.display()
        ));
    }
    Ok(())
}

/// Volume serial for freshly written DOS 2 boot sectors, derived from the
/// clock like MSX-DOS 2's FORMAT does (uniqueness, not secrecy, is the goal).
fn volume_serial() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_secs() as u32) ^ now.subsec_nanos()
}
