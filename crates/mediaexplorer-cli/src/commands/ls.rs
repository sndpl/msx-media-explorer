//! `ls` — render a directory listing of a disk image.

use msx_disk::charset::MsxCharset;
use msx_disk::fs::{detect_dos_version, DirEntry, DiskFs, DosVersion};
use msx_disk::image::geometry::SECTOR_SIZE;

use crate::disk::{charset_or_detect, find_entry, to_fs_key, CmdResult};

pub struct Opts {
    pub long: bool,
    pub recursive: bool,
    pub charset: Option<MsxCharset>,
}

/// Render the listing for `path` (or the root) as a ready-to-print string.
pub fn render(data: &[u8], path: Option<&str>, opts: &Opts) -> CmdResult<String> {
    let fs = DiskFs::mount(data.to_vec()).map_err(|e| e.to_string())?;
    let tree = fs.tree().map_err(|e| e.to_string())?;
    let charset = charset_or_detect(opts.charset, &tree);

    let entries: &[DirEntry] = match path {
        None => &tree,
        Some(p) => {
            let key = to_fs_key(charset, p)?;
            let entry = find_entry(&tree, &key).ok_or_else(|| format!("'{p}': not found"))?;
            if entry.is_dir {
                &entry.children
            } else {
                std::slice::from_ref(entry)
            }
        }
    };

    let mut out = String::new();
    if opts.long {
        out.push_str(&header(data, &tree, fs.volume_label()));
    }
    for entry in entries {
        render_entry(&mut out, entry, 0, charset, opts);
    }
    Ok(out)
}

/// `-l` header: volume label (if any), detected MSX-DOS generation, and size.
fn header(data: &[u8], tree: &[DirEntry], label: Option<String>) -> String {
    let dos = match detect_dos_version(&data[..SECTOR_SIZE.min(data.len())], tree) {
        DosVersion::Dos1 => "MSX-DOS 1",
        DosVersion::Dos2 => "MSX-DOS 2",
    };
    let label = label.unwrap_or_else(|| "(no label)".into());
    format!(
        "volume: {label}  boot: {dos}  size: {} KB\n",
        data.len() / 1024
    )
}

fn render_entry(out: &mut String, entry: &DirEntry, depth: usize, cs: MsxCharset, opts: &Opts) {
    let name = entry.display_name(cs);
    let indent = "  ".repeat(depth);
    if opts.long {
        let a = &entry.attributes;
        let flags: String = [
            (entry.is_dir, 'd'),
            (a.read_only, 'r'),
            (a.hidden, 'h'),
            (a.system, 's'),
            (a.archive, 'a'),
        ]
        .iter()
        .map(|&(on, c)| if on { c } else { '-' })
        .collect();
        let size = if entry.is_dir {
            "<DIR>".to_string()
        } else {
            entry.size.to_string()
        };
        let when = entry.modified.map_or_else(String::new, |t| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                t.year, t.month, t.day, t.hour, t.minute
            )
        });
        out.push_str(&format!(
            "{flags}  {size:>8}  {when:<16}  {indent}{name}{}\n",
            if entry.is_dir { "/" } else { "" }
        ));
    } else {
        out.push_str(&format!(
            "{indent}{name}{}\n",
            if entry.is_dir { "/" } else { "" }
        ));
    }
    if opts.recursive {
        for child in &entry.children {
            render_entry(out, child, depth + 1, cs, opts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;
    use msx_disk::charset::byte_to_pua;
    use msx_disk::fs::write;

    fn flat() -> Opts {
        Opts {
            long: false,
            recursive: false,
            charset: None,
        }
    }

    #[test]
    fn lists_root_entries_marking_directories() {
        let listing = render(&testdisk::populated(), None, &flat()).unwrap();
        assert!(listing.contains("HELLO.TXT\n"));
        assert!(listing.contains("UTILS/\n"));
        assert!(!listing.contains("GAME.COM"), "flat ls must not recurse");
    }

    #[test]
    fn recursive_listing_indents_children() {
        let opts = Opts {
            recursive: true,
            ..flat()
        };
        let listing = render(&testdisk::populated(), None, &opts).unwrap();
        assert!(listing.contains("UTILS/\n  GAME.COM\n"));
    }

    #[test]
    fn lists_a_subdirectory_by_path_case_insensitively() {
        let listing = render(&testdisk::populated(), Some("utils"), &flat()).unwrap();
        assert_eq!(listing, "GAME.COM\n");
    }

    #[test]
    fn missing_path_is_an_error() {
        let err = render(&testdisk::populated(), Some("NOPE"), &flat()).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn long_listing_has_header_sizes_and_flags() {
        let opts = Opts {
            long: true,
            ..flat()
        };
        let listing = render(&testdisk::populated(), None, &opts).unwrap();
        assert!(listing.starts_with("volume:"), "{listing}");
        // A subdirectory alone marks the disk as DOS 2 (only DOS 2 creates them).
        assert!(listing.contains("boot: MSX-DOS 2"), "{listing}");
        assert!(listing.contains("<DIR>"));
        assert!(listing.contains('8'), "HELLO.TXT is 8 bytes");

        // A flat disk with no marker reads as DOS 1.
        let plain = write::add_files(&testdisk::blank(), &[("HELLO.TXT", b"hi".as_ref())]).unwrap();
        let listing = render(&plain, None, &opts).unwrap();
        assert!(listing.contains("boot: MSX-DOS 1"), "{listing}");
    }

    #[test]
    fn long_listing_reports_dos2_after_bootsector_install() {
        use msx_disk::fs::DosVersion;
        let disk = write::set_boot_block(&testdisk::blank(), DosVersion::Dos2, 7).unwrap();
        let opts = Opts {
            long: true,
            ..flat()
        };
        let listing = render(&disk, None, &opts).unwrap();
        assert!(listing.contains("boot: MSX-DOS 2"), "{listing}");
    }

    #[test]
    fn kana_names_decode_via_autodetected_japanese_charset() {
        let kana_key: String = [0xB1u8, 0xB2, 0xB3]
            .iter()
            .map(|&b| byte_to_pua(b))
            .collect();
        let disk = write::add_files(
            &testdisk::blank(),
            &[(format!("{kana_key}.BAS").as_str(), b"x".as_ref())],
        )
        .unwrap();
        let listing = render(&disk, None, &flat()).unwrap();
        assert!(
            listing.contains("\u{FF71}\u{FF72}\u{FF73}.BAS"),
            "{listing}"
        );
    }
}
