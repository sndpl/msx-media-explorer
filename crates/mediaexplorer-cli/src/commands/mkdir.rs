//! `mkdir` — create directories on a disk image.

use msx_disk::charset::MsxCharset;
use msx_disk::fs::{write, DiskFs};

use crate::disk::{charset_or_detect, find_entry, to_fs_key, CmdResult};

/// Create each of `paths`. With `parents`, missing parents are created and an
/// already-existing directory is not an error (like `mkdir -p`).
pub fn apply(
    data: &[u8],
    paths: &[String],
    parents: bool,
    charset: Option<MsxCharset>,
) -> CmdResult<Vec<u8>> {
    let fs = DiskFs::mount(data.to_vec()).map_err(|e| e.to_string())?;
    let tree = fs.tree().map_err(|e| e.to_string())?;
    let charset = charset_or_detect(charset, &tree);

    let mut buffer = data.to_vec();
    // Tracks keys created in this run, so `mkdir -p A A/B` and parent chains
    // see their own earlier work without remounting.
    let mut created: Vec<String> = Vec::new();
    let exists = |tree: &[msx_disk::fs::DirEntry], created: &[String], key: &str| {
        find_entry(tree, key).map(|e| e.is_dir).or_else(|| {
            created
                .iter()
                .any(|c| c.eq_ignore_ascii_case(key))
                .then_some(true)
        })
    };

    for path in paths {
        let key = to_fs_key(charset, path)?;
        let components: Vec<&str> = key.split('/').filter(|c| !c.is_empty()).collect();
        if components.is_empty() {
            return Err(format!("'{path}': not a valid directory name"));
        }

        let mut prefix = String::new();
        for (i, component) in components.iter().enumerate() {
            let is_last = i == components.len() - 1;
            let full = if prefix.is_empty() {
                (*component).to_string()
            } else {
                format!("{prefix}/{component}")
            };
            match exists(&tree, &created, &full) {
                Some(true) if is_last && !parents => {
                    return Err(format!("'{path}': already exists"));
                }
                Some(false) => {
                    return Err(format!("'{full}' exists and is a file"));
                }
                Some(true) => {}
                None => {
                    if !is_last && !parents {
                        return Err(format!(
                            "'{full}': no such directory; use -p to create parents"
                        ));
                    }
                    buffer = write::create_dir(&buffer, &full).map_err(|e| e.to_string())?;
                    created.push(full.clone());
                }
            }
            prefix = full;
        }
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;
    use msx_disk::fs::DiskFs;

    fn dirs(data: Vec<u8>) -> Vec<String> {
        DiskFs::mount(data)
            .unwrap()
            .tree()
            .unwrap()
            .iter()
            .flat_map(|e| e.walk().map(|d| d.path.clone()).collect::<Vec<_>>())
            .collect()
    }

    #[test]
    fn creates_a_directory() {
        let out = apply(&testdisk::blank(), &["games".into()], false, None).unwrap();
        assert!(dirs(out).contains(&"GAMES".to_string()));
    }

    #[test]
    fn nested_path_requires_parents_flag() {
        let disk = testdisk::blank();
        let err = apply(&disk, &["A/B/C".into()], false, None).unwrap_err();
        assert!(err.contains("-p"), "{err}");

        let out = apply(&disk, &["A/B/C".into()], true, None).unwrap();
        let all = dirs(out);
        for d in ["A", "A/B", "A/B/C"] {
            assert!(all.contains(&d.to_string()), "missing {d} in {all:?}");
        }
    }

    #[test]
    fn existing_directory_errors_without_parents_flag() {
        let disk = testdisk::populated();
        let err = apply(&disk, &["UTILS".into()], false, None).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        // -p tolerates it.
        let out = apply(&disk, &["UTILS".into()], true, None).unwrap();
        assert_eq!(out, disk);
    }

    #[test]
    fn a_file_in_the_way_is_an_error() {
        let err = apply(
            &testdisk::populated(),
            &["HELLO.TXT/SUB".into()],
            true,
            None,
        )
        .unwrap_err();
        assert!(err.contains("is a file"), "{err}");
    }

    #[test]
    fn creates_multiple_directories() {
        let out = apply(
            &testdisk::blank(),
            &["ONE".into(), "TWO".into()],
            false,
            None,
        )
        .unwrap();
        let all = dirs(out);
        assert!(all.contains(&"ONE".to_string()) && all.contains(&"TWO".to_string()));
    }
}
