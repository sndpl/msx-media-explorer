//! `bootsector` — inspect or install the MSX-DOS 1/2 boot block.

use msx_disk::fs::{detect_dos_version, write, DiskFs, DosVersion};
use msx_disk::image::geometry::SECTOR_SIZE;

use crate::disk::CmdResult;

/// Offset of the `VOL_ID` marker / volume serial in a DOS 2 boot sector.
const VOL_ID: &[u8] = b"VOL_ID";
const VOL_ID_OFFSET: usize = 0x20;
const SERIAL_OFFSET: usize = 0x27;

/// Describe the current boot sector: DOS generation, plus the volume serial
/// when the DOS 2 `VOL_ID` marker is present.
pub fn describe(data: &[u8]) -> CmdResult<String> {
    if data.len() < SECTOR_SIZE {
        return Err("image smaller than one sector".into());
    }
    // The filesystem fallbacks (subdirectory / MSXDOS2.SYS) need the tree; a
    // disk that fails to mount is still classified from the boot sector alone.
    let tree = DiskFs::mount(data.to_vec())
        .and_then(|fs| fs.tree())
        .unwrap_or_default();
    let boot = &data[..SECTOR_SIZE];
    Ok(match detect_dos_version(boot, &tree) {
        DosVersion::Dos1 => "boot sector: MSX-DOS 1".to_string(),
        DosVersion::Dos2 => {
            let end = VOL_ID_OFFSET + VOL_ID.len();
            if &boot[VOL_ID_OFFSET..end] == VOL_ID {
                let serial =
                    u32::from_le_bytes(boot[SERIAL_OFFSET..SERIAL_OFFSET + 4].try_into().unwrap());
                format!("boot sector: MSX-DOS 2 (volume serial {serial:08X})")
            } else {
                "boot sector: MSX-DOS 2 (no VOL_ID marker; detected from filesystem)".to_string()
            }
        }
    })
}

/// Install a `dos` boot block (FIXDISK-style), preserving the disk's BPB.
pub fn apply(data: &[u8], dos: DosVersion, serial: u32) -> CmdResult<Vec<u8>> {
    write::set_boot_block(data, dos, serial).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;

    #[test]
    fn blank_disk_reports_dos1() {
        let text = describe(&testdisk::blank()).unwrap();
        assert_eq!(text, "boot sector: MSX-DOS 1");
    }

    #[test]
    fn installing_dos2_reports_generation_and_serial() {
        let out = apply(&testdisk::blank(), DosVersion::Dos2, 0xDEADBEEF).unwrap();
        let text = describe(&out).unwrap();
        assert_eq!(text, "boot sector: MSX-DOS 2 (volume serial DEADBEEF)");
    }

    #[test]
    fn a_subdirectory_alone_marks_dos2_without_marker() {
        let text = describe(&testdisk::populated()).unwrap();
        assert!(text.contains("MSX-DOS 2"), "{text}");
        assert!(text.contains("no VOL_ID"), "{text}");
    }

    #[test]
    fn downgrade_to_dos1_round_trips() {
        let dos2 = apply(&testdisk::blank(), DosVersion::Dos2, 1).unwrap();
        let dos1 = apply(&dos2, DosVersion::Dos1, 0).unwrap();
        assert_eq!(describe(&dos1).unwrap(), "boot sector: MSX-DOS 1");
    }
}
