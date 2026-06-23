//! `.ddi` — DiskDupe image: a fixed-size header followed by raw disk data.
//!
//! Sources disagree on the header size (0x1200 per the msxnet FAQ, 0x1800 per
//! MESS/imgtool), so instead of hardcoding we derive it: try each candidate
//! header size and keep the one whose remaining payload is a standard MSX disk
//! size. This is robust to either convention.

use crate::error::{Error, Result};
use crate::image::geometry::{SIZE_360K, SIZE_720K};

/// Candidate header sizes, in priority order.
const HEADER_CANDIDATES: [usize; 2] = [0x1200, 0x1800];

/// Strip the DiskDupe header and return the normalized raw sector buffer.
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>> {
    for &header in &HEADER_CANDIDATES {
        if bytes.len() <= header {
            continue;
        }
        let payload = bytes.len() - header;
        if payload == SIZE_360K || payload == SIZE_720K {
            return Ok(bytes[header..].to_vec());
        }
    }
    Err(Error::Malformed(format!(
        "ddi image size {} matches no known header (0x1200/0x1800) + disk-size combination",
        bytes.len()
    )))
}
