//! Read-only MSX FAT volume for a single hard-disk partition.
//!
//! Unlike [`super::DiskFs`] (which mounts floppies via the `fatfs` crate), a
//! [`Volume`] reads the FAT directly using the width-aware primitives in
//! [`super::map`]. This is mandatory for MSX hard-disk partitions: `fatfs`
//! 0.3.6 would mis-detect their FAT width from the cluster count alone and
//! silently corrupt file and subdirectory reads. Everything here is read-only.

use super::entry::{Attributes, DirEntry, Timestamp};
use super::map::{self, Bpb, DirLocation, FatType};
use super::partition::PartitionEntry;
use crate::image::geometry::SECTOR_SIZE;
use crate::image::DiskImage;

/// Largest directory nesting depth followed when building the tree. Guards
/// against pathological or maliciously deep structures.
const MAX_DEPTH: usize = 64;

/// A mounted, read-only FAT12/FAT16 volume backed by one partition's bytes.
pub struct Volume {
    /// A boot-sector-repaired copy of the partition slice, so byte 0 is the
    /// volume's boot sector and [`map`] semantics apply unchanged.
    data: Vec<u8>,
    bpb: Bpb,
    fat_type: FatType,
    /// The partition's first sector (LBA) in the whole image, so volume-relative
    /// sector indices can be translated back to absolute ones.
    lba_offset: usize,
    label: Option<String>,
}

impl Volume {
    /// Mount the partition described by `entry` from `disk`'s normalized bytes,
    /// or `None` if its slice has no parseable FAT BPB.
    pub fn from_partition(disk: &DiskImage, entry: &PartitionEntry) -> Option<Volume> {
        Volume::from_image_slice(
            disk.data(),
            entry.lba_start as usize,
            entry.sector_count as usize,
        )
    }

    /// Mount a partition slice given its `lba_start` and declared `sector_count`
    /// within `image`. Shared by [`Volume::from_partition`] and tests.
    pub fn from_image_slice(image: &[u8], lba_start: usize, sector_count: usize) -> Option<Volume> {
        let start = lba_start.checked_mul(SECTOR_SIZE)?;
        if start >= image.len() {
            return None;
        }
        let declared = sector_count.saturating_mul(SECTOR_SIZE);
        let end = start.saturating_add(declared).min(image.len());
        let mut data = image[start..end].to_vec();
        // A partition boot sector is normally already a valid BPB (we only need
        // to add the PC signature for any downstream parser). The floppy boot
        // repair is a fallback for the rare partition whose BPB is DOS1 garbage,
        // but it rejects valid large FAT16 BPBs (its heuristic caps
        // sectors_per_fat at 15), so it is only invoked when no BPB parses.
        if Bpb::parse(&data).is_some() {
            if data.len() >= SECTOR_SIZE {
                data[510] = 0x55;
                data[511] = 0xAA;
            }
        } else {
            super::boot::repair_boot_sector(&mut data).ok()?;
        }
        let bpb = Bpb::parse(&data)?;
        let fat_type = bpb.fat_type(data.len() / SECTOR_SIZE);
        let label = map::root_volume_label(&data, &bpb);
        Some(Volume {
            data,
            bpb,
            fat_type,
            lba_offset: lba_start,
            label,
        })
    }

    /// The volume's FAT width.
    pub fn fat_type(&self) -> FatType {
        self.fat_type
    }

    /// The volume's label (from the root VOLUME_ID entry), if any.
    pub fn volume_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// The partition's first absolute sector (LBA) in the whole image.
    pub fn lba_offset(&self) -> usize {
        self.lba_offset
    }

    /// BPB-derived filesystem geometry for this volume.
    pub fn fs_geometry(&self) -> map::FsGeometry {
        let total_sectors = self.data.len() / SECTOR_SIZE;
        map::FsGeometry {
            bytes_per_sector: self.bpb.bytes_per_sector,
            sectors_per_cluster: self.bpb.sectors_per_cluster,
            total_sectors,
            cluster_count: self.bpb.cluster_count(total_sectors),
        }
    }

    /// The full recursive directory tree from the volume's root.
    pub fn tree(&self) -> Vec<DirEntry> {
        let mut visited = Vec::new();
        self.read_dir(&DirLocation::Root, "", 0, &mut visited)
    }

    /// Read a file's bytes by its slash-separated path, truncated to the size
    /// recorded in its directory entry. Returns `None` if the path does not
    /// resolve to a file.
    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let (first, size, is_dir) = self.resolve_entry(path)?;
        if is_dir {
            return None;
        }
        let sectors = map::cluster_chain_sectors(&self.data, &self.bpb, self.fat_type, first);
        let mut bytes = Vec::with_capacity(sectors.len() * SECTOR_SIZE);
        for sector in sectors {
            let off = sector * SECTOR_SIZE;
            if let Some(chunk) = self.data.get(off..off + SECTOR_SIZE) {
                bytes.extend_from_slice(chunk);
            }
        }
        bytes.truncate(size as usize);
        Some(bytes)
    }

    /// The absolute sectors (in the whole image) occupied by `path`'s data.
    pub fn file_sectors_absolute(&self, path: &str) -> Vec<usize> {
        let Some((first, _, is_dir)) = self.resolve_entry(path) else {
            return Vec::new();
        };
        if is_dir {
            return Vec::new();
        }
        map::cluster_chain_sectors(&self.data, &self.bpb, self.fat_type, first)
            .into_iter()
            .map(|s| s + self.lba_offset)
            .collect()
    }

    /// Resolve a path to `(first_cluster, size, is_dir)` by walking the tree.
    fn resolve_entry(&self, path: &str) -> Option<(u16, u32, bool)> {
        let mut dir = DirLocation::Root;
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            let found = self.find_in_dir(&dir, component)?;
            let is_last = components.peek().is_none();
            if is_last {
                return Some(found);
            }
            if !found.2 {
                return None; // a non-final component must be a directory
            }
            dir = DirLocation::Cluster(found.0);
        }
        None
    }

    /// Find a child entry by name within `dir`, returning `(first, size, is_dir)`.
    fn find_in_dir(&self, dir: &DirLocation, name: &str) -> Option<(u16, u32, bool)> {
        let target = name.to_ascii_uppercase();
        let mut result = None;
        map::for_each_entry(&self.data, &self.bpb, self.fat_type, dir, |entry| {
            if map::entry_name(entry).eq_ignore_ascii_case(&target) {
                result = Some(decode_entry_location(entry));
                true
            } else {
                false
            }
        });
        result
    }

    /// Recursively build `DirEntry` rows under `dir`, prefixing paths with
    /// `prefix`. `visited` records the directory cluster chain starts already
    /// followed on this path, guarding against cyclic chains.
    fn read_dir(
        &self,
        dir: &DirLocation,
        prefix: &str,
        depth: usize,
        visited: &mut Vec<u16>,
    ) -> Vec<DirEntry> {
        if depth >= MAX_DEPTH {
            return Vec::new();
        }
        let mut rows = Vec::new();
        map::for_each_entry(&self.data, &self.bpb, self.fat_type, dir, |entry| {
            let name = map::entry_name(entry);
            // Skip dot/dotdot, which `entry_name` renders as "." / "..".
            if name == "." || name == ".." {
                return false;
            }
            let (first, size, is_dir) = decode_entry_location(entry);
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let children = if is_dir {
                if visited.contains(&first) {
                    Vec::new() // cyclic chain: do not recurse
                } else {
                    visited.push(first);
                    let nested =
                        self.read_dir(&DirLocation::Cluster(first), &path, depth + 1, visited);
                    visited.pop();
                    nested
                }
            } else {
                Vec::new()
            };
            rows.push(DirEntry {
                name,
                path,
                is_dir,
                size: if is_dir { 0 } else { size as u64 },
                attributes: decode_attributes(entry[11]),
                modified: decode_timestamp(entry),
                children,
            });
            false
        });
        rows
    }
}

/// Decode `(first_cluster, size, is_dir)` from a 32-byte directory entry.
fn decode_entry_location(entry: &[u8]) -> (u16, u32, bool) {
    let first = u16::from_le_bytes([entry[26], entry[27]]);
    let size = u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]);
    let is_dir = entry[11] & 0x10 != 0;
    (first, size, is_dir)
}

/// Decode the DOS attribute byte into an [`Attributes`] set.
fn decode_attributes(attr: u8) -> Attributes {
    Attributes {
        read_only: attr & 0x01 != 0,
        hidden: attr & 0x02 != 0,
        system: attr & 0x04 != 0,
        archive: attr & 0x20 != 0,
    }
}

/// Decode the DOS date/time fields (bytes 22-25) into a [`Timestamp`], or
/// `None` when the entry carries the 1980 epoch / no real date.
fn decode_timestamp(entry: &[u8]) -> Option<Timestamp> {
    let time = u16::from_le_bytes([entry[22], entry[23]]);
    let date = u16::from_le_bytes([entry[24], entry[25]]);
    if date == 0 {
        return None;
    }
    let year = 1980 + (date >> 9);
    let month = ((date >> 5) & 0x0F) as u8;
    let day = (date & 0x1F) as u8;
    if year <= 1980 {
        return None;
    }
    Some(Timestamp {
        year,
        month,
        day,
        hour: ((time >> 11) & 0x1F) as u8,
        minute: ((time >> 5) & 0x3F) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic FAT16 volume with a file and a subdirectory, hand-laid
    /// so the FAT-width detection and chain walking are exercised independently
    /// of `fatfs`. Returns the raw bytes (byte 0 = boot sector).
    ///
    /// Layout: 1 reserved + 2 FATs * spf + root + data. spc = 1.
    struct Builder {
        data: Vec<u8>,
        spf: usize,
        root_entries: usize,
        num_fats: usize,
        reserved: usize,
        spc: usize,
    }

    impl Builder {
        fn new(total_sectors: usize, spf: usize, root_entries: usize) -> Builder {
            let mut b = Builder {
                data: vec![0u8; total_sectors * SECTOR_SIZE],
                spf,
                root_entries,
                num_fats: 2,
                reserved: 1,
                spc: 1,
            };
            b.write_bpb();
            b
        }

        fn write_bpb(&mut self) {
            let d = &mut self.data;
            d[0] = 0xEB;
            d[1] = 0x3C;
            d[2] = 0x90;
            d[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
            d[13] = self.spc as u8;
            d[14..16].copy_from_slice(&(self.reserved as u16).to_le_bytes());
            d[16] = self.num_fats as u8;
            d[17..19].copy_from_slice(&(self.root_entries as u16).to_le_bytes());
            d[22..24].copy_from_slice(&(self.spf as u16).to_le_bytes());
            d[510] = 0x55;
            d[511] = 0xAA;
        }

        fn root_start(&self) -> usize {
            self.reserved + self.num_fats * self.spf
        }

        fn data_start(&self) -> usize {
            self.root_start() + (self.root_entries * 32).div_ceil(SECTOR_SIZE)
        }

        fn cluster_first_sector(&self, c: usize) -> usize {
            self.data_start() + (c - 2) * self.spc
        }

        /// Write a FAT16 entry (in both FAT copies) for `cluster`.
        fn set_fat16(&mut self, cluster: usize, value: u16) {
            for fat in 0..self.num_fats {
                let fat_start = (self.reserved + fat * self.spf) * SECTOR_SIZE;
                let off = fat_start + cluster * 2;
                self.data[off..off + 2].copy_from_slice(&value.to_le_bytes());
            }
        }

        /// Write a directory entry at `slot` of the sector beginning at `base`.
        fn write_dir_entry(
            &mut self,
            base_sector: usize,
            slot: usize,
            name: &[u8; 11],
            attr: u8,
            first: u16,
            size: u32,
        ) {
            let off = base_sector * SECTOR_SIZE + slot * 32;
            self.data[off..off + 11].copy_from_slice(name);
            self.data[off + 11] = attr;
            self.data[off + 26..off + 28].copy_from_slice(&first.to_le_bytes());
            self.data[off + 28..off + 32].copy_from_slice(&size.to_le_bytes());
        }

        fn write_cluster_bytes(&mut self, cluster: usize, bytes: &[u8]) {
            let off = self.cluster_first_sector(cluster) * SECTOR_SIZE;
            self.data[off..off + bytes.len()].copy_from_slice(bytes);
        }
    }

    /// A FAT16 volume: 8000 sectors, spf=64 (forces FAT16 by the rule), with
    /// HELLO.TXT (cluster 2) and a SUB directory (cluster 3) holding
    /// SUB/HI.TXT (cluster 4).
    fn fat16_volume() -> Vec<u8> {
        let mut b = Builder::new(8000, 64, 512);
        let root = b.root_start();
        let payload = b"hello world";
        // HELLO.TXT -> cluster 2, end-of-chain.
        b.write_dir_entry(root, 0, b"HELLO   TXT", 0x20, 2, payload.len() as u32);
        b.set_fat16(2, 0xFFFF);
        b.write_cluster_bytes(2, payload);
        // SUB directory -> cluster 3.
        b.write_dir_entry(root, 1, b"SUB        ", 0x10, 3, 0);
        b.set_fat16(3, 0xFFFF);
        // Volume label in root.
        b.write_dir_entry(root, 2, b"MYVOL      ", 0x08, 0, 0);
        // Inside SUB (cluster 3): . / .. / HI.TXT (cluster 4).
        let sub = b.cluster_first_sector(3);
        b.write_dir_entry(sub, 0, b".          ", 0x10, 3, 0);
        b.write_dir_entry(sub, 1, b"..         ", 0x10, 0, 0);
        let sub_payload = b"sub file body";
        b.write_dir_entry(sub, 2, b"HI      TXT", 0x20, 4, sub_payload.len() as u32);
        b.set_fat16(4, 0xFFFF);
        b.write_cluster_bytes(4, sub_payload);
        b.data
    }

    fn mount(bytes: Vec<u8>) -> Volume {
        // Mount as a whole image at LBA 0 (no clamping).
        let total = bytes.len() / SECTOR_SIZE;
        Volume::from_image_slice(&bytes, 0, total).expect("mount volume")
    }

    #[test]
    fn detects_fat16_for_hand_built_volume() {
        let vol = mount(fat16_volume());
        assert_eq!(vol.fat_type(), FatType::Fat16);
    }

    #[test]
    fn reads_volume_label() {
        let vol = mount(fat16_volume());
        assert_eq!(vol.volume_label(), Some("MYVOL"));
    }

    #[test]
    fn tree_lists_files_and_recurses_into_subdir() {
        let vol = mount(fat16_volume());
        let tree = vol.tree();
        let names: Vec<&str> = tree.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"HELLO.TXT"));
        assert!(names.contains(&"SUB"));
        // The volume label is not listed as a file.
        assert!(!names.contains(&"MYVOL"));
        let sub = tree.iter().find(|e| e.name == "SUB").unwrap();
        assert!(sub.is_dir);
        assert_eq!(sub.children.len(), 1);
        assert_eq!(sub.children[0].name, "HI.TXT");
        assert_eq!(sub.children[0].path, "SUB/HI.TXT");
    }

    #[test]
    fn read_file_truncates_to_entry_size() {
        let vol = mount(fat16_volume());
        // HELLO.TXT is 11 bytes inside a 512-byte cluster; must return 11.
        let bytes = vol.read_file("HELLO.TXT").unwrap();
        assert_eq!(bytes, b"hello world");
        // A file in a subdirectory.
        let sub = vol.read_file("SUB/HI.TXT").unwrap();
        assert_eq!(sub, b"sub file body");
    }

    #[test]
    fn read_missing_file_is_none() {
        let vol = mount(fat16_volume());
        assert!(vol.read_file("NOPE.XXX").is_none());
        assert!(vol.read_file("SUB").is_none()); // a directory, not a file
    }

    #[test]
    fn cyclic_chain_does_not_loop_forever() {
        // Build a volume whose SUB directory's chain points back at itself.
        let mut b = Builder::new(8000, 64, 512);
        let root = b.root_start();
        b.write_dir_entry(root, 0, b"SUB        ", 0x10, 3, 0);
        // Cluster 3 -> 3 (self-referential chain).
        b.set_fat16(3, 3);
        let sub = b.cluster_first_sector(3);
        b.write_dir_entry(sub, 0, b".          ", 0x10, 3, 0);
        b.write_dir_entry(sub, 1, b"..         ", 0x10, 0, 0);
        // A nested SUB entry pointing back at cluster 3 to trigger recursion.
        b.write_dir_entry(sub, 2, b"LOOP       ", 0x10, 3, 0);
        let vol = mount(b.data);
        // Must terminate (the guard stops re-entering cluster 3).
        let tree = vol.tree();
        assert!(tree.iter().any(|e| e.name == "SUB"));
    }

    #[test]
    fn from_image_slice_clamps_and_offsets() {
        // Place the volume at LBA 4 of a larger image; sectors come back offset.
        let vol_bytes = fat16_volume();
        let lba = 4usize;
        let mut image = vec![0u8; lba * SECTOR_SIZE];
        image.extend_from_slice(&vol_bytes);
        let total = vol_bytes.len() / SECTOR_SIZE;
        let vol = Volume::from_image_slice(&image, lba, total).unwrap();
        assert_eq!(vol.lba_offset(), lba);
        let sectors = vol.file_sectors_absolute("HELLO.TXT");
        assert!(!sectors.is_empty());
        // Absolute sectors are volume-relative + lba_offset.
        assert!(sectors.iter().all(|&s| s >= lba));
    }
}
