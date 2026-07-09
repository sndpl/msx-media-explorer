//! `new` — build a blank, formatted disk image with a real MSX-DOS boot block.

use msx_disk::fs::{write, DosVersion};
use msx_disk::image::geometry::DiskFormat;

use crate::disk::CmdResult;

/// A freshly formatted image of `format` carrying a `dos` boot block. The
/// serial only matters for DOS 2, where it becomes the disk's volume serial.
pub fn build(format: DiskFormat, dos: DosVersion, serial: u32) -> CmdResult<Vec<u8>> {
    let blank = write::create_blank(format).map_err(|e| e.to_string())?;
    write::set_boot_block(&blank, dos, serial).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use msx_disk::fs::{detect_dos_version, DiskFs};
    use msx_disk::image::geometry::SECTOR_SIZE;

    #[test]
    fn every_format_mounts_empty_and_accepts_a_file() {
        for format in DiskFormat::ALL {
            let disk = build(format, DosVersion::Dos2, 42).unwrap();
            assert_eq!(disk.len(), format.total_bytes(), "{format:?}");
            let fs = DiskFs::mount(disk.clone()).unwrap();
            assert!(fs.tree().unwrap().is_empty(), "{format:?}");
            let out = write::add_files(&disk, &[("OK.TXT", b"ok")]).unwrap();
            assert_eq!(
                DiskFs::mount(out).unwrap().read_file("OK.TXT").unwrap(),
                b"ok"
            );
        }
    }

    #[test]
    fn dos_choice_lands_in_the_boot_sector() {
        let dos1 = build(DiskFormat::Ds720, DosVersion::Dos1, 0).unwrap();
        let dos2 = build(DiskFormat::Ds720, DosVersion::Dos2, 0xAABBCCDD).unwrap();
        assert_eq!(
            detect_dos_version(&dos1[..SECTOR_SIZE], &[]),
            DosVersion::Dos1
        );
        assert_eq!(
            detect_dos_version(&dos2[..SECTOR_SIZE], &[]),
            DosVersion::Dos2
        );
        assert_eq!(&dos2[0x27..0x2B], &0xAABBCCDDu32.to_le_bytes());
    }
}
