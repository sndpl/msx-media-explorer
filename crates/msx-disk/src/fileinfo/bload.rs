//! The MSX BSAVE/BLOAD binary header.
//!
//! Files saved with MSX-BASIC's `BSAVE` start with a 7-byte header: a marker
//! byte `0xFE`, then three little-endian 16-bit addresses — start, end, and
//! execution. `BLOAD` uses these to place the data back in memory. Many MSX
//! binaries (machine code, BSAVE'd screens, several music formats) carry it.

/// A decoded BSAVE/BLOAD header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloadHeader {
    /// Load address of the first byte.
    pub start: u16,
    /// Load address of the last byte (inclusive).
    pub end: u16,
    /// Execution address (`0` when the file is data, not code).
    pub exec: u16,
}

impl BloadHeader {
    /// Number of payload bytes the header describes (`end - start + 1`).
    pub fn data_len(&self) -> u32 {
        u32::from(self.end.wrapping_sub(self.start)) + 1
    }
}

/// Parse the 7-byte BSAVE header, or `None` if `bytes` does not start with one.
pub fn parse(bytes: &[u8]) -> Option<BloadHeader> {
    if bytes.first() != Some(&0xFE) || bytes.len() < 7 {
        return None;
    }
    Some(BloadHeader {
        start: u16::from_le_bytes([bytes[1], bytes[2]]),
        end: u16::from_le_bytes([bytes[3], bytes[4]]),
        exec: u16::from_le_bytes([bytes[5], bytes[6]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_bsave_header() {
        // 0xFE, start=0x8000, end=0x9FFF, exec=0x8000
        let bytes = [0xFE, 0x00, 0x80, 0xFF, 0x9F, 0x00, 0x80, 0x11, 0x22];
        let h = parse(&bytes).expect("header");
        assert_eq!(h.start, 0x8000);
        assert_eq!(h.end, 0x9FFF);
        assert_eq!(h.exec, 0x8000);
        assert_eq!(h.data_len(), 0x2000);
    }

    #[test]
    fn rejects_non_bsave() {
        assert!(parse(&[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06]).is_none());
    }

    #[test]
    fn rejects_short_input() {
        assert!(parse(&[0xFE, 0x00, 0x80]).is_none());
    }
}
