//! `rm` — remove files or directories from a disk image.

use msx_disk::charset::MsxCharset;
use msx_disk::fs::{write, DiskFs};

use crate::disk::{charset_or_detect, find_entry, removal_paths, to_fs_key, CmdResult};

/// Remove `paths`. Directories need `recursive`; with `force`, missing paths
/// are silently skipped.
pub fn apply(
    data: &[u8],
    paths: &[String],
    recursive: bool,
    force: bool,
    charset: Option<MsxCharset>,
) -> CmdResult<Vec<u8>> {
    let fs = DiskFs::mount(data.to_vec()).map_err(|e| e.to_string())?;
    let tree = fs.tree().map_err(|e| e.to_string())?;
    let charset = charset_or_detect(charset, &tree);

    let mut doomed: Vec<String> = Vec::new();
    for path in paths {
        let key = to_fs_key(charset, path)?;
        let Some(entry) = find_entry(&tree, &key) else {
            if force {
                continue;
            }
            return Err(format!("'{path}': not found"));
        };
        if entry.is_dir && !recursive {
            return Err(format!("'{path}' is a directory; use -r to remove it"));
        }
        doomed.extend(removal_paths(entry));
    }

    if doomed.is_empty() {
        return Ok(data.to_vec());
    }
    let refs: Vec<&str> = doomed.iter().map(String::as_str).collect();
    write::delete(data, &refs).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;
    use msx_disk::fs::DiskFs;

    fn names(data: Vec<u8>) -> Vec<String> {
        DiskFs::mount(data)
            .unwrap()
            .tree()
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    #[test]
    fn removes_a_file_case_insensitively() {
        let out = apply(
            &testdisk::populated(),
            &["hello.txt".into()],
            false,
            false,
            None,
        )
        .unwrap();
        assert_eq!(names(out), vec!["UTILS"]);
    }

    #[test]
    fn directory_requires_recursive() {
        let disk = testdisk::populated();
        let err = apply(&disk, &["UTILS".into()], false, false, None).unwrap_err();
        assert!(err.contains("-r"), "{err}");

        let out = apply(&disk, &["UTILS".into()], true, false, None).unwrap();
        assert_eq!(names(out), vec!["HELLO.TXT"]);
    }

    #[test]
    fn missing_path_errors_unless_forced() {
        let disk = testdisk::populated();
        assert!(apply(&disk, &["NOPE".into()], false, false, None).is_err());
        let out = apply(&disk, &["NOPE".into()], false, true, None).unwrap();
        assert_eq!(out, disk, "force on a missing path is a no-op");
    }

    #[test]
    fn removes_multiple_paths_in_one_call() {
        let out = apply(
            &testdisk::populated(),
            &["HELLO.TXT".into(), "UTILS".into()],
            true,
            false,
            None,
        )
        .unwrap();
        assert!(names(out).is_empty());
    }
}
