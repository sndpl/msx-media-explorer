//! `.msx` — always a 720KB double-sided image.
//!
//! Documented sector order: "the first 9 sectors of side 0, then the first 9
//! sectors of side 1, then the next 9 of side 0, the next 9 of side 1, ...".
//! That is cylinder-interleaved-by-head, which is identical to the standard
//! logical (LBA) ordering of a plain `.dsk`, so normalizing is currently just
//! validation that the payload is the expected 720KB.
//!
//! PROVISIONAL: this format is rare and could not be validated against a real
//! sample. If a genuine `.msx` image turns out to use a side-major order (all
//! of side 0 followed by all of side 1), this is where the de-interleave would
//! be added — convert physical order to LBA before returning.

use crate::error::{Error, Result};
use crate::image::geometry::SIZE_720K;

/// Validate a `.msx` image and return its normalized 720KB sector buffer.
pub fn normalize(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.len() != SIZE_720K {
        return Err(Error::Malformed(format!(
            "msx image is {} bytes, expected {SIZE_720K} (720KB)",
            bytes.len()
        )));
    }
    Ok(bytes)
}
