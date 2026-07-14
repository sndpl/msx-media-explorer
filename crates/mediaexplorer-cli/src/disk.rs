//! Shared helpers: path/charset handling, tree lookup, and safe file I/O.

use std::io;
use std::path::Path;

use msx_disk::charset::{self, MsxCharset};
use msx_disk::fs::{DirEntry, Timestamp};

/// Errors surfaced to the user; commands return these as plain messages.
pub type CmdResult<T> = Result<T, String>;

/// Convert a user-typed image path into the PUA-encoded fatfs key form the
/// filesystem stores. ASCII letters are uppercased (FAT 8.3 names are stored
/// uppercase; this also keeps `fatfs` from adding VFAT long-name entries that
/// MSX-DOS would show as garbage). `/` separators pass through unchanged.
pub fn to_fs_key(charset: MsxCharset, path: &str) -> CmdResult<String> {
    let upper = path.to_ascii_uppercase();
    charset::encode_fs_name(charset, &upper)
        .ok_or_else(|| format!("'{path}' contains characters not representable in this charset"))
}

/// Find an entry by its slash-separated key, matching ASCII case-insensitively
/// (FAT names are stored uppercase; users type either).
pub fn find_entry<'a>(tree: &'a [DirEntry], key: &str) -> Option<&'a DirEntry> {
    tree.iter()
        .flat_map(|e| e.walk())
        .find(|e| e.path.eq_ignore_ascii_case(key))
}

/// Deletion order for an entry: children deepest-first, the entry itself last,
/// so every directory is empty by the time it is removed.
pub fn removal_paths(entry: &DirEntry) -> Vec<String> {
    let mut paths = Vec::new();
    for child in &entry.children {
        paths.extend(removal_paths(child));
    }
    paths.push(entry.path.clone());
    paths
}

/// Auto-detect the display charset from the raw filename bytes of every entry,
/// mirroring the GUI's per-disk behaviour (kana disks come out Japanese).
pub fn detect_charset(tree: &[DirEntry]) -> MsxCharset {
    let bytes: Vec<u8> = tree
        .iter()
        .flat_map(|e| e.walk())
        .flat_map(|e| e.name.chars().filter_map(charset::fs_name_byte))
        .collect();
    charset::detect(&bytes)
}

/// The charset to use: an explicit choice, or auto-detection from the tree.
pub fn charset_or_detect(explicit: Option<MsxCharset>, tree: &[DirEntry]) -> MsxCharset {
    explicit.unwrap_or_else(|| detect_charset(tree))
}

/// Write an extracted file to `target`, then restore its original directory-
/// entry modification time so it keeps its disk date instead of "now". Applying
/// the timestamp is best-effort: a failure to set it (e.g. a filesystem that
/// does not support it) must not fail the extraction, so it is ignored.
pub fn write_extracted(target: &Path, bytes: &[u8], modified: Option<Timestamp>) -> io::Result<()> {
    std::fs::write(target, bytes)?;
    if let Some(time) = modified.and_then(|t| t.to_system_time()) {
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .open(target)
            .and_then(|f| f.set_modified(time));
    }
    Ok(())
}

/// Write `bytes` to `path` atomically: write a sibling temp file, then rename
/// it over the target. Disk images are often irreplaceable originals, so a
/// crash mid-write must never leave a half-written image behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    let tmp = match dir {
        Some(d) => d.join(&tmp_name),
        None => tmp_name.into(),
    };
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use msx_disk::charset::byte_to_pua;
    use msx_disk::fs::{write, DiskFs};
    use msx_disk::image::geometry::DiskFormat;

    fn tree_of(data: &[u8]) -> Vec<DirEntry> {
        DiskFs::mount(data.to_vec()).unwrap().tree().unwrap()
    }

    #[test]
    fn to_fs_key_uppercases_ascii_and_keeps_separators() {
        let key = to_fs_key(MsxCharset::International, "utils/game.com").unwrap();
        assert_eq!(key, "UTILS/GAME.COM");
    }

    #[test]
    fn to_fs_key_encodes_kana_to_pua() {
        // Half-width katakana ｱ is byte 0xB1 under the Japanese charset.
        let key = to_fs_key(MsxCharset::Japanese, "\u{FF71}.BAS").unwrap();
        assert_eq!(key, format!("{}.BAS", byte_to_pua(0xB1)));
    }

    #[test]
    fn to_fs_key_rejects_unrepresentable_characters() {
        let err = to_fs_key(MsxCharset::International, "\u{FF71}.BAS").unwrap_err();
        assert!(err.contains("not representable"), "{err}");
    }

    #[test]
    fn find_entry_is_case_insensitive_and_reaches_nested_paths() {
        let disk = write::create_blank(DiskFormat::Ds720).unwrap();
        let disk = write::create_dir(&disk, "UTILS").unwrap();
        let disk = write::add_files(&disk, &[("UTILS/GAME.COM", b"x")]).unwrap();
        let tree = tree_of(&disk);
        let hit = find_entry(&tree, "utils/game.com").expect("found");
        assert_eq!(hit.path, "UTILS/GAME.COM");
        assert!(find_entry(&tree, "UTILS/MISSING.COM").is_none());
    }

    #[test]
    fn removal_paths_lists_children_before_their_directory() {
        let disk = write::create_blank(DiskFormat::Ds720).unwrap();
        let disk = write::create_dir(&disk, "TOOLS").unwrap();
        let disk = write::create_dir(&disk, "TOOLS/SUB").unwrap();
        let disk =
            write::add_files(&disk, &[("TOOLS/A.TXT", b"a"), ("TOOLS/SUB/B.TXT", b"b")]).unwrap();
        let tree = tree_of(&disk);
        let tools = find_entry(&tree, "TOOLS").unwrap();

        let paths = removal_paths(tools);
        assert_eq!(
            paths.last().unwrap(),
            "TOOLS",
            "directory itself comes last"
        );
        let idx = |p: &str| paths.iter().position(|x| x == p).unwrap();
        assert!(idx("TOOLS/SUB/B.TXT") < idx("TOOLS/SUB"));
        assert!(idx("TOOLS/A.TXT") < idx("TOOLS"));

        // And the order actually deletes cleanly.
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let out = write::delete(&disk, &refs).unwrap();
        assert!(tree_of(&out).is_empty());
    }

    #[test]
    fn detect_charset_reports_japanese_for_kana_names() {
        let disk = write::create_blank(DiskFormat::Ds720).unwrap();
        let kana: String = [0xB1u8, 0xB2, 0xB3]
            .iter()
            .map(|&b| byte_to_pua(b))
            .collect();
        let disk = write::add_files(&disk, &[(&format!("{kana}.BAS"), b"x".as_ref())]).unwrap();
        assert_eq!(detect_charset(&tree_of(&disk)), MsxCharset::Japanese);

        let ascii = write::create_blank(DiskFormat::Ds720).unwrap();
        let ascii = write::add_files(&ascii, &[("PLAIN.TXT", b"x")]).unwrap();
        assert_eq!(detect_charset(&tree_of(&ascii)), MsxCharset::International);
    }

    #[test]
    fn write_extracted_preserves_the_fat_modification_time() {
        use msx_disk::fs::Timestamp;
        let dir = std::env::temp_dir().join(format!("mecli-mtime-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("OLD.TXT");

        let modified = Timestamp {
            year: 1990,
            month: 5,
            day: 12,
            hour: 14,
            minute: 30,
        };
        write_extracted(&target, b"vintage", Some(modified)).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"vintage");
        let got = std::fs::metadata(&target).unwrap().modified().unwrap();
        assert_eq!(got, modified.to_system_time().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_atomic_replaces_target_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("mecli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("image.dsk");
        std::fs::write(&target, b"old").unwrap();

        write_atomic(&target, b"new contents").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new contents");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file must not remain");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
