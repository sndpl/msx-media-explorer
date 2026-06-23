//! `.dsk` / `.di1` / `.ds1` / `.di2` / `.ds2` — raw sector dumps.
//!
//! These are already in normalized (linear, logical-sector) order, so
//! "normalizing" is just validation: the length must be a whole number of
//! sectors.

use crate::error::{Error, Result};
use crate::image::geometry::SECTOR_SIZE;

/// Validate a raw dump and return it unchanged as a normalized sector buffer.
pub fn normalize(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(SECTOR_SIZE) {
        return Err(Error::Malformed(format!(
            "raw image length {} is not a whole number of {SECTOR_SIZE}-byte sectors",
            bytes.len()
        )));
    }
    Ok(bytes)
}
