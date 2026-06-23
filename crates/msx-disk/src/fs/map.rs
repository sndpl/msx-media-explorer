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

/// Parsed BIOS Parameter Block fields needed for sector mapping.
struct Bpb {
    bytes_per_sector: usize,
    sectors_per_cluster: usize,
    reserved: usize,
    num_fats: usize,
    root_entries: usize,
    sectors_per_fat: usize,
}

impl Bpb {
    fn parse(buf: &[u8]) -> Option<Bpb> {
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

    fn fat_start(&self) -> usize {
        self.reserved
    }

    fn root_start(&self) -> usize {
        self.reserved + self.num_fats * self.sectors_per_fat
    }

    fn root_sectors(&self) -> usize {
        (self.root_entries * 32).div_ceil(SECTOR_SIZE)
    }

    fn data_start(&self) -> usize {
        self.root_start() + self.root_sectors()
    }

    /// First sector index of cluster `c` (c >= 2).
    fn cluster_first_sector(&self, c: usize) -> usize {
        self.data_start() + (c - 2) * self.sectors_per_cluster
    }
}

/// Read FAT12 entry for `cluster`.
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

/// A boot-sector-repaired copy of `data`, so the BPB is always valid.
fn repaired(data: &[u8]) -> Option<Vec<u8>> {
    let mut buf = data.to_vec();
    super::boot::repair_boot_sector(&mut buf).ok()?;
    Some(buf)
}

/// Classify every sector of the disk.
pub fn disk_usage(data: &[u8]) -> Option<DiskMap> {
    let buf = repaired(data)?;
    let bpb = Bpb::parse(&buf)?;
    let sector_count = buf.len() / SECTOR_SIZE;
    let fat_start_byte = bpb.fat_start() * SECTOR_SIZE;
    let data_start = bpb.data_start();
    let total_clusters = if sector_count > data_start {
        (sector_count - data_start) / bpb.sectors_per_cluster
    } else {
        0
    };

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
            if cluster - 2 < total_clusters && fat12_entry(&buf, fat_start_byte, cluster) != 0 {
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
}

/// Parse BPB-derived filesystem geometry from a normalized disk buffer, or
/// `None` if it has no recognizable FAT BPB even after boot-sector repair.
pub fn fs_geometry(data: &[u8]) -> Option<FsGeometry> {
    let buf = repaired(data)?;
    let bpb = Bpb::parse(&buf)?;
    let total_sectors = buf.len() / SECTOR_SIZE;
    let data_start = bpb.data_start();
    let cluster_count = if total_sectors > data_start {
        (total_sectors - data_start) / bpb.sectors_per_cluster
    } else {
        0
    };
    Some(FsGeometry {
        bytes_per_sector: bpb.bytes_per_sector,
        sectors_per_cluster: bpb.sectors_per_cluster,
        total_sectors,
        cluster_count,
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
    let Some(first) = find_first_cluster(&buf, &bpb, path) else {
        return Vec::new();
    };
    cluster_chain_sectors(&buf, &bpb, first)
}

/// Walk a cluster chain and return all of its sectors.
fn cluster_chain_sectors(buf: &[u8], bpb: &Bpb, first: u16) -> Vec<usize> {
    let fat_start_byte = bpb.fat_start() * SECTOR_SIZE;
    let sector_count = buf.len() / SECTOR_SIZE;
    let max_clusters = sector_count / bpb.sectors_per_cluster + 2;
    let mut sectors = Vec::new();
    let mut cluster = first as usize;
    let mut guard = 0;
    while (2..0xFF8).contains(&cluster) && guard < max_clusters {
        let base = bpb.cluster_first_sector(cluster);
        for s in base..base + bpb.sectors_per_cluster {
            if s < sector_count {
                sectors.push(s);
            }
        }
        cluster = fat12_entry(buf, fat_start_byte, cluster) as usize;
        guard += 1;
    }
    sectors
}

/// Build the 8.3 name from a 32-byte directory entry.
fn entry_name(entry: &[u8]) -> String {
    let base: String = entry[..8]
        .iter()
        .take_while(|&&b| b != b' ')
        .map(|&b| b as char)
        .collect();
    let ext: String = entry[8..11]
        .iter()
        .take_while(|&&b| b != b' ')
        .map(|&b| b as char)
        .collect();
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

/// Resolve a slash-separated path to its first cluster.
fn find_first_cluster(buf: &[u8], bpb: &Bpb, path: &str) -> Option<u16> {
    let mut dir = DirLocation::Root;
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        let is_last = components.peek().is_none();
        let entry = find_entry(buf, bpb, &dir, component)?;
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

enum DirLocation {
    Root,
    Cluster(u16),
}

/// Find a directory entry by name within a directory.
fn find_entry(buf: &[u8], bpb: &Bpb, dir: &DirLocation, name: &str) -> Option<[u8; 32]> {
    let target = name.to_ascii_uppercase();
    let mut result = None;
    for_each_entry(buf, bpb, dir, |entry| {
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

/// Visit directory entries, stopping when `visit` returns true or the directory
/// ends. Skips deleted, volume-label, and long-file-name entries.
fn for_each_entry(buf: &[u8], bpb: &Bpb, dir: &DirLocation, mut visit: impl FnMut(&[u8]) -> bool) {
    let sectors: Vec<usize> = match dir {
        DirLocation::Root => (bpb.root_start()..bpb.data_start()).collect(),
        DirLocation::Cluster(first) => cluster_chain_sectors(buf, bpb, *first),
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
}
