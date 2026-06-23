//! MSX-DOS version detection.
//!
//! MSX-DOS 1 and MSX-DOS 2 disks are both FAT12; the format carries no version
//! flag. The most reliable on-disk marker lives in the boot sector: MSX-DOS 2
//! format writes the ASCII string `VOL_ID` at offset 0x20 (immediately before a
//! 4-byte volume serial number), where MSX-DOS 1 has Z80 boot code instead. This
//! is the `vol_id` field openMSX documents in its `MSXBootSector` layout, and is
//! the same marker real MSX-DOS 2.20 and openMSX write.
//!
//! A boot sector can be wiped or non-standard, so detection also accepts two
//! filesystem-level fallbacks that only MSX-DOS 2 produces: the presence of a
//! subdirectory, or of the `MSXDOS2.SYS` system file.

use super::entry::DirEntry;

/// Which MSX-DOS generation produced a disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DosVersion {
    Dos1,
    Dos2,
}

/// Offset of the `VOL_ID` marker MSX-DOS 2 writes into the boot sector.
const VOL_ID_OFFSET: usize = 0x20;
/// The ASCII marker itself.
const VOL_ID_MARKER: &[u8] = b"VOL_ID";
/// The MSX-DOS 2 system file, present only on DOS2 system disks.
const MSXDOS2_SYS: &str = "MSXDOS2.SYS";

/// Classify a disk as MSX-DOS 1 or 2.
///
/// `boot_sector` must be the raw first sector of the image — not one that has
/// been through [`super::boot::repair_boot_sector`], which zeroes the marker
/// region. `tree` is the mounted root directory listing (from
/// [`super::DiskFs::tree`]) used for the subdirectory / `MSXDOS2.SYS` fallbacks.
pub fn detect_dos_version(boot_sector: &[u8], tree: &[DirEntry]) -> DosVersion {
    if has_vol_id_marker(boot_sector) || has_subdirectory(tree) || has_msxdos2_sys(tree) {
        DosVersion::Dos2
    } else {
        DosVersion::Dos1
    }
}

/// Whether the boot sector carries MSX-DOS 2's `VOL_ID` marker.
fn has_vol_id_marker(boot_sector: &[u8]) -> bool {
    let end = VOL_ID_OFFSET + VOL_ID_MARKER.len();
    boot_sector.len() >= end && &boot_sector[VOL_ID_OFFSET..end] == VOL_ID_MARKER
}

/// Whether the disk has any subdirectory — only MSX-DOS 2 creates them. A nested
/// directory always implies a root-level one, so checking the root listing is
/// sufficient.
fn has_subdirectory(tree: &[DirEntry]) -> bool {
    tree.iter().any(|e| e.is_dir)
}

/// Whether the MSX-DOS 2 system file is present in the root directory.
fn has_msxdos2_sys(tree: &[DirEntry]) -> bool {
    tree.iter()
        .any(|e| !e.is_dir && e.name.eq_ignore_ascii_case(MSXDOS2_SYS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::geometry::SECTOR_SIZE;

    fn boot_with_marker() -> Vec<u8> {
        let mut boot = vec![0u8; SECTOR_SIZE];
        boot[VOL_ID_OFFSET..VOL_ID_OFFSET + VOL_ID_MARKER.len()].copy_from_slice(VOL_ID_MARKER);
        boot
    }

    fn entry(name: &str, is_dir: bool) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            path: name.to_string(),
            is_dir,
            size: 0,
            attributes: Default::default(),
            modified: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn detects_dos2_from_vol_id_marker() {
        assert_eq!(
            detect_dos_version(&boot_with_marker(), &[]),
            DosVersion::Dos2
        );
    }

    #[test]
    fn detects_dos2_when_subdirectory_present() {
        let boot = vec![0xC9u8; SECTOR_SIZE]; // no marker
        let tree = vec![entry("TOOLS", true)];
        assert_eq!(detect_dos_version(&boot, &tree), DosVersion::Dos2);
    }

    #[test]
    fn detects_dos2_when_msxdos2_sys_present() {
        let boot = vec![0xC9u8; SECTOR_SIZE]; // no marker
        let tree = vec![entry("msxdos2.sys", false)]; // case-insensitive
        assert_eq!(detect_dos_version(&boot, &tree), DosVersion::Dos2);
    }

    #[test]
    fn detects_dos1_without_any_signal() {
        let boot = vec![0xC9u8; SECTOR_SIZE]; // no marker
        let tree = vec![entry("README.TXT", false), entry("GAME.COM", false)];
        assert_eq!(detect_dos_version(&boot, &tree), DosVersion::Dos1);
    }

    #[test]
    fn short_buffer_is_dos1() {
        assert_eq!(detect_dos_version(&[0u8; 16], &[]), DosVersion::Dos1);
    }
}
