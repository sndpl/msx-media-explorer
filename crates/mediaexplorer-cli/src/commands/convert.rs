//! `convert` — re-container normalized disk data as `.dsk` or `.xsa`.

use msx_disk::image::{xsa, ImageFormat};

use crate::disk::CmdResult;

/// Encode `data` (normalized sectors) into the container `format`.
///
/// Only raw `.dsk`-family output and compressed `.xsa` output are supported;
/// the other containers carry framing this tool does not synthesize from
/// scratch (editing existing ones still works via reencode).
pub fn encode(data: &[u8], format: ImageFormat) -> CmdResult<Vec<u8>> {
    match format {
        ImageFormat::Dsk => Ok(data.to_vec()),
        ImageFormat::Xsa => Ok(xsa::compress(data)),
        other => Err(format!(
            "cannot write {other:?} containers; use a .dsk or .xsa output name"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testdisk;
    use msx_disk::image::DiskImage;

    #[test]
    fn dsk_output_is_the_raw_normalized_data() {
        let disk = testdisk::populated();
        assert_eq!(encode(&disk, ImageFormat::Dsk).unwrap(), disk);
    }

    #[test]
    fn xsa_output_round_trips_through_the_public_reader() {
        let disk = testdisk::populated();
        let compressed = encode(&disk, ImageFormat::Xsa).unwrap();
        assert!(compressed.starts_with(xsa::MAGIC));
        assert!(
            compressed.len() < disk.len(),
            "a mostly-empty disk must shrink"
        );
        let image = DiskImage::open_bytes(ImageFormat::Xsa, compressed).unwrap();
        assert_eq!(image.data(), disk.as_slice(), "byte-identical round trip");
    }

    #[test]
    fn unsupported_output_container_is_an_error() {
        let err = encode(&testdisk::blank(), ImageFormat::Dmk).unwrap_err();
        assert!(err.contains(".dsk or .xsa"), "{err}");
    }
}
