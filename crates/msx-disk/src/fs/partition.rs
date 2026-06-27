//! MBR-style partition table parsing for MSX hard-disk images.
//!
//! openMSX hard-disk images (`MSX_IDE`) place a PC-style partition table at
//! byte offset `0x1BE` of sector 0, holding up to four 16-byte entries. Each
//! live partition is an independent FAT12/FAT16 volume starting at its own LBA.
//! This module locates the live partitions; [`super::volume`] mounts them.

use super::map::Bpb;
use crate::image::geometry::SECTOR_SIZE;

/// Byte offset of the first partition-table entry within sector 0.
const TABLE_OFFSET: usize = 0x1BE;
/// Size of one partition-table entry.
const ENTRY_SIZE: usize = 16;
/// Number of entries in an MBR partition table.
const ENTRY_COUNT: usize = 4;

/// One live partition discovered in the partition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionEntry {
    /// Table slot index (0-3) the entry was read from.
    pub index: usize,
    /// Status byte (`0x80` marks the active/boot partition).
    pub status: u8,
    /// Partition type byte.
    pub type_byte: u8,
    /// First sector (LBA) of the partition.
    pub lba_start: u32,
    /// Length of the partition in sectors.
    pub sector_count: u32,
}

/// The byte slice of the partition starting at `lba_start`, clamped to the
/// image's bounds, or `None` if the start is past the end of the image.
fn partition_slice<'a>(data: &'a [u8], entry: &PartitionEntry) -> Option<&'a [u8]> {
    // `lba_start` comes straight from an attacker-controlled partition table, so
    // the byte offset must use `checked_mul`: on a 32-bit `usize` target the
    // product can overflow. Mirrors `Volume::from_image_slice`.
    let start = (entry.lba_start as usize).checked_mul(SECTOR_SIZE)?;
    if start >= data.len() {
        return None;
    }
    let declared = (entry.sector_count as usize).saturating_mul(SECTOR_SIZE);
    let end = start.saturating_add(declared).min(data.len());
    Some(&data[start..end])
}

/// Parse the partition table at `0x1BE`, returning the live partitions sorted
/// ascending by `lba_start`, or `None` if `data` is too small to hold a table.
///
/// A slot is "live" when its type and sector count and LBA are all non-zero and
/// the slice it points at begins with a parseable FAT BPB. Empty/zero slots and
/// slots whose target is not a FAT volume are dropped.
pub fn parse_partition_table(data: &[u8]) -> Option<Vec<PartitionEntry>> {
    if data.len() < SECTOR_SIZE {
        return None;
    }
    let mut entries: Vec<PartitionEntry> = (0..ENTRY_COUNT)
        .filter_map(|i| {
            let off = TABLE_OFFSET + i * ENTRY_SIZE;
            let raw = data.get(off..off + ENTRY_SIZE)?;
            let entry = PartitionEntry {
                index: i,
                status: raw[0],
                type_byte: raw[4],
                lba_start: u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
                sector_count: u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]),
            };
            if entry.type_byte == 0 || entry.sector_count == 0 || entry.lba_start == 0 {
                return None;
            }
            let slice = partition_slice(data, &entry)?;
            Bpb::parse(slice)?;
            Some(entry)
        })
        .collect();
    entries.sort_by_key(|e| e.lba_start);
    Some(entries)
}

/// Whether `data` is a partitioned hard-disk image rather than a single FAT
/// volume.
///
/// True only when, on the *raw* (un-repaired) bytes, the boot signature
/// `0x55AA` is present at offset 510, sector 0 is itself **not** a valid FAT
/// BPB (so it is an MBR, not a volume boot sector), and at least one partition
/// slot points at a slice that begins with a valid BPB. This rejects ordinary
/// floppies (valid BPB at sector 0) and DOS1-garbage floppies (no sub-volume).
pub fn is_partitioned(data: &[u8]) -> bool {
    if data.len() < SECTOR_SIZE {
        return false;
    }
    if data[510] != 0x55 || data[511] != 0xAA {
        return false;
    }
    if Bpb::parse(&data[..SECTOR_SIZE]).is_some() {
        return false; // sector 0 is a volume boot sector, not an MBR
    }
    parse_partition_table(data).is_some_and(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a minimal but parseable FAT BPB into the first 512 bytes of `buf`.
    fn write_bpb(buf: &mut [u8], sectors_per_fat: u16) {
        buf[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
        buf[13] = 2; // sectors per cluster
        buf[14..16].copy_from_slice(&1u16.to_le_bytes()); // reserved
        buf[16] = 2; // num fats
        buf[17..19].copy_from_slice(&112u16.to_le_bytes()); // root entries
        buf[22..24].copy_from_slice(&sectors_per_fat.to_le_bytes());
    }

    /// Write a partition-table entry into slot `slot` of sector 0.
    fn write_entry(buf: &mut [u8], slot: usize, type_byte: u8, lba: u32, count: u32) {
        let off = TABLE_OFFSET + slot * ENTRY_SIZE;
        buf[off] = 0;
        buf[off + 4] = type_byte;
        buf[off + 8..off + 12].copy_from_slice(&lba.to_le_bytes());
        buf[off + 12..off + 16].copy_from_slice(&count.to_le_bytes());
    }

    /// Build a small partitioned image: an MBR plus `lbas` partitions, each a
    /// single sector of valid BPB. The table entries are written in the given
    /// order (so the test can verify sorting).
    fn make_partitioned(lbas: &[u32]) -> Vec<u8> {
        let max_lba = lbas.iter().copied().max().unwrap_or(0) as usize;
        let mut buf = vec![0u8; (max_lba + 2) * SECTOR_SIZE];
        buf[510] = 0x55;
        buf[511] = 0xAA;
        for (slot, &lba) in lbas.iter().enumerate() {
            write_entry(&mut buf, slot, 0x01, lba, 1);
            write_bpb(&mut buf[lba as usize * SECTOR_SIZE..], 3);
        }
        buf
    }

    #[test]
    fn parses_and_sorts_reverse_ordered_entries_by_lba() {
        // Slots written high-to-low LBA; result must be sorted ascending.
        let buf = make_partitioned(&[4, 3, 2, 1]);
        let parts = parse_partition_table(&buf).unwrap();
        assert_eq!(parts.len(), 4);
        let lbas: Vec<u32> = parts.iter().map(|p| p.lba_start).collect();
        assert_eq!(lbas, vec![1, 2, 3, 4]);
    }

    #[test]
    fn drops_empty_and_zero_entries() {
        let mut buf = make_partitioned(&[1, 2]);
        // Slot 2: zero type/count/lba (already zero) — stays dropped.
        // Slot 3: non-zero type but zero count -> dropped.
        write_entry(&mut buf, 3, 0x01, 9, 0);
        let parts = parse_partition_table(&buf).unwrap();
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn clamps_overlong_partition_to_image_bounds() {
        // A partition that claims far more sectors than the image holds is still
        // accepted; its slice is clamped so the BPB read does not run past EOF.
        let mut buf = make_partitioned(&[1]);
        write_entry(&mut buf, 0, 0x01, 1, 1_000_000);
        let parts = parse_partition_table(&buf).unwrap();
        assert_eq!(parts.len(), 1);
        let slice = partition_slice(&buf, &parts[0]).unwrap();
        assert!(slice.len() <= buf.len());
    }

    #[test]
    fn ignores_slot_pointing_at_non_fat_data() {
        // Slot 1 points at an LBA whose sector is not a valid BPB -> dropped.
        let mut buf = make_partitioned(&[1]);
        write_entry(&mut buf, 1, 0x01, 1, 1); // reuse lba 1 but garbage slot?
                                              // Point slot 1 at a sector with no BPB (lba 0, the MBR) -> lba_start==0
                                              // is already rejected, so instead point it past valid data.
        let extra = buf.len() / SECTOR_SIZE;
        buf.resize((extra + 1) * SECTOR_SIZE, 0);
        write_entry(&mut buf, 1, 0x01, extra as u32, 1);
        let parts = parse_partition_table(&buf).unwrap();
        // Only the original valid partition at lba 1 survives.
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].lba_start, 1);
    }

    #[test]
    fn drops_partition_whose_lba_start_is_out_of_range() {
        // An `lba_start` past the end of the image (here the u32 maximum) must be
        // dropped gracefully, never panic. The byte offset `lba_start * 512`
        // overflows `usize` on 32-bit targets, so the multiply is checked.
        let mut buf = make_partitioned(&[1]);
        write_entry(&mut buf, 1, 0x01, u32::MAX, 1);
        let parts = parse_partition_table(&buf).unwrap();
        // Only the in-bounds partition at lba 1 survives.
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].lba_start, 1);

        // The out-of-range entry yields no slice rather than panicking.
        let rogue = PartitionEntry {
            index: 1,
            status: 0,
            type_byte: 0x01,
            lba_start: u32::MAX,
            sector_count: 1,
        };
        assert!(partition_slice(&buf, &rogue).is_none());
    }

    #[test]
    fn is_partitioned_true_for_mbr_with_live_partition() {
        let buf = make_partitioned(&[1, 2]);
        assert!(is_partitioned(&buf));
    }

    #[test]
    fn is_partitioned_false_for_normal_floppy() {
        // Sector 0 is a valid BPB -> a single volume, not an MBR.
        let mut buf = vec![0u8; 32 * SECTOR_SIZE];
        write_bpb(&mut buf, 3);
        buf[510] = 0x55;
        buf[511] = 0xAA;
        assert!(!is_partitioned(&buf));
    }

    #[test]
    fn is_partitioned_false_for_dos1_garbage_floppy() {
        // 0x55AA present and sector 0 is not a BPB, but no slot points at a
        // valid sub-volume -> not partitioned.
        let mut buf = vec![0u8; 32 * SECTOR_SIZE];
        buf[510] = 0x55;
        buf[511] = 0xAA;
        // A garbage "partition" entry whose target has no BPB.
        write_entry(&mut buf, 0, 0x01, 10, 1);
        assert!(!is_partitioned(&buf));
    }

    #[test]
    fn is_partitioned_false_without_signature() {
        let buf = {
            let mut b = make_partitioned(&[1]);
            b[510] = 0;
            b[511] = 0;
            b
        };
        assert!(!is_partitioned(&buf));
    }
}
