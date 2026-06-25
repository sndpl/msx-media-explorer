//! Read-only, content-derived statistics for a mounted MSX FAT volume.
//!
//! Two layers: [`usage`] derives counts, the file-type breakdown, and the
//! largest files from an already-built directory tree; [`integrity`] walks the
//! FAT itself to find lost, cross-linked, and out-of-range clusters and to
//! measure fragmentation. The shared [`stats_from_parts`] core takes the
//! decomposed `(buf, &Bpb, FatType, ..)` so it serves both the floppy path
//! ([`disk_stats`]) and the read-only hard-disk [`Volume`](crate::fs::Volume)
//! path identically.

mod integrity;
mod usage;

use crate::fs::map::{self, Bpb, FatType, FsGeometry};
use crate::fs::DirEntry;
use crate::DiskFs;

/// A statistical summary of one FAT volume.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskStats {
    /// Total volume size in bytes (`total_sectors * bytes_per_sector`).
    pub total_bytes: u64,
    /// Bytes in allocated data clusters (FAT-derived, so it includes slack).
    pub used_bytes: u64,
    /// Bytes in free data clusters — the same figure as [`FsGeometry::free_bytes`].
    pub free_bytes: u64,
    /// Sum of every file's directory-recorded size (logical bytes, no slack).
    pub file_bytes: u64,
    pub file_count: usize,
    pub dir_count: usize,
    /// Per-extension breakdown, sorted by total bytes descending.
    pub extensions: Vec<ExtensionStat>,
    /// The largest files, by size descending (capped at the requested count).
    pub largest_files: Vec<FileSize>,
    /// Percentage of files whose cluster chain is non-contiguous (`0.0..=100.0`).
    pub fragmentation_pct: f64,
    /// FAT-chain integrity findings.
    pub integrity: FatIntegrity,
}

/// Count and total size of all files sharing one extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionStat {
    /// Lowercase extension without the dot, or empty for files with no extension.
    pub ext: String,
    /// Human description from the extension table, when the extension is known.
    pub description: Option<&'static str>,
    pub count: usize,
    pub total_bytes: u64,
}

/// One file's path and size, for the largest-files list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSize {
    pub path: String,
    pub size: u64,
}

/// FAT-chain integrity findings. An empty value means a clean filesystem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FatIntegrity {
    /// Clusters allocated in the FAT but not reachable from any directory entry.
    pub lost_clusters: Vec<u16>,
    /// Clusters referenced by more than one chain.
    pub cross_linked: Vec<CrossLink>,
    /// Chain pointers that fall outside the valid cluster range.
    pub bad_pointers: Vec<BadPointer>,
}

impl FatIntegrity {
    /// Whether no integrity problems were found.
    pub fn is_clean(&self) -> bool {
        self.lost_clusters.is_empty()
            && self.cross_linked.is_empty()
            && self.bad_pointers.is_empty()
    }
}

/// A cluster referenced by more than one chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossLink {
    pub cluster: u16,
    pub references: usize,
}

/// A next-pointer that points outside the volume's valid cluster range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadPointer {
    pub from_cluster: u16,
    pub points_to: u16,
}

/// Compute full statistics for a whole single-volume (floppy) image, or `None`
/// if it has no recognizable FAT BPB even after boot-sector repair.
pub fn disk_stats(data: &[u8], largest_n: usize) -> Option<DiskStats> {
    let (buf, bpb, fat_type) = map::parse_volume(data)?;
    let geo = map::fs_geometry(data)?;
    // The tree is read through `fatfs`; degrade to an empty tree (zero files)
    // for DOS1-garbage disks the FAT layer cannot mount.
    let tree = DiskFs::mount(data.to_vec())
        .and_then(|fs| fs.tree())
        .unwrap_or_default();
    Some(stats_from_parts(
        &buf, &bpb, fat_type, &geo, &tree, largest_n,
    ))
}

/// The shared core both the floppy and [`Volume`](crate::fs::Volume) paths call.
///
/// `buf` must start at the volume's boot sector (already boot-repaired). `tree`
/// is the volume's directory tree, used for names/sizes without re-walking the
/// FAT directory structure (the integrity pass does its own FAT walk).
pub(crate) fn stats_from_parts(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    geo: &FsGeometry,
    tree: &[DirEntry],
    largest_n: usize,
) -> DiskStats {
    let total_bytes = (geo.total_sectors * geo.bytes_per_sector) as u64;
    let cluster_bytes = (geo.sectors_per_cluster * geo.bytes_per_sector) as u64;
    let used_clusters = geo.cluster_count.saturating_sub(geo.free_clusters);
    let used_bytes = used_clusters as u64 * cluster_bytes;

    let usage = usage::from_tree(tree, largest_n);
    let (integrity, fragmentation_pct) = integrity::analyze(buf, bpb, fat_type, geo.cluster_count);

    DiskStats {
        total_bytes,
        used_bytes,
        free_bytes: geo.free_bytes(),
        file_bytes: usage.file_bytes,
        file_count: usage.file_count,
        dir_count: usage.dir_count,
        extensions: usage.extensions,
        largest_files: usage.largest_files,
        fragmentation_pct,
        integrity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::geometry::SIZE_720K;
    use std::io::{Cursor, Write};

    /// A 720KB FAT12 disk with BIG.BIN (5000 bytes) and SUB/HI.TXT (5 bytes) —
    /// the same shape used by `fs::map`'s tests.
    fn make_disk() -> Vec<u8> {
        let mut cursor = Cursor::new(vec![0u8; SIZE_720K]);
        fatfs::format_volume(
            &mut cursor,
            fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat12),
        )
        .unwrap();
        {
            let fs = fatfs::FileSystem::new(&mut cursor, fatfs::FsOptions::new()).unwrap();
            let root = fs.root_dir();
            let mut f = root.create_file("BIG.BIN").unwrap();
            f.write_all(&vec![0xAB; 5000]).unwrap();
            let sub = root.create_dir("SUB").unwrap();
            let mut g = sub.create_file("HI.TXT").unwrap();
            g.write_all(b"hello").unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn populated_disk_counts_files_and_dirs() {
        let stats = disk_stats(&make_disk(), 10).expect("stats");
        assert_eq!(stats.file_count, 2); // BIG.BIN + SUB/HI.TXT
        assert_eq!(stats.dir_count, 1); // SUB
        assert_eq!(stats.file_bytes, 5005); // 5000 + 5
    }

    #[test]
    fn extension_breakdown_is_sorted_and_described() {
        let stats = disk_stats(&make_disk(), 10).expect("stats");
        // bin (5000) sorts before txt (5).
        assert_eq!(stats.extensions[0].ext, "bin");
        assert_eq!(stats.extensions[0].count, 1);
        assert_eq!(stats.extensions[0].total_bytes, 5000);
        assert!(stats.extensions.iter().any(|e| e.ext == "txt"));
        // The extension table supplies a description for BIN files.
        assert!(stats.extensions[0].description.is_some());
    }

    #[test]
    fn largest_files_are_sorted_descending_and_capped() {
        let stats = disk_stats(&make_disk(), 1).expect("stats");
        assert_eq!(stats.largest_files.len(), 1);
        assert_eq!(stats.largest_files[0].size, 5000);
        assert!(stats.largest_files[0].path.ends_with("BIG.BIN"));
    }

    #[test]
    fn used_plus_free_equals_cluster_bytes_and_matches_geometry() {
        let disk = make_disk();
        let stats = disk_stats(&disk, 10).expect("stats");
        let geo = map::fs_geometry(&disk).expect("geo");
        assert_eq!(stats.free_bytes, geo.free_bytes());
        // Used + free is exactly the data-cluster region.
        let cluster_bytes =
            (geo.cluster_count * geo.sectors_per_cluster * geo.bytes_per_sector) as u64;
        assert_eq!(stats.used_bytes + stats.free_bytes, cluster_bytes);
    }

    #[test]
    fn populated_disk_is_clean_and_unfragmented() {
        let stats = disk_stats(&make_disk(), 10).expect("stats");
        assert!(stats.integrity.is_clean());
        // A freshly written disk lays files out contiguously.
        assert_eq!(stats.fragmentation_pct, 0.0);
    }

    #[test]
    fn empty_disk_reports_no_files_all_free() {
        let stats = disk_stats(&vec![0u8; SIZE_720K], 10).expect("stats");
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.dir_count, 0);
        assert_eq!(stats.used_bytes, 0);
        assert!(stats.extensions.is_empty());
        assert!(stats.largest_files.is_empty());
        assert_eq!(stats.fragmentation_pct, 0.0);
        assert!(stats.integrity.is_clean());
        // 713 clusters * 1024 bytes (matches fs::map's geometry tests).
        assert_eq!(stats.free_bytes, 713 * 1024);
    }
}
