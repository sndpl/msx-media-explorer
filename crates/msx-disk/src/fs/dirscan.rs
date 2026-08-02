//! Structural validation of raw root-directory entries.
//!
//! Plenty of MSX game disks carry a plausible-looking BPB but no filesystem at
//! all: the loader reads raw sectors, and the "root directory" area holds Z80
//! code. `fatfs` mounts such a disk happily and reports whatever those bytes
//! decode to — typically one nonsense file with an impossible size — which is
//! worse than reporting nothing, because it looks like real content.
//!
//! This module reads the 32-byte entries itself and rejects the ones that
//! *cannot* describe a file. Only structural impossibilities count:
//!
//! - reserved attribute bits set (bits 6 and 7 are defined as zero),
//! - a first cluster outside the volume,
//! - a size larger than the whole data area.
//!
//! Deliberately **not** used, each having been measured against a corpus of
//! real MSX disks and found to reject working ones:
//!
//! - *control bytes in the 8.3 name* — some authors sign their disks with
//!   entries like `P.Vaesen` padded with control bytes; those disks work.
//! - *a missing `0x55AA` boot signature* — most genuine MSX disks lack it.
//! - *the two FAT copies disagreeing* — real disks are found with 70% of their
//!   second FAT differing, still perfectly readable.

use super::map::Bpb;
use crate::image::geometry::SECTOR_SIZE;

/// Bytes per directory entry.
const ENTRY_SIZE: usize = 32;

/// Attribute bits with no defined meaning; a real entry leaves them clear.
const ATTR_RESERVED: u8 = 0xC0;
const ATTR_VOLUME_ID: u8 = 0x08;
/// The attribute value marking a long-file-name fragment rather than an entry.
const ATTR_LFN: u8 = 0x0F;

/// First byte of an entry that has never been used: the end of the directory.
const ENTRY_END: u8 = 0x00;
/// First byte of a deleted entry.
const ENTRY_DELETED: u8 = 0xE5;
/// FAT stores a leading 0xE5 as 0x05, since 0xE5 marks a deletion.
const ENTRY_REALLY_E5: u8 = 0x05;

/// Why a directory entry cannot describe a real file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    /// Attribute bits that are defined as zero are set.
    ReservedAttributeBits,
    /// The first cluster is outside the volume's cluster range.
    ClusterOutOfRange { cluster: u32, max: u32 },
    /// The file is bigger than the disk it is supposedly stored on.
    SizeExceedsVolume { size: u64, capacity: u64 },
}

/// A rejected directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidEntry {
    /// The entry's short name in `fatfs`'s formatted form (`NAME.EXT`, padding
    /// trimmed), which is what [`short_name_key`] produces and what the tree
    /// builder matches against.
    pub key: Vec<u8>,
    pub reason: Invalid,
}

/// The result of validating a volume's root directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootScan {
    /// Entries that claim to be files or directories, i.e. everything but free,
    /// deleted, long-name and volume-label slots.
    pub live: usize,
    /// Those of them that cannot be real, in directory order.
    pub invalid: Vec<InvalidEntry>,
}

impl RootScan {
    /// Whether this volume has no usable FAT filesystem: it has directory
    /// entries and *every* one of them is structurally impossible.
    ///
    /// Requiring all of them keeps damaged-but-real disks — which have a few
    /// bad entries among many good ones — out of this verdict.
    pub fn has_no_filesystem(&self) -> bool {
        self.live > 0 && self.invalid.len() == self.live
    }

    /// Whether the entry with this short-name key was rejected.
    pub fn rejects(&self, key: &[u8]) -> bool {
        self.invalid.iter().any(|e| e.key == key)
    }
}

/// The formatted short name `fatfs` reports for a raw 8.3 directory name:
/// padding trimmed, a dot inserted before a non-empty extension, and a leading
/// `0x05` restored to `0xE5`. Mirrors `fatfs`'s own `ShortName`, so the two can
/// be compared byte for byte.
fn short_name_key(raw: &[u8]) -> Vec<u8> {
    let trimmed = |slice: &[u8]| slice.iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
    let name_len = trimmed(&raw[..8]);
    let ext_len = trimmed(&raw[8..11]);
    let mut key = raw[..name_len].to_vec();
    if ext_len > 0 {
        key.push(b'.');
        key.extend_from_slice(&raw[8..8 + ext_len]);
    }
    if key.first() == Some(&ENTRY_REALLY_E5) {
        key[0] = ENTRY_DELETED;
    }
    key
}

/// Validate the root directory of the volume in `data` (a normalized sector
/// buffer whose boot sector has already been repaired).
///
/// Returns `None` when the buffer has no parseable BPB at all, in which case
/// there is nothing to validate against.
pub fn scan_root(data: &[u8]) -> Option<RootScan> {
    let bpb = Bpb::parse(data)?;
    let total_sectors = data.len() / SECTOR_SIZE;
    let data_start = bpb.data_start();
    if total_sectors <= data_start {
        return None;
    }
    // A file cannot be larger than the data area, and cannot start outside it.
    let capacity = ((total_sectors - data_start) * SECTOR_SIZE) as u64;
    let max_cluster = bpb.cluster_count(total_sectors) as u32 + 1;

    let root = bpb.root_start() * SECTOR_SIZE;
    let mut scan = RootScan::default();
    for slot in 0..bpb.root_entries {
        let at = root + slot * ENTRY_SIZE;
        let Some(entry) = data.get(at..at + ENTRY_SIZE) else {
            break;
        };
        match entry[0] {
            // Never-used slot: MSX-DOS stops scanning here, so we do too.
            ENTRY_END => break,
            ENTRY_DELETED => continue,
            _ => {}
        }
        let attr = entry[11];
        if attr == ATTR_LFN || attr & ATTR_VOLUME_ID != 0 {
            // A long-name fragment or the volume label: not a file either way,
            // and the tree builder skips both.
            continue;
        }
        scan.live += 1;

        let cluster = u16::from_le_bytes([entry[26], entry[27]]) as u32;
        let size = u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]) as u64;
        let reason = if attr & ATTR_RESERVED != 0 {
            Invalid::ReservedAttributeBits
        } else if cluster != 0 && !(2..=max_cluster).contains(&cluster) {
            Invalid::ClusterOutOfRange {
                cluster,
                max: max_cluster,
            }
        } else if size > capacity {
            Invalid::SizeExceedsVolume { size, capacity }
        } else {
            continue;
        };
        scan.invalid.push(InvalidEntry {
            key: short_name_key(&entry[..11]),
            reason,
        });
    }
    Some(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 720 kB volume with a standard MSX BPB and an empty root directory.
    fn blank_volume() -> Vec<u8> {
        let mut data = vec![0u8; 737_280];
        let bpb: [(usize, &[u8]); 7] = [
            (11, &[0x00, 0x02]), // 512 bytes/sector
            (13, &[2]),          // 2 sectors/cluster
            (14, &[0x01, 0x00]), // 1 reserved sector
            (16, &[2]),          // 2 FATs
            (17, &[112, 0x00]),  // 112 root entries
            (19, &[0xA0, 0x05]), // 1440 total sectors
            (22, &[0x03, 0x00]), // 3 sectors per FAT
        ];
        for (at, bytes) in bpb {
            data[at..at + bytes.len()].copy_from_slice(bytes);
        }
        data
    }

    /// Offset of root-directory slot `n` in [`blank_volume`].
    fn slot(n: usize) -> usize {
        let bpb = Bpb::parse(&blank_volume()).expect("bpb");
        bpb.root_start() * SECTOR_SIZE + n * ENTRY_SIZE
    }

    /// Write a directory entry into slot `n`.
    fn put(data: &mut [u8], n: usize, name: &[u8; 11], attr: u8, cluster: u16, size: u32) {
        let at = slot(n);
        data[at..at + 11].copy_from_slice(name);
        data[at + 11] = attr;
        data[at + 26..at + 28].copy_from_slice(&cluster.to_le_bytes());
        data[at + 28..at + 32].copy_from_slice(&size.to_le_bytes());
    }

    #[test]
    fn an_ordinary_file_is_accepted() {
        let mut data = blank_volume();
        put(&mut data, 0, b"GAME    COM", 0x20, 2, 4096);
        let scan = scan_root(&data).expect("scan");
        assert_eq!(scan.live, 1);
        assert!(scan.invalid.is_empty());
        assert!(!scan.has_no_filesystem());
    }

    #[test]
    fn a_size_larger_than_the_disk_is_rejected() {
        let mut data = blank_volume();
        put(&mut data, 0, b"HUGE    BIN", 0x20, 2, 3_236_299_646);
        let scan = scan_root(&data).expect("scan");
        assert_eq!(scan.live, 1);
        assert!(matches!(
            scan.invalid[0].reason,
            Invalid::SizeExceedsVolume { .. }
        ));
        assert!(scan.has_no_filesystem());
    }

    #[test]
    fn a_first_cluster_outside_the_volume_is_rejected() {
        let mut data = blank_volume();
        put(&mut data, 0, b"BAD     DAT", 0x20, 64_847, 512);
        let scan = scan_root(&data).expect("scan");
        assert!(matches!(
            scan.invalid[0].reason,
            Invalid::ClusterOutOfRange {
                cluster: 64_847,
                ..
            }
        ));
    }

    #[test]
    fn reserved_attribute_bits_are_rejected() {
        let mut data = blank_volume();
        put(&mut data, 0, b"CODE    BIN", 0xC6, 2, 512);
        let scan = scan_root(&data).expect("scan");
        assert_eq!(scan.invalid[0].reason, Invalid::ReservedAttributeBits);
    }

    /// A cluster of 0 means "no data allocated" and is normal for an empty
    /// file or a placeholder entry; it must not be read as out of range.
    #[test]
    fn a_zero_first_cluster_is_accepted() {
        let mut data = blank_volume();
        put(&mut data, 0, b"EMPTY   TXT", 0x20, 0, 0);
        let scan = scan_root(&data).expect("scan");
        assert!(scan.invalid.is_empty());
    }

    /// MSX names carry kana and accented Latin as bytes >= 0x80, and some real
    /// disks pad names with control bytes. Neither may be treated as invalid.
    #[test]
    fn unusual_name_bytes_are_not_a_rejection_reason() {
        let mut data = blank_volume();
        put(&mut data, 0, b"\x83J\x83i    PIC", 0x20, 2, 512);
        put(&mut data, 1, b"P.Vaesen\x01\x02\x03", 0x00, 0, 0);
        let scan = scan_root(&data).expect("scan");
        assert_eq!(scan.live, 2);
        assert!(scan.invalid.is_empty(), "{:?}", scan.invalid);
    }

    #[test]
    fn scanning_stops_at_the_end_of_directory_marker() {
        let mut data = blank_volume();
        put(&mut data, 0, b"FIRST   TXT", 0x20, 2, 512);
        // Slot 1 left as zeros: the end marker. Slot 2 is past it.
        put(&mut data, 2, b"UNSEEN  TXT", 0x20, 2, 512);
        let scan = scan_root(&data).expect("scan");
        assert_eq!(scan.live, 1);
    }

    #[test]
    fn deleted_volume_label_and_long_name_slots_are_not_counted() {
        let mut data = blank_volume();
        put(&mut data, 0, b"\xe5ELETED TXT", 0x20, 2, 512);
        put(&mut data, 1, b"LABEL      ", ATTR_VOLUME_ID, 0, 0);
        put(&mut data, 2, b"LFNFRAGMENT", ATTR_LFN, 0, 0);
        put(&mut data, 3, b"REAL    TXT", 0x20, 2, 512);
        let scan = scan_root(&data).expect("scan");
        assert_eq!(scan.live, 1);
        assert!(scan.invalid.is_empty());
    }

    /// The verdict needs *every* live entry to be impossible: a disk with real
    /// files plus one bad entry is damaged, not filesystem-less.
    #[test]
    fn one_bad_entry_among_good_ones_is_not_a_missing_filesystem() {
        let mut data = blank_volume();
        put(&mut data, 0, b"GOOD    COM", 0x20, 2, 4096);
        put(&mut data, 1, b"HUGE    BIN", 0x20, 2, 3_236_299_646);
        let scan = scan_root(&data).expect("scan");
        assert_eq!(scan.live, 2);
        assert_eq!(scan.invalid.len(), 1);
        assert!(!scan.has_no_filesystem());
    }

    #[test]
    fn an_empty_root_directory_is_not_a_missing_filesystem() {
        let scan = scan_root(&blank_volume()).expect("scan");
        assert_eq!(scan.live, 0);
        assert!(!scan.has_no_filesystem());
    }

    #[test]
    fn a_buffer_without_a_bpb_yields_no_scan() {
        assert!(scan_root(&[0u8; 1024]).is_none());
    }

    /// The rejection key must match what `fatfs` reports for the same entry,
    /// or the tree builder would filter nothing.
    #[test]
    fn short_name_keys_match_the_fatfs_format() {
        assert_eq!(short_name_key(b"GAME    COM"), b"GAME.COM");
        assert_eq!(short_name_key(b"NOEXT      "), b"NOEXT");
        assert_eq!(short_name_key(b"\x05ELETED TXT"), b"\xe5ELETED.TXT");
        assert_eq!(short_name_key(b"           "), b"");
    }

    #[test]
    fn rejects_looks_entries_up_by_key() {
        let mut data = blank_volume();
        put(&mut data, 0, b"HUGE    BIN", 0x20, 2, 3_236_299_646);
        let scan = scan_root(&data).expect("scan");
        assert!(scan.rejects(b"HUGE.BIN"));
        assert!(!scan.rejects(b"GAME.COM"));
    }
}
