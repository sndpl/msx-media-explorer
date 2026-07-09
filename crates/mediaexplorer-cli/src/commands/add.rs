//! `add` — add or update files on a disk image.

use msx_disk::charset::MsxCharset;
use msx_disk::fs::{write, DiskFs};

use crate::disk::{charset_or_detect, find_entry, to_fs_key, CmdResult};

/// Add `files` (already-read host bytes, keyed by their intended image name)
/// under `dest`. Refuses to overwrite unless `force`.
pub fn apply(
    data: &[u8],
    files: &[(String, Vec<u8>)],
    dest: Option<&str>,
    charset: Option<MsxCharset>,
    force: bool,
) -> CmdResult<Vec<u8>> {
    let fs = DiskFs::mount(data.to_vec()).map_err(|e| e.to_string())?;
    let tree = fs.tree().map_err(|e| e.to_string())?;
    let charset = charset_or_detect(charset, &tree);

    let dest_key = match dest {
        None => None,
        Some(d) => {
            let key = to_fs_key(charset, d)?;
            let entry = find_entry(&tree, &key)
                .ok_or_else(|| format!("'{d}': no such directory; create it with mkdir first"))?;
            if !entry.is_dir {
                return Err(format!("'{d}' is a file, not a directory"));
            }
            Some(entry.path.clone())
        }
    };

    let mut keyed: Vec<(String, &[u8])> = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        let key = to_fs_key(charset, name)?;
        let full = match &dest_key {
            Some(d) => format!("{d}/{key}"),
            None => key,
        };
        if !force {
            if let Some(existing) = find_entry(&tree, &full) {
                return Err(format!(
                    "'{}' already exists on the image; use --force to overwrite",
                    existing.path
                ));
            }
        }
        keyed.push((full, bytes));
    }

    let refs: Vec<(&str, &[u8])> = keyed.iter().map(|(k, b)| (k.as_str(), *b)).collect();
    write::add_files(data, &refs).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;
    use msx_disk::fs::DiskFs;

    fn pair(name: &str, bytes: &[u8]) -> (String, Vec<u8>) {
        (name.to_string(), bytes.to_vec())
    }

    #[test]
    fn adds_a_file_uppercasing_its_name() {
        let out = apply(
            &testdisk::blank(),
            &[pair("readme.txt", b"hi")],
            None,
            None,
            false,
        )
        .unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("README.TXT").unwrap(), b"hi");
    }

    #[test]
    fn adds_into_an_existing_directory() {
        let out = apply(
            &testdisk::populated(),
            &[pair("tool.com", b"x")],
            Some("utils"),
            None,
            false,
        )
        .unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("UTILS/TOOL.COM").unwrap(), b"x");
    }

    #[test]
    fn missing_dest_directory_is_an_error() {
        let err = apply(
            &testdisk::blank(),
            &[pair("A.TXT", b"a")],
            Some("NOPE"),
            None,
            false,
        )
        .unwrap_err();
        assert!(err.contains("mkdir"), "{err}");
    }

    #[test]
    fn refuses_overwrite_without_force() {
        let disk = testdisk::populated();
        let err = apply(&disk, &[pair("hello.txt", b"new")], None, None, false).unwrap_err();
        assert!(err.contains("--force"), "{err}");

        let out = apply(&disk, &[pair("hello.txt", b"new")], None, None, true).unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("HELLO.TXT").unwrap(), b"new");
    }

    #[test]
    fn adds_multiple_files_in_one_transaction() {
        let out = apply(
            &testdisk::blank(),
            &[pair("A.TXT", b"a"), pair("B.TXT", b"b")],
            None,
            None,
            false,
        )
        .unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("A.TXT").unwrap(), b"a");
        assert_eq!(fs.read_file("B.TXT").unwrap(), b"b");
    }
}
