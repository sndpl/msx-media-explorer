//! BSAVE binary-header parsing.
//!
//! A BSAVEd file begins with a 7-byte header: `0xFE`, then the start, end, and
//! execution addresses as little-endian 16-bit words. The payload that follows
//! is `end - start + 1` bytes. For screen dumps the payload is a raw VRAM copy.

/// Length of the BSAVE header in bytes.
pub const HEADER_LEN: usize = 7;

/// A parsed BSAVE header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BsaveHeader {
    pub start: u16,
    pub end: u16,
    pub exec: u16,
}

impl BsaveHeader {
    /// Declared payload length (`end - start + 1`), saturating if malformed.
    pub fn data_len(&self) -> usize {
        (self.end as usize).saturating_sub(self.start as usize) + 1
    }
}

/// Parse a BSAVE header if the data begins with one.
pub fn parse(bytes: &[u8]) -> Option<BsaveHeader> {
    if bytes.len() < HEADER_LEN || bytes[0] != 0xFE {
        return None;
    }
    Some(BsaveHeader {
        start: u16::from_le_bytes([bytes[1], bytes[2]]),
        end: u16::from_le_bytes([bytes[3], bytes[4]]),
        exec: u16::from_le_bytes([bytes[5], bytes[6]]),
    })
}

/// The VRAM payload: bytes after the BSAVE header if present, otherwise the
/// whole input (some screen dumps are stored as raw VRAM with no header).
pub fn payload(bytes: &[u8]) -> &[u8] {
    if parse(bytes).is_some() {
        &bytes[HEADER_LEN..]
    } else {
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_and_strips_payload() {
        // 0xFE, start=0x0000, end=0x0002, exec=0x0000, then 3 payload bytes.
        let bytes = [0xFE, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC];
        let h = parse(&bytes).unwrap();
        assert_eq!(h.start, 0);
        assert_eq!(h.end, 2);
        assert_eq!(h.data_len(), 3);
        assert_eq!(payload(&bytes), &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn no_header_returns_whole_input() {
        let bytes = [0x00, 0x11, 0x22];
        assert!(parse(&bytes).is_none());
        assert_eq!(payload(&bytes), &bytes);
    }
}
