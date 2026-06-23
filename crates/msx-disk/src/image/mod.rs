//! Disk-image container layer.
//!
//! Every supported container is read into a *normalized* representation: a flat
//! byte buffer of all sectors in logical (LBA) order, exactly like a plain
//! `.dsk`. The filesystem layer and viewers only ever see this normalized form,
//! so they never need to know which on-disk container they came from.

mod ddi;
pub mod dmk;
mod dsk;
pub mod geometry;
mod img;
mod msx;
pub mod xsa;

use std::path::Path;

use crate::error::{Error, Result};
use geometry::{Geometry, SECTOR_SIZE};

/// A recognized disk-image container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// Raw sector dump: `.dsk`, `.di1`, `.ds1`, `.di2`, `.ds2`.
    Dsk,
    /// Raw dump with a leading side-count byte: `.img`.
    Img,
    /// 720KB image with documented interleave: `.msx`.
    Msx,
    /// DiskDupe image with a fixed header: `.ddi`.
    Ddi,
    /// Compressed XelaSoft Archive: `.xsa`.
    Xsa,
    /// David Keil raw-track image: `.dmk`.
    Dmk,
}

impl ImageFormat {
    /// Map a file extension (without the dot, any case) to a format.
    pub fn from_extension(ext: &str) -> Option<ImageFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "dsk" | "di1" | "ds1" | "di2" | "ds2" => Some(ImageFormat::Dsk),
            "img" => Some(ImageFormat::Img),
            "msx" => Some(ImageFormat::Msx),
            "ddi" => Some(ImageFormat::Ddi),
            "xsa" => Some(ImageFormat::Xsa),
            "dmk" => Some(ImageFormat::Dmk),
            _ => None,
        }
    }
}

/// Detect a format from leading magic bytes, for files with an unknown or
/// missing extension.
fn detect_by_magic(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(xsa::MAGIC) {
        Some(ImageFormat::Xsa)
    } else if bytes.starts_with(b"IM") {
        Some(ImageFormat::Ddi)
    } else {
        None
    }
}

/// A disk image normalized to linear sector order.
#[derive(Debug, Clone)]
pub struct DiskImage {
    format: ImageFormat,
    geometry: Geometry,
    /// Container bytes that precede the normalized data (e.g. the `.img`
    /// side-count byte or the `.ddi` header); empty for raw `.dsk`/`.msx`.
    /// Preserved so modified data can be written back in the same container.
    prefix: Vec<u8>,
    data: Vec<u8>,
}

impl DiskImage {
    /// The container format this image was read from.
    pub fn format(&self) -> ImageFormat {
        self.format
    }

    /// The disk geometry.
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// The normalized sector data in logical order.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Consume the image, yielding its normalized sector data.
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Number of 512-byte sectors.
    pub fn sector_count(&self) -> usize {
        self.data.len() / SECTOR_SIZE
    }

    /// Borrow a single sector by index, if in range.
    pub fn sector(&self, index: usize) -> Option<&[u8]> {
        let start = index.checked_mul(SECTOR_SIZE)?;
        self.data.get(start..start + SECTOR_SIZE)
    }

    /// Open a disk image from a path, detecting the format from its extension
    /// (falling back to magic-byte sniffing).
    pub fn open(path: impl AsRef<Path>) -> Result<DiskImage> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageFormat::from_extension)
            .or_else(|| detect_by_magic(&bytes))
            .ok_or(Error::UnknownFormat)?;
        DiskImage::open_bytes(format, bytes)
    }

    /// Build a normalized image from in-memory bytes for a known format.
    pub fn open_bytes(format: ImageFormat, bytes: Vec<u8>) -> Result<DiskImage> {
        let (prefix, data) = match format {
            ImageFormat::Dsk => (Vec::new(), dsk::normalize(bytes)?),
            ImageFormat::Img => {
                let prefix = bytes[..1.min(bytes.len())].to_vec();
                (prefix, img::normalize(&bytes)?.0)
            }
            ImageFormat::Msx => (Vec::new(), msx::normalize(bytes)?),
            ImageFormat::Ddi => {
                let data = ddi::normalize(&bytes)?;
                let header_len = bytes.len() - data.len();
                (bytes[..header_len].to_vec(), data)
            }
            ImageFormat::Xsa => (Vec::new(), xsa::decompress(&bytes)?),
            ImageFormat::Dmk => (Vec::new(), dmk::normalize(&bytes)?),
        };
        let geometry = Geometry::for_raw_len(data.len())
            .ok_or_else(|| Error::Malformed("normalized image is not sector-aligned".into()))?;
        Ok(DiskImage {
            format,
            geometry,
            prefix,
            data,
        })
    }

    /// Re-encode modified normalized sector data back into this image's
    /// container framing, ready to write to disk.
    ///
    /// Returns an error for `.xsa`, which would require re-compression
    /// (planned for a later phase); save those as `.dsk` instead.
    pub fn reencode(&self, data: &[u8]) -> Result<Vec<u8>> {
        if !self.is_writable() {
            return Err(Error::Unsupported(format!(
                "cannot write back to {:?}; save as .dsk instead",
                self.format
            )));
        }
        let mut out = Vec::with_capacity(self.prefix.len() + data.len());
        out.extend_from_slice(&self.prefix);
        out.extend_from_slice(data);
        Ok(out)
    }

    /// Whether modified data can be written back to this image's container.
    /// Compressed (`.xsa`) and raw-track (`.dmk`) containers are read-only;
    /// save them as `.dsk` to edit.
    pub fn is_writable(&self) -> bool {
        !matches!(self.format, ImageFormat::Xsa | ImageFormat::Dmk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geometry::{SIZE_360K, SIZE_720K};

    /// A 720KB buffer where each byte encodes its sector index, so any
    /// reordering or truncation is detectable.
    fn synthetic_raw(size: usize) -> Vec<u8> {
        (0..size)
            .map(|i| ((i / SECTOR_SIZE) & 0xFF) as u8)
            .collect()
    }

    #[test]
    fn extension_mapping_covers_all_raw_variants() {
        for ext in ["dsk", "DSK", "di1", "ds1", "di2", "ds2"] {
            assert_eq!(ImageFormat::from_extension(ext), Some(ImageFormat::Dsk));
        }
        assert_eq!(ImageFormat::from_extension("xsa"), Some(ImageFormat::Xsa));
        assert_eq!(ImageFormat::from_extension("zip"), None);
    }

    #[test]
    fn dsk_passthrough_720k() {
        let raw = synthetic_raw(SIZE_720K);
        let img = DiskImage::open_bytes(ImageFormat::Dsk, raw.clone()).unwrap();
        assert_eq!(img.data(), &raw[..]);
        assert_eq!(img.geometry(), Geometry::DS_720K);
        assert_eq!(img.sector_count(), 1440);
    }

    #[test]
    fn dsk_360k_geometry() {
        let img = DiskImage::open_bytes(ImageFormat::Dsk, synthetic_raw(SIZE_360K)).unwrap();
        assert_eq!(img.geometry(), Geometry::SS_360K);
    }

    #[test]
    fn dsk_rejects_non_sector_multiple() {
        assert!(DiskImage::open_bytes(ImageFormat::Dsk, vec![0u8; 513]).is_err());
    }

    #[test]
    fn img_strips_leading_side_byte() {
        let raw = synthetic_raw(SIZE_720K);
        let mut bytes = vec![0x02];
        bytes.extend_from_slice(&raw);
        let img = DiskImage::open_bytes(ImageFormat::Img, bytes).unwrap();
        assert_eq!(img.data(), &raw[..]);
    }

    #[test]
    fn ddi_strips_header_either_size() {
        let raw = synthetic_raw(SIZE_720K);
        for header in [0x1200usize, 0x1800] {
            let mut bytes = vec![0u8; header];
            bytes.extend_from_slice(&raw);
            let img = DiskImage::open_bytes(ImageFormat::Ddi, bytes).unwrap();
            assert_eq!(img.data(), &raw[..], "header 0x{header:X}");
        }
    }

    #[test]
    fn msx_requires_720k() {
        assert!(DiskImage::open_bytes(ImageFormat::Msx, synthetic_raw(SIZE_720K)).is_ok());
        assert!(DiskImage::open_bytes(ImageFormat::Msx, synthetic_raw(SIZE_360K)).is_err());
    }

    #[test]
    fn magic_detects_xsa_and_ddi() {
        assert_eq!(detect_by_magic(xsa::MAGIC), Some(ImageFormat::Xsa));
        assert_eq!(detect_by_magic(b"IMxx"), Some(ImageFormat::Ddi));
        assert_eq!(detect_by_magic(b"random"), None);
    }

    #[test]
    fn reencode_dsk_is_identity() {
        let raw = synthetic_raw(SIZE_720K);
        let img = DiskImage::open_bytes(ImageFormat::Dsk, raw.clone()).unwrap();
        assert_eq!(img.reencode(&raw).unwrap(), raw);
    }

    #[test]
    fn reencode_img_restores_side_byte() {
        let raw = synthetic_raw(SIZE_720K);
        let mut bytes = vec![0x02];
        bytes.extend_from_slice(&raw);
        let img = DiskImage::open_bytes(ImageFormat::Img, bytes.clone()).unwrap();
        assert_eq!(img.reencode(img.data()).unwrap(), bytes);
    }

    #[test]
    fn reencode_ddi_restores_header() {
        let raw = synthetic_raw(SIZE_720K);
        let mut bytes = vec![0u8; 0x1200];
        bytes.extend_from_slice(&raw);
        let img = DiskImage::open_bytes(ImageFormat::Ddi, bytes.clone()).unwrap();
        assert_eq!(img.reencode(img.data()).unwrap(), bytes);
    }

    #[test]
    fn reencode_xsa_is_unsupported() {
        let img = DiskImage {
            format: ImageFormat::Xsa,
            geometry: Geometry::DS_720K,
            prefix: Vec::new(),
            data: vec![0u8; SIZE_720K],
        };
        assert!(!img.is_writable());
        assert!(img.reencode(img.data()).is_err());
    }
}
