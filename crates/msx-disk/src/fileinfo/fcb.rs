//! The MSX-DOS File Control Block (FCB).
//!
//! An FCB is the 37-byte structure MSX-DOS programs build in memory to open a
//! file. It appears in memory dumps and inside some save files, so the hex data
//! inspector decodes it when the bytes at the cursor look like a plausible FCB.
//!
//! Only the human-meaningful leading fields are decoded; the cluster/date/time
//! bookkeeping that follows is left to the raw hex view. Layout:
//!
//! | Offset | Size | Field          |
//! |--------|------|----------------|
//! | 0      | 1    | drive (0=default, 1=A:, ...) |
//! | 1      | 8    | filename       |
//! | 9      | 3    | extension      |
//! | 12     | 2    | current block  |
//! | 14     | 2    | record size    |
//! | 16     | 4    | file size      |

/// The decoded leading fields of an MSX-DOS FCB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FcbHeader {
    /// Drive number: 0 = default, 1 = A:, 2 = B:, ...
    pub drive: u8,
    /// The 8.3 filename, e.g. `GAME.COM` (trailing padding trimmed).
    pub name: String,
    /// Current block number used for sequential I/O.
    pub current_block: u16,
    /// Record size in bytes (commonly 128).
    pub record_size: u16,
    /// File size in bytes.
    pub file_size: u32,
}

/// Parse an FCB from the start of `bytes`, or `None` if the bytes do not look
/// like one. Validation is deliberately conservative — the drive byte must be a
/// plausible drive and the 11 name bytes must all be printable ASCII — so the
/// inspector does not present FCB noise over arbitrary data.
pub fn parse(bytes: &[u8]) -> Option<FcbHeader> {
    if bytes.len() < 20 {
        return None;
    }
    let drive = bytes[0];
    if drive > 8 {
        return None;
    }
    let name_bytes = &bytes[1..12];
    if !name_bytes.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        return None;
    }
    let base: String = bytes[1..9]
        .iter()
        .take_while(|&&b| b != b' ')
        .map(|&b| b as char)
        .collect();
    if base.is_empty() {
        return None; // an all-spaces name is not a plausible FCB
    }
    let ext: String = bytes[9..12]
        .iter()
        .take_while(|&&b| b != b' ')
        .map(|&b| b as char)
        .collect();
    let name = if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    };
    Some(FcbHeader {
        drive,
        name,
        current_block: u16::from_le_bytes([bytes[12], bytes[13]]),
        record_size: u16::from_le_bytes([bytes[14], bytes[15]]),
        file_size: u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut b = vec![0u8; 37];
        b[0] = 1; // drive A:
        b[1..9].copy_from_slice(b"GAME    ");
        b[9..12].copy_from_slice(b"COM");
        b[12..14].copy_from_slice(&3u16.to_le_bytes());
        b[14..16].copy_from_slice(&128u16.to_le_bytes());
        b[16..20].copy_from_slice(&1024u32.to_le_bytes());
        b
    }

    #[test]
    fn parses_a_plausible_fcb() {
        let fcb = parse(&sample()).expect("fcb");
        assert_eq!(fcb.drive, 1);
        assert_eq!(fcb.name, "GAME.COM");
        assert_eq!(fcb.current_block, 3);
        assert_eq!(fcb.record_size, 128);
        assert_eq!(fcb.file_size, 1024);
    }

    #[test]
    fn name_without_extension_has_no_dot() {
        let mut b = sample();
        b[9..12].copy_from_slice(b"   ");
        assert_eq!(parse(&b).unwrap().name, "GAME");
    }

    #[test]
    fn rejects_short_input() {
        assert!(parse(&[1u8; 8]).is_none());
    }

    #[test]
    fn rejects_implausible_drive() {
        let mut b = sample();
        b[0] = 200;
        assert!(parse(&b).is_none());
    }

    #[test]
    fn rejects_non_printable_name() {
        // Random binary data should not be mistaken for an FCB.
        let b = vec![0u8; 37];
        assert!(parse(&b).is_none());
    }
}
