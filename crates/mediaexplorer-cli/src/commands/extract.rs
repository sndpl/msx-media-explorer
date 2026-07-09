//! `extract` — plan which files to copy out of a disk image.

use msx_disk::charset::MsxCharset;
use msx_disk::fs::{DirEntry, DiskFs};

use crate::disk::{charset_or_detect, find_entry, to_fs_key, CmdResult};

/// One file to write on the host: a relative path (display-decoded, so kana
/// names come out as real kana) and its bytes.
pub type Extraction = (String, Vec<u8>);

/// Plan the extraction of `path` (a file, or a directory with `recursive`),
/// or of the whole image when `path` is `None`.
pub fn plan(
    data: &[u8],
    path: Option<&str>,
    recursive: bool,
    charset: Option<MsxCharset>,
) -> CmdResult<Vec<Extraction>> {
    let fs = DiskFs::mount(data.to_vec()).map_err(|e| e.to_string())?;
    let tree = fs.tree().map_err(|e| e.to_string())?;
    let charset = charset_or_detect(charset, &tree);

    let roots: Vec<&DirEntry> = match path {
        None => tree.iter().collect(),
        Some(p) => {
            let key = to_fs_key(charset, p)?;
            let entry = find_entry(&tree, &key).ok_or_else(|| format!("'{p}': not found"))?;
            if entry.is_dir && !recursive {
                return Err(format!("'{p}' is a directory; use -R to extract it"));
            }
            vec![entry]
        }
    };

    let mut out = Vec::new();
    for root in roots {
        collect(&fs, root, "", charset, &mut out)?;
    }
    Ok(out)
}

fn collect(
    fs: &DiskFs,
    entry: &DirEntry,
    prefix: &str,
    charset: MsxCharset,
    out: &mut Vec<Extraction>,
) -> CmdResult<()> {
    let name = entry.display_name(charset);
    let rel = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    if entry.is_dir {
        for child in &entry.children {
            collect(fs, child, &rel, charset, out)?;
        }
    } else {
        let bytes = fs.read_file(&entry.path).map_err(|e| e.to_string())?;
        out.push((rel, bytes));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;

    #[test]
    fn extracts_a_single_file_by_name() {
        let files = plan(&testdisk::populated(), Some("hello.txt"), false, None).unwrap();
        assert_eq!(files, vec![("HELLO.TXT".to_string(), b"hi there".to_vec())]);
    }

    #[test]
    fn directory_requires_recursive_flag() {
        let err = plan(&testdisk::populated(), Some("UTILS"), false, None).unwrap_err();
        assert!(err.contains("-R"), "{err}");

        let files = plan(&testdisk::populated(), Some("UTILS"), true, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "UTILS/GAME.COM");
    }

    #[test]
    fn whole_image_extraction_walks_everything() {
        let files = plan(&testdisk::populated(), None, false, None).unwrap();
        let mut names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["HELLO.TXT", "UTILS/GAME.COM"]);
    }

    #[test]
    fn missing_path_is_an_error() {
        let err = plan(&testdisk::populated(), Some("NOPE.TXT"), false, None).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn kana_names_are_decoded_for_the_host() {
        use msx_disk::charset::byte_to_pua;
        use msx_disk::fs::write;
        let kana_key: String = [0xB1u8, 0xB2].iter().map(|&b| byte_to_pua(b)).collect();
        let disk = write::add_files(
            &testdisk::blank(),
            &[(format!("{kana_key}.BAS").as_str(), b"10 END".as_ref())],
        )
        .unwrap();
        let files = plan(&disk, None, false, None).unwrap();
        assert_eq!(files[0].0, "\u{FF71}\u{FF72}.BAS");
    }
}
