//! Hex-dump view model.
//!
//! Produces the classic three-column layout: an address/offset column, the hex
//! bytes, and a text gutter. Printable ASCII renders as-is, control bytes as
//! `.`, and bytes `>= 0x80` decode to real glyphs through the selected MSX
//! [`charset`](crate::charset). The row width is configurable (8/16/24/32) and
//! an optional `base_address` lets BSAVE files show their MSX RAM load address
//! instead of a plain file offset.

use crate::charset::{self, MsxCharset};

/// Gutter rendering of a byte under `charset`: printable ASCII as-is, control
/// bytes as `.`, and high bytes decoded to their MSX glyph.
pub fn ascii_char(b: u8, charset: MsxCharset) -> char {
    match b {
        0x20..=0x7E => b as char,
        0x80..=0xFF => charset::decode_byte(charset, b),
        _ => '.',
    }
}

/// Layout options for a hex dump. The display toggles mirror the GUI's View
/// menu so exported (clipboard) dumps match what is shown on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexConfig {
    /// Bytes shown per row (clamped to at least 1).
    pub bytes_per_row: usize,
    /// Value added to each row's offset to form its displayed address.
    pub base_address: usize,
    /// Show the address gutter.
    pub show_line_numbers: bool,
    /// Address gutter base: `true` = hexadecimal, `false` = decimal.
    pub line_number_hex: bool,
    /// Show the hexadecimal byte columns.
    pub show_hex: bool,
    /// Show the plain-text (ASCII) gutter.
    pub show_ascii: bool,
    /// Bytes per group: a single space separates groups, `None` packs bytes
    /// contiguously, and `Some(1)` is the classic one-space-per-byte dump.
    pub grouping: Option<usize>,
    /// Render `0x00` bytes as blanks in the hex and ASCII columns.
    pub hide_null_bytes: bool,
}

impl Default for HexConfig {
    fn default() -> Self {
        HexConfig {
            bytes_per_row: 16,
            base_address: 0,
            show_line_numbers: true,
            line_number_hex: true,
            show_hex: true,
            show_ascii: true,
            grouping: Some(1),
            hide_null_bytes: false,
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

/// Iterate the hex rows of `data`, decoding the gutter under `charset`.
pub fn rows(data: &[u8], cfg: HexConfig, charset: MsxCharset) -> impl Iterator<Item = HexRow> + '_ {
    let bpr = cfg.bytes_per_row.max(1);
    data.chunks(bpr).enumerate().map(move |(i, chunk)| {
        let offset = i * bpr;
        HexRow {
            address: cfg.base_address + offset,
            offset,
            bytes: chunk.to_vec(),
            ascii: chunk.iter().map(|&b| ascii_char(b, charset)).collect(),
        }
    })
}

/// Render a full hex dump to a string (used for clipboard export). Honors the
/// display toggles in `cfg` so the export matches the on-screen view.
pub fn dump_to_string(data: &[u8], cfg: HexConfig, charset: MsxCharset) -> String {
    use std::fmt::Write;
    let bpr = cfg.bytes_per_row.max(1);
    let group = cfg.grouping.filter(|&n| n >= 1);
    // ~3 hex chars + 1 ASCII char per byte, plus per-row line-number/newline.
    let mut out = String::with_capacity(data.len() * 4 + 64);
    for row in rows(data, cfg, charset) {
        if cfg.show_line_numbers {
            if cfg.line_number_hex {
                let _ = write!(out, "{:06X}  ", row.address);
            } else {
                let _ = write!(out, "{:06}  ", row.address);
            }
        }
        if cfg.show_hex {
            for col in 0..bpr {
                // A single space precedes each group boundary (ungrouped: none).
                if col > 0 && group.is_some_and(|n| col % n == 0) {
                    out.push(' ');
                }
                match row.bytes.get(col) {
                    Some(&b) if cfg.hide_null_bytes && b == 0 => out.push_str("  "),
                    Some(&b) => {
                        let _ = write!(out, "{b:02X}");
                    }
                    None => out.push_str("  "),
                }
            }
        }
        if cfg.show_ascii {
            if cfg.show_hex {
                out.push(' ');
            }
            for &b in &row.bytes {
                if cfg.hide_null_bytes && b == 0 {
                    out.push(' ');
                } else {
                    out.push(ascii_char(b, charset));
                }
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTL: MsxCharset = MsxCharset::International;

    #[test]
    fn control_bytes_become_dots() {
        assert_eq!(ascii_char(b'A', INTL), 'A');
        assert_eq!(ascii_char(0x00, INTL), '.');
        assert_eq!(ascii_char(0x1F, INTL), '.');
        assert_eq!(ascii_char(0x7F, INTL), '.');
    }

    #[test]
    fn high_bytes_decode_to_glyphs_in_gutter() {
        // The gutter now shows real glyphs for high bytes instead of `.`.
        assert_eq!(ascii_char(0xB1, MsxCharset::Japanese), '\u{FF71}');
        assert_eq!(ascii_char(0x82, INTL), '\u{00E9}');
    }

    #[test]
    fn rows_split_and_label_correctly() {
        let data: Vec<u8> = (0..20).collect();
        let cfg = HexConfig {
            bytes_per_row: 8,
            base_address: 0x100,
            ..HexConfig::default()
        };
        let rows: Vec<_> = rows(&data, cfg, INTL).collect();
        assert_eq!(rows.len(), 3); // 8 + 8 + 4
        assert_eq!(rows[0].address, 0x100);
        assert_eq!(rows[1].offset, 8);
        assert_eq!(rows[2].bytes.len(), 4);
    }

    #[test]
    fn dump_to_string_formats_first_row() {
        let data = b"ABC";
        let cfg = HexConfig::default();
        let dump = dump_to_string(data, cfg, INTL);
        let first = dump.lines().next().unwrap();
        assert!(first.starts_with("000000  41 42 43 "));
        assert!(first.trim_end().ends_with("ABC"));
    }

    #[test]
    fn dump_honors_hidden_regions() {
        // Hex only: no gutter, no ascii.
        let cfg = HexConfig {
            bytes_per_row: 16,
            show_line_numbers: false,
            show_ascii: false,
            ..HexConfig::default()
        };
        let first = dump_to_string(b"AB", cfg, INTL)
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert_eq!(first.trim_end(), "41 42");
    }

    #[test]
    fn dump_decimal_addresses() {
        let cfg = HexConfig {
            line_number_hex: false,
            ..HexConfig::default()
        };
        let first = dump_to_string(b"A", cfg, INTL)
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert!(first.starts_with("000000  41 "));
    }

    #[test]
    fn dump_groups_and_hides_nulls() {
        let cfg = HexConfig {
            bytes_per_row: 8,
            grouping: Some(4),
            hide_null_bytes: true,
            show_ascii: false,
            show_line_numbers: false,
            ..HexConfig::default()
        };
        let data = [0x11u8, 0x00, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        let first = dump_to_string(&data, cfg, INTL)
            .lines()
            .next()
            .unwrap()
            .to_string();
        // Groups of 4 contiguous bytes, one space between groups; null blanked.
        assert_eq!(first, "11  2233 44556677");
    }

    #[test]
    fn dump_ungrouped_packs_contiguously() {
        let cfg = HexConfig {
            grouping: None,
            show_ascii: false,
            show_line_numbers: false,
            ..HexConfig::default()
        };
        let first = dump_to_string(&[0xAA, 0x01, 0xFF], cfg, INTL)
            .lines()
            .next()
            .unwrap()
            .trim_end()
            .to_string();
        assert_eq!(first, "AA01FF");
    }
}
