//! Sector-usage mapping: classify every sector of a disk and resolve a file's
//! cluster chain to the sectors it occupies.
//!
//! This parses the BPB and FAT12 directly (information the `fatfs` crate does
//! not expose) on a boot-sector-repaired copy of the normalized buffer, so it
//! works on MSX-DOS 1 disks with no valid on-disk BPB too.

use crate::image::geometry::SECTOR_SIZE;

/// What a sector is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorKind {
    /// Boot sector / reserved area.
    Reserved,
    /// File Allocation Table.
    Fat,
    /// Root directory region.
    RootDir,
    /// Data sector belonging to an allocated cluster.
    DataUsed,
    /// Data sector in a free cluster.
    DataFree,
}

/// A classification of every sector on the disk.
#[derive(Debug, Clone)]
pub struct DiskMap {
    pub sector_count: usize,
    pub kinds: Vec<SectorKind>,
}

/// Which FAT width a volume uses.
///
/// MSX volumes can be either FAT12 (floppies, small hard-disk partitions) or
/// FAT16 (larger partitions). The two encode FAT entries differently — 12 vs
/// 16 bits — so the width must be known before walking any cluster chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat12,
    Fat16,
}

/// Parsed BIOS Parameter Block fields needed for sector mapping.
///
/// Every field is read straight from the volume's own boot sector; nothing is
/// assumed about layout (e.g. `root_entries` is whatever the disk declares).
pub struct Bpb {
    pub bytes_per_sector: usize,
    pub sectors_per_cluster: usize,
    pub reserved: usize,
    pub num_fats: usize,
    pub root_entries: usize,
    pub sectors_per_fat: usize,
}

impl Bpb {
    /// Parse the BPB from the boot sector at the start of `buf`, or `None` if
    /// the fields are not a self-consistent FAT BPB.
    pub fn parse(buf: &[u8]) -> Option<Bpb> {
        if buf.len() < SECTOR_SIZE {
            return None;
        }
        let bytes_per_sector = u16::from_le_bytes([buf[11], buf[12]]) as usize;
        let sectors_per_cluster = buf[13] as usize;
        let reserved = u16::from_le_bytes([buf[14], buf[15]]) as usize;
        let num_fats = buf[16] as usize;
        let root_entries = u16::from_le_bytes([buf[17], buf[18]]) as usize;
        let sectors_per_fat = u16::from_le_bytes([buf[22], buf[23]]) as usize;
        if bytes_per_sector != SECTOR_SIZE
            || sectors_per_cluster == 0
            || num_fats == 0
            || sectors_per_fat == 0
        {
            return None;
        }
        Some(Bpb {
            bytes_per_sector,
            sectors_per_cluster,
            reserved,
            num_fats,
            root_entries,
            sectors_per_fat,
        })
    }

    pub fn fat_start(&self) -> usize {
        self.reserved
    }

    pub fn root_start(&self) -> usize {
        self.reserved + self.num_fats * self.sectors_per_fat
    }

    pub fn root_sectors(&self) -> usize {
        (self.root_entries * 32).div_ceil(SECTOR_SIZE)
    }

    pub fn data_start(&self) -> usize {
        self.root_start() + self.root_sectors()
    }

    /// First sector index of cluster `c` (c >= 2).
    pub fn cluster_first_sector(&self, c: usize) -> usize {
        self.data_start() + (c - 2) * self.sectors_per_cluster
    }

    /// Number of data clusters in a `total_sectors`-sector volume.
    pub fn cluster_count(&self, total_sectors: usize) -> usize {
        let data_start = self.data_start();
        if total_sectors > data_start {
            (total_sectors - data_start) / self.sectors_per_cluster
        } else {
            0
        }
    }

    /// Decide the FAT width for a volume of `total_sectors` sectors.
    ///
    /// `fatfs` 0.3.6 picks the type from the cluster count alone (`< 4085 ->
    /// FAT12`), which mis-detects MSX FAT12 partitions that have just over 4085
    /// clusters but a `sectors_per_fat` only large enough for 12-bit entries.
    /// The extra `(clusters + 2) * 2 > sectors_per_fat * 512` term catches
    /// exactly that: if a 16-bit table would not fit in the declared FAT
    /// region, the volume must be FAT12.
    pub fn fat_type(&self, total_sectors: usize) -> FatType {
        let clusters = self.cluster_count(total_sectors);
        if clusters < 4085 || (clusters + 2) * 2 > self.sectors_per_fat * SECTOR_SIZE {
            FatType::Fat12
        } else {
            FatType::Fat16
        }
    }
}

/// Read the FAT12 entry for `cluster`.
fn fat12_entry(buf: &[u8], fat_start_byte: usize, cluster: usize) -> u16 {
    let off = fat_start_byte + cluster * 3 / 2;
    if off + 1 >= buf.len() {
        return 0xFFF;
    }
    let pair = buf[off] as u16 | (buf[off + 1] as u16) << 8;
    if cluster & 1 == 0 {
        pair & 0x0FFF
    } else {
        pair >> 4
    }
}

/// Read the FAT16 entry for `cluster` (little-endian 16-bit word).
fn fat16_entry(buf: &[u8], fat_start_byte: usize, cluster: usize) -> u16 {
    let off = fat_start_byte + cluster * 2;
    match buf.get(off..off + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]),
        None => 0xFFFF,
    }
}

/// Read the FAT entry for `cluster`, dispatching on the volume's FAT width.
pub(crate) fn fat_entry(
    buf: &[u8],
    fat_start_byte: usize,
    fat_type: FatType,
    cluster: usize,
) -> u16 {
    match fat_type {
        FatType::Fat12 => fat12_entry(buf, fat_start_byte, cluster),
        FatType::Fat16 => fat16_entry(buf, fat_start_byte, cluster),
    }
}

/// Whether `entry` is a continuation cluster (points at more data) for the
/// given FAT width. Bad-cluster and end-of-chain markers terminate the chain
/// (FAT12 bad `0xFF7`, EOC `0xFF8..`; FAT16 bad `0xFFF7`, EOC `0xFFF8..`).
fn is_data_cluster(fat_type: FatType, entry: u16) -> bool {
    match fat_type {
        FatType::Fat12 => (0x002..=0xFEF).contains(&entry),
        FatType::Fat16 => (0x0002..=0xFFEF).contains(&entry),
    }
}

/// The meaning of a FAT next-pointer when walking a cluster chain. Used by the
/// integrity analysis ([`crate::stats`]) to distinguish a normal continuation
/// from a structurally invalid pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pointer {
    /// A continuation cluster within range: the chain advances here.
    Next(u16),
    /// An end-of-chain marker (`0xFF8..` / `0xFFF8..`): the current cluster is
    /// the last one of the chain.
    End,
    /// A bad-cluster marker (`0xFF7` / `0xFFF7`).
    Bad,
    /// A reserved value (`0xFF0..=0xFF6` / `0xFFF0..=0xFFF6`): not free, not a
    /// data cluster, and not a terminator — intentional, but not chain data.
    Reserved,
    /// A free (unallocated) entry — value `0`.
    Free,
    /// A non-free value that is neither a recognized terminator nor a cluster
    /// within `2..2 + cluster_count`: structurally invalid for this volume.
    OutOfRange(u16),
}

/// Classify a raw FAT `entry` value for a volume with `cluster_count` data
/// clusters, reusing the same FAT12/FAT16 thresholds as chain-walking.
pub(crate) fn classify_pointer(fat_type: FatType, entry: u16, cluster_count: usize) -> Pointer {
    if entry == 0 {
        return Pointer::Free;
    }
    if is_data_cluster(fat_type, entry) {
        if (entry as usize) < 2 + cluster_count {
            Pointer::Next(entry)
        } else {
            Pointer::OutOfRange(entry)
        }
    } else {
        // Non-data, non-free: bad marker, reserved range, or end-of-chain.
        let (bad, reserved_lo) = match fat_type {
            FatType::Fat12 => (0xFF7, 0xFF0),
            FatType::Fat16 => (0xFFF7, 0xFFF0),
        };
        if entry == bad {
            Pointer::Bad
        } else if entry >= reserved_lo && entry < bad {
            Pointer::Reserved
        } else {
            Pointer::End
        }
    }
}

/// A boot-sector-repaired copy of `data`, so the BPB is always valid.
fn repaired(data: &[u8]) -> Option<Vec<u8>> {
    let mut buf = data.to_vec();
    super::boot::repair_boot_sector(&mut buf).ok()?;
    Some(buf)
}

/// Boot-repair `data` and parse its BPB and FAT width in one step, or `None` if
/// it has no recognizable FAT BPB even after repair. The returned buffer starts
/// at the volume's boot sector, so every `map` primitive applies to it.
pub(crate) fn parse_volume(data: &[u8]) -> Option<(Vec<u8>, Bpb, FatType)> {
    let buf = repaired(data)?;
    let bpb = Bpb::parse(&buf)?;
    let fat_type = bpb.fat_type(buf.len() / SECTOR_SIZE);
    Some((buf, bpb, fat_type))
}

/// Classify every sector of the disk.
pub fn disk_usage(data: &[u8]) -> Option<DiskMap> {
    let buf = repaired(data)?;
    let bpb = Bpb::parse(&buf)?;
    let sector_count = buf.len() / SECTOR_SIZE;
    let fat_type = bpb.fat_type(sector_count);
    let fat_start_byte = bpb.fat_start() * SECTOR_SIZE;
    let data_start = bpb.data_start();
    let total_clusters = bpb.cluster_count(sector_count);

    let mut kinds = Vec::with_capacity(sector_count);
    for sector in 0..sector_count {
        let kind = if sector < bpb.fat_start() {
            SectorKind::Reserved
        } else if sector < bpb.root_start() {
            SectorKind::Fat
        } else if sector < data_start {
            SectorKind::RootDir
        } else {
            let cluster = 2 + (sector - data_start) / bpb.sectors_per_cluster;
            if cluster - 2 < total_clusters
                && fat_entry(&buf, fat_start_byte, fat_type, cluster) != 0
            {
                SectorKind::DataUsed
            } else {
                SectorKind::DataFree
            }
        };
        kinds.push(kind);
    }
    Some(DiskMap {
        sector_count,
        kinds,
    })
}

/// Filesystem geometry derived from the (repaired) BPB: the figures shown in
/// the status bar. Distinct from physical [`crate::image::geometry::Geometry`],
/// which describes sides/tracks rather than the FAT layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsGeometry {
    pub bytes_per_sector: usize,
    pub sectors_per_cluster: usize,
    pub total_sectors: usize,
    /// Number of data clusters (excluding the two reserved FAT entries).
    pub cluster_count: usize,
    /// Data clusters whose FAT entry is 0 (unallocated, i.e. free space).
    pub free_clusters: usize,
}

impl FsGeometry {
    /// Unallocated space in bytes (free data clusters times the cluster size).
    pub fn free_bytes(&self) -> u64 {
        (self.free_clusters * self.sectors_per_cluster * self.bytes_per_sector) as u64
    }
}

/// Count the free data clusters (FAT entries equal to 0) for a volume with
/// `cluster_count` data clusters. `buf` must start at the volume's boot sector.
pub(crate) fn count_free_clusters(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    cluster_count: usize,
) -> usize {
    let fat_start_byte = bpb.fat_start() * SECTOR_SIZE;
    (2..2 + cluster_count)
        .filter(|&c| fat_entry(buf, fat_start_byte, fat_type, c) == 0)
        .count()
}

/// Parse BPB-derived filesystem geometry from a normalized disk buffer, or
/// `None` if it has no recognizable FAT BPB even after boot-sector repair.
pub fn fs_geometry(data: &[u8]) -> Option<FsGeometry> {
    let buf = repaired(data)?;
    let bpb = Bpb::parse(&buf)?;
    let total_sectors = buf.len() / SECTOR_SIZE;
    let cluster_count = bpb.cluster_count(total_sectors);
    let fat_type = bpb.fat_type(total_sectors);
    let free_clusters = count_free_clusters(&buf, &bpb, fat_type, cluster_count);
    Some(FsGeometry {
        bytes_per_sector: bpb.bytes_per_sector,
        sectors_per_cluster: bpb.sectors_per_cluster,
        total_sectors,
        cluster_count,
        free_clusters,
    })
}

/// The sectors occupied by the file at `path` (slash-separated), in order.
pub fn file_sectors(data: &[u8], path: &str) -> Vec<usize> {
    let Some(buf) = repaired(data) else {
        return Vec::new();
    };
    let Some(bpb) = Bpb::parse(&buf) else {
        return Vec::new();
    };
    let fat_type = bpb.fat_type(buf.len() / SECTOR_SIZE);
    let Some(first) = find_first_cluster(&buf, &bpb, fat_type, path) else {
        return Vec::new();
    };
    cluster_chain_sectors(&buf, &bpb, fat_type, first)
}

/// Walk a cluster chain and return the cluster numbers it visits, in order.
pub(crate) fn cluster_chain_clusters(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    first: u16,
) -> Vec<u16> {
    let fat_start_byte = bpb.fat_start() * SECTOR_SIZE;
    let sector_count = buf.len() / SECTOR_SIZE;
    let max_clusters = sector_count / bpb.sectors_per_cluster + 2;
    let mut clusters = Vec::new();
    let mut cluster = first as usize;
    let mut guard = 0;
    while is_data_cluster(fat_type, cluster as u16) && guard < max_clusters {
        clusters.push(cluster as u16);
        cluster = fat_entry(buf, fat_start_byte, fat_type, cluster) as usize;
        guard += 1;
    }
    clusters
}

/// Walk a cluster chain and return all of its sectors.
pub(crate) fn cluster_chain_sectors(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    first: u16,
) -> Vec<usize> {
    let sector_count = buf.len() / SECTOR_SIZE;
    let clusters = cluster_chain_clusters(buf, bpb, fat_type, first);
    let mut sectors = Vec::with_capacity(clusters.len() * bpb.sectors_per_cluster);
    for cluster in clusters {
        let base = bpb.cluster_first_sector(cluster as usize);
        for s in base..base + bpb.sectors_per_cluster {
            if s < sector_count {
                sectors.push(s);
            }
        }
    }
    sectors
}

/// Build the 8.3 name from a 32-byte directory entry.
///
/// High bytes are PUA-encoded via [`crate::charset::byte_to_pua`] so the names
/// (and the paths built from them) match those from the fatfs floppy path and
/// decode through [`crate::fs::DirEntry::display_name`] the same way.
pub(crate) fn entry_name(entry: &[u8]) -> String {
    let base: String = entry[..8]
        .iter()
        .take_while(|&&b| b != b' ')
        .map(|&b| crate::charset::byte_to_pua(b))
        .collect();
    let ext: String = entry[8..11]
        .iter()
        .take_while(|&&b| b != b' ')
        .map(|&b| crate::charset::byte_to_pua(b))
        .collect();
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

/// Resolve a slash-separated path to its first cluster.
pub(crate) fn find_first_cluster(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    path: &str,
) -> Option<u16> {
    let mut dir = DirLocation::Root;
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        let is_last = components.peek().is_none();
        let entry = find_entry(buf, bpb, fat_type, &dir, component)?;
        let first = u16::from_le_bytes([entry[26], entry[27]]);
        let is_dir = entry[11] & 0x10 != 0;
        if is_last {
            return Some(first);
        }
        if !is_dir {
            return None;
        }
        dir = DirLocation::Cluster(first);
    }
    None
}

pub(crate) enum DirLocation {
    Root,
    Cluster(u16),
}

/// Find a directory entry by name within a directory.
fn find_entry(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    dir: &DirLocation,
    name: &str,
) -> Option<[u8; 32]> {
    let target = name.to_ascii_uppercase();
    let mut result = None;
    for_each_entry(buf, bpb, fat_type, dir, |entry| {
        if entry_name(entry).eq_ignore_ascii_case(&target) {
            let mut owned = [0u8; 32];
            owned.copy_from_slice(entry);
            result = Some(owned);
            true
        } else {
            false
        }
    });
    result
}

/// Read the volume label from the root directory's VOLUME_ID entry, if present.
///
/// This scans the root region directly (rather than via [`for_each_entry`],
/// which deliberately skips volume-label entries). The label is trimmed of
/// trailing spaces; an empty label yields `None`.
pub(crate) fn root_volume_label(buf: &[u8], bpb: &Bpb) -> Option<String> {
    for sector in bpb.root_start()..bpb.data_start() {
        let base = sector * SECTOR_SIZE;
        for e in 0..(SECTOR_SIZE / 32) {
            let off = base + e * 32;
            let entry = buf.get(off..off + 32)?;
            if entry[0] == 0x00 {
                return None; // end of directory
            }
            // VOLUME_ID (0x08) set, LFN bits (0x0F) clear, not deleted.
            if entry[0] != 0xE5 && entry[11] & 0x0F != 0x0F && entry[11] & 0x08 != 0 {
                let raw: String = entry[..11].iter().map(|&b| b as char).collect();
                let cleaned = raw.trim_end().trim().to_string();
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
        }
    }
    None
}

/// Visit directory entries, stopping when `visit` returns true or the directory
/// ends. Skips deleted, volume-label, and long-file-name entries.
pub(crate) fn for_each_entry(
    buf: &[u8],
    bpb: &Bpb,
    fat_type: FatType,
    dir: &DirLocation,
    mut visit: impl FnMut(&[u8]) -> bool,
) {
    let sectors: Vec<usize> = match dir {
        DirLocation::Root => (bpb.root_start()..bpb.data_start()).collect(),
        DirLocation::Cluster(first) => cluster_chain_sectors(buf, bpb, fat_type, *first),
    };
    for sector in sectors {
        let base = sector * SECTOR_SIZE;
        for e in 0..(SECTOR_SIZE / 32) {
            let off = base + e * 32;
            let Some(entry) = buf.get(off..off + 32) else {
                return;
            };
            if entry[0] == 0x00 {
                return; // end of directory
            }
            if entry[0] == 0xE5 || entry[11] & 0x0F == 0x0F || entry[11] & 0x08 != 0 {
                continue; // deleted / LFN / volume label
            }
            if visit(entry) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::DiskFs;
    use crate::image::geometry::SIZE_720K;
    use std::io::{Cursor, Write};

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
    fn entry_name_pua_encodes_high_bytes() {
        // A raw 8.3 entry as a real MSX disk stores it: katakana bytes B1 B2 B3,
        // ext "BAS", no LFN. entry_name must PUA-encode the high bytes so the
        // path matches the fatfs floppy path and decodes to kana for display.
        let mut entry = [b' '; 32];
        entry[..3].copy_from_slice(&[0xB1, 0xB2, 0xB3]);
        entry[8..11].copy_from_slice(b"BAS");
        assert_eq!(entry_name(&entry), "\u{F0B1}\u{F0B2}\u{F0B3}.BAS");
    }

    #[test]
    fn classifies_sectors() {
        let disk = make_disk();
        let map = disk_usage(&disk).unwrap();
        assert_eq!(map.sector_count, 1440);
        assert_eq!(map.kinds[0], SectorKind::Reserved);
        assert!(map.kinds.contains(&SectorKind::Fat));
        assert!(map.kinds.contains(&SectorKind::RootDir));
        assert!(map.kinds.contains(&SectorKind::DataUsed));
        assert!(map.kinds.contains(&SectorKind::DataFree));
    }

    #[test]
    fn file_sectors_match_size() {
        let disk = make_disk();
        // 5000 bytes, 2 sectors/cluster (1024 B) -> ceil(5000/1024)=5 clusters = 10 sectors.
        let sectors = file_sectors(&disk, "BIG.BIN");
        assert_eq!(sectors.len(), 10);
        // All classified as used data.
        let map = disk_usage(&disk).unwrap();
        for s in &sectors {
            assert_eq!(map.kinds[*s], SectorKind::DataUsed);
        }
    }

    #[test]
    fn file_sectors_in_subdirectory() {
        let disk = make_disk();
        let sectors = file_sectors(&disk, "SUB/HI.TXT");
        assert!(!sectors.is_empty());
        // Sanity: reading the file via the fs yields the same content length region.
        let fs = DiskFs::mount(disk).unwrap();
        assert_eq!(fs.read_file("SUB/HI.TXT").unwrap(), b"hello");
    }

    #[test]
    fn missing_file_has_no_sectors() {
        assert!(file_sectors(&make_disk(), "NOPE.XXX").is_empty());
    }

    #[test]
    fn fs_geometry_uses_canonical_msx_layout_when_no_bpb() {
        // A zeroed 720KB image has no valid BPB, so boot repair synthesizes the
        // canonical MSX layout (2 sectors/cluster) that real MSX-DOS 1 disks use.
        let geo = fs_geometry(&vec![0u8; SIZE_720K]).expect("geometry");
        assert_eq!(geo.bytes_per_sector, 512);
        assert_eq!(geo.sectors_per_cluster, 2);
        assert_eq!(geo.total_sectors, 1440);
        // 1440 sectors - 14 system (1 boot + 2*3 FAT + 7 root) = 1426 data
        // sectors / 2 per cluster = 713 clusters.
        assert_eq!(geo.cluster_count, 713);
    }

    #[test]
    fn fs_geometry_reads_actual_bpb_values() {
        // A real (here fatfs-formatted) disk reports its own BPB faithfully.
        let geo = fs_geometry(&make_disk()).expect("geometry");
        assert_eq!(geo.bytes_per_sector, 512);
        assert_eq!(geo.total_sectors, 1440);
        assert!(geo.sectors_per_cluster.is_power_of_two());
        assert!(geo.cluster_count > 0);
    }

    #[test]
    fn fs_geometry_reports_all_clusters_free_when_empty() {
        // A zeroed 720KB image has an all-zero FAT after boot repair, so every
        // data cluster is free.
        let geo = fs_geometry(&vec![0u8; SIZE_720K]).expect("geometry");
        assert_eq!(geo.free_clusters, geo.cluster_count);
        assert_eq!(geo.free_clusters, 713);
        // 713 clusters * 2 sectors * 512 bytes = 730112 bytes free.
        assert_eq!(geo.free_bytes(), 713 * 1024);
    }

    #[test]
    fn fs_geometry_counts_used_clusters_on_populated_disk() {
        let disk = make_disk();
        let geo = fs_geometry(&disk).expect("geometry");
        // BIG.BIN/SUB/HI.TXT occupy some clusters, so not all are free.
        assert!(geo.free_clusters > 0);
        assert!(geo.free_clusters < geo.cluster_count);
        // Free + used clusters (derived from the sector map) == total clusters.
        let map = disk_usage(&disk).unwrap();
        let used_sectors = map
            .kinds
            .iter()
            .filter(|&&k| k == SectorKind::DataUsed)
            .count();
        let used_clusters = used_sectors / geo.sectors_per_cluster;
        assert_eq!(geo.free_clusters + used_clusters, geo.cluster_count);
        // free_bytes derives from free clusters and the cluster size.
        assert_eq!(
            geo.free_bytes(),
            (geo.free_clusters * geo.sectors_per_cluster * geo.bytes_per_sector) as u64
        );
    }

    /// Build a `Bpb` directly from field values for fat-type tests.
    fn bpb(
        sectors_per_cluster: usize,
        reserved: usize,
        num_fats: usize,
        root_entries: usize,
        sectors_per_fat: usize,
    ) -> Bpb {
        Bpb {
            bytes_per_sector: SECTOR_SIZE,
            sectors_per_cluster,
            reserved,
            num_fats,
            root_entries,
            sectors_per_fat,
        }
    }

    #[test]
    fn fat_type_is_fat12_when_a_fat16_table_would_not_fit() {
        // The exact hd.dsk partition shape: spc=16, res=1, 2 FATs, root=256,
        // spf=12, total=65535 sectors. data_start = 1 + 24 + 16 = 41; clusters
        // = (65535-41)/16 = 4093 (>= 4085, so fatfs would call this FAT16), but
        // a 16-bit table needs (4093+2)*2 = 8190 B > 12*512 = 6144 B, so it must
        // be FAT12.
        let b = bpb(16, 1, 2, 256, 12);
        assert_eq!(b.cluster_count(65535), 4093);
        assert_eq!(b.fat_type(65535), FatType::Fat12);
    }

    #[test]
    fn fat_type_is_fat16_when_clusters_exceed_4085_and_table_fits() {
        // spc=16, res=1, 2 FATs, root=512, spf=64. data_start = 1 + 128 + 32 =
        // 161. With 80000 sectors: clusters = (80000-161)/16 = 4989 (>= 4085),
        // and a 16-bit table needs (4989+2)*2 = 9982 B <= 64*512 = 32768 B.
        let b = bpb(16, 1, 2, 512, 64);
        assert_eq!(b.cluster_count(80000), 4989);
        assert_eq!(b.fat_type(80000), FatType::Fat16);
    }

    #[test]
    fn fat_type_is_fat12_below_4085_clusters() {
        // A standard 720KB floppy resolves to FAT12, as before.
        let geo = make_disk();
        let buf = repaired(&geo).unwrap();
        let parsed = Bpb::parse(&buf).unwrap();
        assert_eq!(parsed.fat_type(buf.len() / SECTOR_SIZE), FatType::Fat12);
    }

    #[test]
    fn fat16_entry_decodes_little_endian() {
        // Two 16-bit words: cluster 0 -> 0x1234, cluster 1 -> 0xFFFF.
        let buf = [0x34, 0x12, 0xFF, 0xFF];
        assert_eq!(fat16_entry(&buf, 0, 0), 0x1234);
        assert_eq!(fat16_entry(&buf, 0, 1), 0xFFFF);
        // Out-of-range reads return an end-of-chain marker, not a panic.
        assert_eq!(fat16_entry(&buf, 0, 99), 0xFFFF);
    }

    #[test]
    fn chain_terminators_for_fat12() {
        // Data clusters continue; bad (0xFF7) and EOC (0xFF8..) terminate.
        assert!(is_data_cluster(FatType::Fat12, 0x002));
        assert!(is_data_cluster(FatType::Fat12, 0xFEF));
        assert!(!is_data_cluster(FatType::Fat12, 0x000));
        assert!(!is_data_cluster(FatType::Fat12, 0x001));
        assert!(!is_data_cluster(FatType::Fat12, 0xFF7)); // bad cluster
        assert!(!is_data_cluster(FatType::Fat12, 0xFF8)); // end of chain
        assert!(!is_data_cluster(FatType::Fat12, 0xFFF));
    }

    #[test]
    fn chain_terminators_for_fat16() {
        assert!(is_data_cluster(FatType::Fat16, 0x0002));
        assert!(is_data_cluster(FatType::Fat16, 0xFFEF));
        assert!(!is_data_cluster(FatType::Fat16, 0x0000));
        assert!(!is_data_cluster(FatType::Fat16, 0x0001));
        assert!(!is_data_cluster(FatType::Fat16, 0xFFF7)); // bad cluster
        assert!(!is_data_cluster(FatType::Fat16, 0xFFF8)); // end of chain
        assert!(!is_data_cluster(FatType::Fat16, 0xFFFF));
    }
}
