//! `split` — pull a concatenated multi-disk image apart into one file per disk.

use msx_disk::fs::multidisk;

use crate::disk::CmdResult;

/// One disk of a concatenated image, ready to write to the host.
#[derive(Debug)]
pub struct Disk {
    /// Suggested file name, e.g. `Aleste2 (Disk 1).dsk`.
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Plan the per-disk files for a concatenated image, naming them after `stem`
/// (the input's file name without its extension).
///
/// Fails when the image is not a concatenation of whole disks, so a plain
/// single-disk image is never silently "split" into a copy of itself.
pub fn plan(data: &[u8], stem: &str) -> CmdResult<Vec<Disk>> {
    let slices = multidisk::split(data)
        .ok_or_else(|| "not a multi-disk image: it holds a single disk".to_string())?;
    slices
        .iter()
        .map(|slice| {
            let start = slice.lba_start * 512;
            let bytes = data
                .get(start..start + slice.byte_len())
                .ok_or_else(|| format!("disk {} runs past the end of the image", slice.index + 1))?
                .to_vec();
            Ok(Disk {
                name: format!("{stem} (Disk {}).dsk", slice.index + 1),
                bytes,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;
    use msx_disk::image::geometry::SIZE_720K;

    /// Three blank 720 kB disks back to back.
    fn three_disks() -> Vec<u8> {
        testdisk::blank().repeat(3)
    }

    #[test]
    fn plans_one_file_per_disk() {
        let disks = plan(&three_disks(), "Aleste2").expect("split");
        assert_eq!(disks.len(), 3);
        assert_eq!(disks[0].name, "Aleste2 (Disk 1).dsk");
        assert_eq!(disks[2].name, "Aleste2 (Disk 3).dsk");
        assert!(disks.iter().all(|d| d.bytes.len() == SIZE_720K));
    }

    /// Each output must be the disk itself, starting at its own boot sector.
    #[test]
    fn each_file_is_a_standalone_disk_image() {
        let source = three_disks();
        let disks = plan(&source, "x").expect("split");
        for (i, disk) in disks.iter().enumerate() {
            assert_eq!(disk.bytes, source[i * SIZE_720K..(i + 1) * SIZE_720K]);
            // A real boot sector: 512-byte sectors in the BPB.
            assert_eq!(u16::from_le_bytes([disk.bytes[11], disk.bytes[12]]), 512);
        }
    }

    #[test]
    fn a_single_disk_image_is_not_splittable() {
        let err = plan(&testdisk::blank(), "x").unwrap_err();
        assert!(err.contains("single disk"), "{err}");
    }
}
