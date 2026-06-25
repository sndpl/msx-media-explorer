//! Tree-derived usage stats: file/directory counts, the per-extension
//! breakdown, total logical bytes, and the largest files. All computed from an
//! already-built [`DirEntry`] tree, so it is identical for floppies and
//! hard-disk partitions.

use std::collections::HashMap;

use super::{ExtensionStat, FileSize};
use crate::fileinfo;
use crate::fs::DirEntry;

/// The subset of [`super::DiskStats`] derived purely from the directory tree.
pub(crate) struct TreeStats {
    pub file_count: usize,
    pub dir_count: usize,
    pub file_bytes: u64,
    pub extensions: Vec<ExtensionStat>,
    pub largest_files: Vec<FileSize>,
}

/// Aggregate counts, the extension breakdown, and the largest `largest_n` files
/// from `tree` (a forest of root entries, each carrying its descendants).
pub(crate) fn from_tree(tree: &[DirEntry], largest_n: usize) -> TreeStats {
    let mut file_count = 0;
    let mut dir_count = 0;
    let mut file_bytes = 0u64;
    let mut by_ext: HashMap<String, (usize, u64)> = HashMap::new();
    let mut files: Vec<FileSize> = Vec::new();

    for entry in tree.iter().flat_map(DirEntry::walk) {
        if entry.is_dir {
            dir_count += 1;
            continue;
        }
        file_count += 1;
        file_bytes += entry.size;
        let ext = fileinfo::extension(&entry.name).unwrap_or_default();
        let slot = by_ext.entry(ext).or_insert((0, 0));
        slot.0 += 1;
        slot.1 += entry.size;
        files.push(FileSize {
            path: entry.path.clone(),
            size: entry.size,
        });
    }

    let mut extensions: Vec<ExtensionStat> = by_ext
        .into_iter()
        .map(|(ext, (count, total_bytes))| ExtensionStat {
            description: fileinfo::extensions::describe_ext(&ext),
            ext,
            count,
            total_bytes,
        })
        .collect();
    // Largest first; ties broken by name/path for a stable, deterministic order.
    extensions.sort_by(|a, b| {
        b.total_bytes
            .cmp(&a.total_bytes)
            .then_with(|| a.ext.cmp(&b.ext))
    });
    files.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    files.truncate(largest_n);

    TreeStats {
        file_count,
        dir_count,
        file_bytes,
        extensions,
        largest_files: files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::entry::Attributes;

    fn file(path: &str, size: u64) -> DirEntry {
        let name = path.rsplit('/').next().unwrap().to_string();
        DirEntry {
            name,
            path: path.to_string(),
            is_dir: false,
            size,
            attributes: Attributes::default(),
            modified: None,
            children: Vec::new(),
        }
    }

    fn dir(path: &str, children: Vec<DirEntry>) -> DirEntry {
        let name = path.rsplit('/').next().unwrap().to_string();
        DirEntry {
            name,
            path: path.to_string(),
            is_dir: true,
            size: 0,
            attributes: Attributes::default(),
            modified: None,
            children,
        }
    }

    #[test]
    fn counts_files_and_dirs_recursively() {
        let tree = vec![
            file("A.TXT", 10),
            dir("SUB", vec![file("SUB/B.BIN", 20), file("SUB/C.BIN", 30)]),
        ];
        let s = from_tree(&tree, 10);
        assert_eq!(s.file_count, 3);
        assert_eq!(s.dir_count, 1);
        assert_eq!(s.file_bytes, 60);
    }

    #[test]
    fn extension_breakdown_aggregates_and_sorts() {
        let tree = vec![
            file("A.BIN", 20),
            file("B.BIN", 30),
            file("C.TXT", 100),
            file("NOEXT", 5),
        ];
        let s = from_tree(&tree, 10);
        // txt (100) > bin (50) > "" (5).
        assert_eq!(s.extensions[0].ext, "txt");
        assert_eq!(s.extensions[1].ext, "bin");
        assert_eq!(s.extensions[1].count, 2);
        assert_eq!(s.extensions[1].total_bytes, 50);
        assert_eq!(s.extensions[2].ext, "");
    }

    #[test]
    fn largest_files_capped_and_descending() {
        let tree = vec![file("A", 10), file("B", 99), file("C", 50)];
        let s = from_tree(&tree, 2);
        assert_eq!(s.largest_files.len(), 2);
        assert_eq!(s.largest_files[0].path, "B");
        assert_eq!(s.largest_files[1].path, "C");
    }
}
