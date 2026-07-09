//! `mv` — rename or move a file/directory inside a disk image.

use msx_disk::charset::MsxCharset;
use msx_disk::fs::{write, DiskFs};

use crate::disk::{charset_or_detect, find_entry, to_fs_key, CmdResult};

/// Rename `from` to `to`. When `to` names an existing directory the entry is
/// moved into it (keeping its name). An existing target file is only replaced
/// with `force`.
pub fn apply(
    data: &[u8],
    from: &str,
    to: &str,
    force: bool,
    charset: Option<MsxCharset>,
) -> CmdResult<Vec<u8>> {
    let fs = DiskFs::mount(data.to_vec()).map_err(|e| e.to_string())?;
    let tree = fs.tree().map_err(|e| e.to_string())?;
    let charset = charset_or_detect(charset, &tree);

    let from_key = to_fs_key(charset, from)?;
    let source = find_entry(&tree, &from_key)
        .ok_or_else(|| format!("'{from}': not found"))?
        .clone();

    let to_key = to_fs_key(charset, to)?;
    let target_key = match find_entry(&tree, &to_key) {
        // `mv FILE DIR` moves into the directory under the same name.
        Some(dir) if dir.is_dir => format!("{}/{}", dir.path, source.name),
        _ => to_key,
    };
    if target_key.eq_ignore_ascii_case(&source.path) {
        return Err(format!("'{from}' and '{to}' are the same entry"));
    }

    let mut buffer = data.to_vec();
    if let Some(existing) = find_entry(&tree, &target_key) {
        if existing.is_dir {
            return Err(format!(
                "'{}' already exists and is a directory",
                existing.path
            ));
        }
        if !force {
            return Err(format!(
                "'{}' already exists; use --force to replace it",
                existing.path
            ));
        }
        buffer = write::delete(&buffer, &[existing.path.as_str()]).map_err(|e| e.to_string())?;
    }

    write::rename(&buffer, &source.path, &target_key).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;
    use msx_disk::fs::DiskFs;

    #[test]
    fn renames_a_file() {
        let out = apply(
            &testdisk::populated(),
            "hello.txt",
            "readme.txt",
            false,
            None,
        )
        .unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("README.TXT").unwrap(), b"hi there");
        assert!(fs.read_file("HELLO.TXT").is_err());
    }

    #[test]
    fn moves_a_file_into_an_existing_directory() {
        let out = apply(&testdisk::populated(), "HELLO.TXT", "utils", false, None).unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("UTILS/HELLO.TXT").unwrap(), b"hi there");
    }

    #[test]
    fn refuses_existing_target_unless_forced() {
        let disk = testdisk::populated();
        let with_two = msx_disk::fs::write::add_files(&disk, &[("OTHER.TXT", b"other")]).unwrap();
        let err = apply(&with_two, "HELLO.TXT", "OTHER.TXT", false, None).unwrap_err();
        assert!(err.contains("--force"), "{err}");

        let out = apply(&with_two, "HELLO.TXT", "OTHER.TXT", true, None).unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("OTHER.TXT").unwrap(), b"hi there");
        assert!(fs.read_file("HELLO.TXT").is_err());
    }

    #[test]
    fn missing_source_is_an_error() {
        let err = apply(&testdisk::populated(), "NOPE.TXT", "NEW.TXT", false, None).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn moving_onto_itself_is_an_error() {
        let err = apply(
            &testdisk::populated(),
            "HELLO.TXT",
            "hello.txt",
            false,
            None,
        )
        .unwrap_err();
        assert!(err.contains("same entry"), "{err}");
    }
}
