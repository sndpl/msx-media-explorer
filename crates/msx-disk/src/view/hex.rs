//! Hex-dump view model.
//!
//! Produces the classic three-column layout: an address/offset column, the hex
//! bytes, and an ASCII gutter where non-printable bytes render as `.`. The row
//! width is configurable (8/16/24/32) and an optional `base_address` lets BSAVE
//! files show their MSX RAM load address instead of a plain file offset.

/// ASCII rendering of a byte: printable ASCII as-is, everything else as `.`.
pub fn ascii_char(b: u8) -> char {
    if (0x20..=0x7E).contains(&b) {
        b as char
    } else {
        '.'
    }
}

/// Layout options for a hex dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexConfig {
    /// Bytes shown per row (clamped to at least 1).
    pub bytes_per_row: usize,
    /// Value added to each row's offset to form its displayed address.
    pub base_address: usize,
}

impl Default for HexConfig {
    fn default() -> Self {
        HexConfig {
            bytes_per_row: 16,
            base_address: 0,
        }
    }
}

/// One rendered row of a hex dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    /// Displayed address (`base_address + offset`).
    pub address: usize,
    /// Byte offset of the row's first byte within the data.
    pub offset: usize,
    /// The raw bytes in this row (may be shorter than `bytes_per_row`).
    pub bytes: Vec<u8>,
    /// ASCII gutter for the row.
    pub ascii: String,
}

/// Iterate the hex rows of `data`.
pub fn rows(data: &[u8], cfg: HexConfig) -> impl Iterator<Item = HexRow> + '_ {
    let bpr = cfg.bytes_per_row.max(1);
    data.chunks(bpr).enumerate().map(move |(i, chunk)| {
        let offset = i * bpr;
        HexRow {
            address: cfg.base_address + offset,
            offset,
            bytes: chunk.to_vec(),
            ascii: chunk.iter().map(|&b| ascii_char(b)).collect(),
        }
    })
}

/// Render a full hex dump to a string (used for clipboard export).
pub fn dump_to_string(data: &[u8], cfg: HexConfig) -> String {
    use std::fmt::Write;
    let bpr = cfg.bytes_per_row.max(1);
    let mut out = String::new();
    for row in rows(data, cfg) {
        let _ = write!(out, "{:06X}  ", row.address);
        for col in 0..bpr {
            match row.bytes.get(col) {
                Some(b) => {
                    let _ = write!(out, "{b:02X} ");
                }
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        out.push_str(&row.ascii);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_char_replaces_non_printable() {
        assert_eq!(ascii_char(b'A'), 'A');
        assert_eq!(ascii_char(0x00), '.');
        assert_eq!(ascii_char(0x1F), '.');
        assert_eq!(ascii_char(0x7F), '.');
        assert_eq!(ascii_char(0xFF), '.');
    }

    #[test]
    fn rows_split_and_label_correctly() {
        let data: Vec<u8> = (0..20).collect();
        let cfg = HexConfig {
            bytes_per_row: 8,
            base_address: 0x100,
        };
        let rows: Vec<_> = rows(&data, cfg).collect();
        assert_eq!(rows.len(), 3); // 8 + 8 + 4
        assert_eq!(rows[0].address, 0x100);
        assert_eq!(rows[1].offset, 8);
        assert_eq!(rows[2].bytes.len(), 4);
    }

    #[test]
    fn dump_to_string_formats_first_row() {
        let data = b"ABC";
        let cfg = HexConfig {
            bytes_per_row: 16,
            base_address: 0,
        };
        let dump = dump_to_string(data, cfg);
        let first = dump.lines().next().unwrap();
        assert!(first.starts_with("000000  41 42 43 "));
        assert!(first.trim_end().ends_with("ABC"));
    }
}
