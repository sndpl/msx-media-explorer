//! MSX-DOS version detection from the boot sector.
//!
//! MSX-DOS 1 and MSX-DOS 2 disks are both FAT12; the format carries no version
//! flag. The one reliable on-disk marker lives in the boot sector: MSX-DOS 2
//! format writes the ASCII string `VOL_ID` at offset 0x20 (immediately before a
//! 4-byte volume serial number), where MSX-DOS 1 has Z80 boot code instead. This
//! is the `vol_id` field openMSX documents in its `MSXBootSector` layout, and is
//! the same marker real MSX-DOS 2.20 and openMSX write.

/// Which MSX-DOS generation produced a disk's boot sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DosVersion {
    Dos1,
    Dos2,
}

/// Offset of the `VOL_ID` marker MSX-DOS 2 writes into the boot sector.
const VOL_ID_OFFSET: usize = 0x20;
/// The ASCII marker itself.
const VOL_ID_MARKER: &[u8] = b"VOL_ID";

/// Classify a disk as MSX-DOS 1 or 2 from its original, unrepaired boot sector.
///
/// Pass the raw first sector of the image — not one that has been through
/// [`super::boot::repair_boot_sector`], which zeroes the marker region.
pub fn detect_dos_version(boot_sector: &[u8]) -> DosVersion {
    let end = VOL_ID_OFFSET + VOL_ID_MARKER.len();
    if boot_sector.len() >= end && &boot_sector[VOL_ID_OFFSET..end] == VOL_ID_MARKER {
        DosVersion::Dos2
    } else {
        DosVersion::Dos1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::geometry::SECTOR_SIZE;

    #[test]
    fn detects_dos2_from_vol_id_marker() {
        let mut boot = vec![0u8; SECTOR_SIZE];
        boot[VOL_ID_OFFSET..VOL_ID_OFFSET + VOL_ID_MARKER.len()].copy_from_slice(VOL_ID_MARKER);
        assert_eq!(detect_dos_version(&boot), DosVersion::Dos2);
    }

    #[test]
    fn detects_dos1_without_marker() {
        // A boot sector that is all boot code / zeros, no VOL_ID at 0x20.
        let boot = vec![0xC9u8; SECTOR_SIZE];
        assert_eq!(detect_dos_version(&boot), DosVersion::Dos1);
    }

    #[test]
    fn short_buffer_is_dos1() {
        assert_eq!(detect_dos_version(&[0u8; 16]), DosVersion::Dos1);
    }
}
