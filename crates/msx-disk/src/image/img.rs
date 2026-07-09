//! `.img` — a raw MSX disk dump, in one of two layouts.
//!
//! - **Leading side-count byte**: byte 0 holds the number of sides (1 or 2) and
//!   the remainder is raw, linear sector data identical to a `.dsk`. Such a
//!   file is always `1 + N*512` bytes long.
//! - **Plain raw dump**: no prefix at all — the whole file is linear sector
//!   data, exactly like a `.dsk` (e.g. a bare FAT12/FAT16 partition pulled off
//!   an MMC/SD card, whose sector 0 is the boot sector). Such a file is `N*512`
//!   bytes long.
//!
//! The two are told apart by length: a side-byte image is one byte past a
//! sector boundary, a raw dump lands exactly on one. Disambiguating this way
//! avoids mistaking a raw dump's first byte — the boot sector's `EB`/`E9` jump —
//! for a side count and shifting every subsequent sector by one byte.

use crate::error::{Error, Result};
use crate::image::geometry::SECTOR_SIZE;

/// Split an `.img` into its container prefix and normalized sector buffer.
///
/// Returns `(prefix, data)`: `prefix` is the single leading side-count byte for
/// the prefixed layout, or empty for a plain raw dump; it is preserved so
/// `reencode` can write the image back in the same framing. `data` is always a
/// whole number of 512-byte sectors in logical order.
pub fn normalize(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    if bytes.is_empty() {
        return Err(Error::Malformed("img image is empty".into()));
    }
    match bytes.len() % SECTOR_SIZE {
        // Exact sector multiple: a plain raw dump, nothing to strip.
        0 => Ok((Vec::new(), bytes.to_vec())),
        // One byte past a sector boundary: the leading side-count byte layout.
        1 => {
            let (prefix, rest) = bytes.split_at(1);
            Ok((prefix.to_vec(), rest.to_vec()))
        }
        remainder => Err(Error::Malformed(format!(
            "img length {} is neither a whole number of {SECTOR_SIZE}-byte sectors \
             nor one side-count byte past one (trailing {remainder} bytes)",
            bytes.len()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_byte_layout_splits_off_the_prefix() {
        let mut bytes = vec![0x02];
        bytes.extend_from_slice(&[0u8; SECTOR_SIZE]);
        let (prefix, data) = normalize(&bytes).unwrap();
        assert_eq!(prefix, vec![0x02]);
        assert_eq!(data.len(), SECTOR_SIZE);
    }

    #[test]
    fn raw_dump_layout_keeps_every_byte() {
        // Sector 0 begins with a boot-sector `EB` jump, not a side count.
        let mut bytes = vec![0u8; SECTOR_SIZE * 3];
        bytes[0] = 0xEB;
        let (prefix, data) = normalize(&bytes).unwrap();
        assert!(prefix.is_empty());
        assert_eq!(data, bytes);
    }

    #[test]
    fn rejects_length_that_is_neither_layout() {
        // Two bytes past a sector boundary: not a raw dump, not one side byte.
        assert!(normalize(&[0u8; SECTOR_SIZE + 2]).is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(normalize(&[]).is_err());
    }
}
