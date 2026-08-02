//! Detection of images that hold several whole disks back to back.
//!
//! Multi-disk MSX releases are often distributed as one file with each disk
//! image concatenated — `Aleste2.dsk` is three 720 kB disks in a 2.2 MB file.
//! Opened naively such a file mounts as a single volume using the *first*
//! disk's boot sector, so everything past the first disk is invisible and the
//! reported geometry is nonsense (240 tracks).
//!
//! Splitting is deliberately conservative. It is not enough for the length to
//! divide evenly by a standard disk size: every slice must also carry a boot
//! sector whose BPB declares exactly that slice's own sector count. That is
//! what stops an ordinary 720 kB disk — whose length is also two 360 kB disks —
//! from being torn in half, and what rejects the halves of a large image that
//! merely happens to be a multiple of a floppy size.

use super::map::Bpb;
use crate::image::geometry::{Geometry, SECTOR_SIZE, SIZE_360K, SIZE_720K};

/// Standard MSX floppy sizes, largest first so a 3x720 kB image is never
/// mistaken for 6x360 kB.
const STANDARD_SIZES: [usize; 2] = [SIZE_720K, SIZE_360K];

/// Offset of the BPB's 16-bit total-sector count within a boot sector.
const TOTAL_SECTORS_AT: usize = 0x13;

/// One whole disk inside a concatenated image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSlice {
    /// Zero-based position in the file.
    pub index: usize,
    /// First sector (LBA) of this disk within the image.
    pub lba_start: usize,
    /// Length of this disk in sectors.
    pub sector_count: usize,
}

impl DiskSlice {
    /// Byte length of this disk.
    pub fn byte_len(&self) -> usize {
        self.sector_count * SECTOR_SIZE
    }

    /// The physical shape of this disk, from its size.
    pub fn geometry(&self) -> Geometry {
        match self.byte_len() {
            SIZE_360K => Geometry::SS_360K,
            _ => Geometry::DS_720K,
        }
    }
}

/// The BPB's declared total sector count, or `None` when the boot sector does
/// not parse as one.
fn declared_sectors(slice: &[u8]) -> Option<usize> {
    Bpb::parse(slice)?;
    let raw = slice.get(TOTAL_SECTORS_AT..TOTAL_SECTORS_AT + 2)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]) as usize)
}

/// Split `data` into whole disks if it holds two or more of them concatenated,
/// or `None` for an ordinary single-disk image.
pub fn split(data: &[u8]) -> Option<Vec<DiskSlice>> {
    for size in STANDARD_SIZES {
        if data.len() < size * 2 || !data.len().is_multiple_of(size) {
            continue;
        }
        let count = data.len() / size;
        let sectors = size / SECTOR_SIZE;
        // Every slice must describe itself, not the whole file: this is the
        // check that makes the split safe.
        let all_match = (0..count).all(|i| {
            let at = i * size;
            data.get(at..at + SECTOR_SIZE)
                .and_then(declared_sectors)
                .is_some_and(|declared| declared == sectors)
        });
        if all_match {
            return Some(
                (0..count)
                    .map(|index| DiskSlice {
                        index,
                        lba_start: index * sectors,
                        sector_count: sectors,
                    })
                    .collect(),
            );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A boot sector for a disk of `sectors` sectors, with the BPB fields the
    /// split relies on.
    fn boot_sector(sectors: u16, spf: u16) -> Vec<u8> {
        let mut s = vec![0u8; SECTOR_SIZE];
        s[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes()); // bytes/sector
        s[0x0D] = 2; // sectors/cluster
        s[0x0E..0x10].copy_from_slice(&1u16.to_le_bytes()); // reserved
        s[0x10] = 2; // FATs
        s[0x11..0x13].copy_from_slice(&112u16.to_le_bytes()); // root entries
        s[0x13..0x15].copy_from_slice(&sectors.to_le_bytes()); // total sectors
        s[0x16..0x18].copy_from_slice(&spf.to_le_bytes()); // sectors/FAT
        s
    }

    /// `count` disks of `size` bytes each, every one carrying a matching BPB.
    fn concatenated(count: usize, size: usize) -> Vec<u8> {
        let sectors = (size / SECTOR_SIZE) as u16;
        let spf = if size == SIZE_360K { 2 } else { 3 };
        let mut data = vec![0u8; size * count];
        for i in 0..count {
            let at = i * size;
            data[at..at + SECTOR_SIZE].copy_from_slice(&boot_sector(sectors, spf));
        }
        data
    }

    #[test]
    fn three_concatenated_720k_disks_are_split() {
        let data = concatenated(3, SIZE_720K);
        let slices = split(&data).expect("split");
        assert_eq!(slices.len(), 3);
        assert_eq!(slices[0].lba_start, 0);
        assert_eq!(slices[1].lba_start, 1440);
        assert_eq!(slices[2].lba_start, 2880);
        assert!(slices.iter().all(|s| s.sector_count == 1440));
        assert_eq!(slices[0].geometry(), Geometry::DS_720K);
    }

    #[test]
    fn two_concatenated_360k_disks_are_split() {
        let data = concatenated(2, SIZE_360K);
        let slices = split(&data).expect("split");
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[1].lba_start, 720);
        assert_eq!(slices[0].geometry(), Geometry::SS_360K);
    }

    /// The whole point of the self-description check: a plain 720 kB disk is
    /// exactly two 360 kB disks long, and must never be torn in half.
    #[test]
    fn a_single_720k_disk_is_not_split_into_two_360k_halves() {
        let mut data = vec![0u8; SIZE_720K];
        data[..SECTOR_SIZE].copy_from_slice(&boot_sector(1440, 3));
        assert_eq!(split(&data), None);
    }

    #[test]
    fn a_single_360k_disk_is_not_split() {
        let mut data = vec![0u8; SIZE_360K];
        data[..SECTOR_SIZE].copy_from_slice(&boot_sector(720, 2));
        assert_eq!(split(&data), None);
    }

    /// A multiple of a disk size whose later slices are just data, not boot
    /// sectors — an ordinary large image — stays whole.
    #[test]
    fn a_large_image_that_is_merely_a_multiple_is_not_split() {
        let mut data = vec![0u8; SIZE_720K * 4];
        data[..SECTOR_SIZE].copy_from_slice(&boot_sector(1440, 3));
        assert_eq!(split(&data), None);
    }

    /// Every slice has to check out; one bad disk in the middle means the file
    /// is not a clean concatenation and is left alone.
    #[test]
    fn one_slice_without_a_matching_bpb_prevents_the_split() {
        let mut data = concatenated(3, SIZE_720K);
        let at = SIZE_720K; // second disk's boot sector
        data[at..at + SECTOR_SIZE].fill(0xAB);
        assert_eq!(split(&data), None);
    }

    /// A slice whose BPB describes a different-sized disk is not this disk.
    #[test]
    fn a_slice_declaring_the_wrong_sector_count_prevents_the_split() {
        let mut data = concatenated(2, SIZE_720K);
        let at = SIZE_720K;
        data[at..at + SECTOR_SIZE].copy_from_slice(&boot_sector(720, 3));
        assert_eq!(split(&data), None);
    }

    #[test]
    fn odd_sizes_and_tiny_buffers_are_left_alone() {
        assert_eq!(split(&[]), None);
        assert_eq!(split(&[0u8; 1024]), None);
        assert_eq!(split(&vec![0u8; SIZE_720K + 512]), None);
    }

    #[test]
    fn slice_byte_length_matches_its_sector_count() {
        let slices = split(&concatenated(3, SIZE_720K)).expect("split");
        assert!(slices.iter().all(|s| s.byte_len() == SIZE_720K));
    }
}
