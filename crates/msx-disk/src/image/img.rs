//! `.img` — a raw dump prefixed with one leading byte giving the side count.
//!
//! Byte 0 is the number of sides (1 or 2); the remainder is the raw, linear
//! sector data identical to a `.dsk`.

use crate::error::{Error, Result};
use crate::image::geometry::SECTOR_SIZE;

/// Strip the leading side-count byte and return the normalized sector buffer.
///
/// Returns the buffer together with the declared side count.
pub fn normalize(bytes: &[u8]) -> Result<(Vec<u8>, u8)> {
    let Some((&sides, rest)) = bytes.split_first() else {
        return Err(Error::Malformed("img image is empty".into()));
    };
    if rest.is_empty() || !rest.len().is_multiple_of(SECTOR_SIZE) {
        return Err(Error::Malformed(format!(
            "img payload length {} is not a whole number of {SECTOR_SIZE}-byte sectors",
            rest.len()
        )));
    }
    Ok((rest.to_vec(), sides))
}
