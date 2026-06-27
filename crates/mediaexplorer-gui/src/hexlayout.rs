//! Pure column geometry for the hex view, in monospace character columns.
//!
//! A single [`HexLayout`] is the source of truth for both painting (where each
//! address/hex/ASCII glyph goes) and hit-testing (which byte a click landed
//! on), so the two can never drift apart. It is independent of egui and works
//! in integer character columns; the renderer multiplies by the glyph advance.
//!
//! Layout, left to right (each region optional, per [`HexViewOptions`]):
//! `<address gutter><2 trailing spaces><hex cells><1 gap><ascii gutter>`.
//! Each hex byte is two digits with no internal padding; byte grouping inserts
//! a single space between every group of N bytes (ungrouped prints them
//! contiguously, `Of(1)` yields the classic one-space-per-byte dump).

use crate::settings::HexViewOptions;

/// Which column of the hex dump a gesture landed in. A selection remembers this
/// so the keyboard copy reproduces what the user pointed at.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum HexRegion {
    /// The two-digit hex byte columns.
    #[default]
    Hex,
    /// The ASCII gutter on the right.
    Ascii,
}

/// Minimum address-gutter digit count, preserving the classic 6-digit look.
const MIN_ADDR_DIGITS: usize = 6;

/// Character columns occupied by a hex byte's two digits.
const HEX_DIGITS: usize = 2;

/// Geometry for one hex view, derived from the display options and row width.
pub struct HexLayout {
    bpr: usize,
    show_line_numbers: bool,
    addr_hex: bool,
    /// Digits in the address gutter (without the 2-space pad).
    addr_digits: usize,
    show_hex: bool,
    show_ascii: bool,
    /// Byte-grouping size, or `None` when ungrouped.
    group: Option<usize>,
    hex_start: usize,
    ascii_start: usize,
    total_cols: usize,
}

impl HexLayout {
    /// Build the layout for `opts`, `bytes_per_row`, and the largest address
    /// that will be shown (used to size the gutter).
    pub fn new(opts: &HexViewOptions, bytes_per_row: usize, max_addr: usize) -> Self {
        let bpr = bytes_per_row.max(1);
        let addr_digits = if opts.line_number_hex {
            format!("{max_addr:X}").len().max(MIN_ADDR_DIGITS)
        } else {
            format!("{max_addr}").len().max(MIN_ADDR_DIGITS)
        };
        let gutter_width = if opts.show_line_numbers {
            addr_digits + 2
        } else {
            0
        };
        let group = opts.grouping.size();
        let hex_start = gutter_width;
        // Width of the hex area: bpr two-digit bytes plus one separator before
        // every group boundary that precedes the last byte.
        let hex_width = if opts.show_hex {
            let seps = group.map_or(0, |n| (bpr - 1) / n);
            bpr * HEX_DIGITS + seps
        } else {
            0
        };
        let ascii_start = if opts.show_hex {
            hex_start + hex_width + 1
        } else {
            hex_start
        };
        let total_cols = if opts.show_ascii {
            ascii_start + bpr
        } else if opts.show_hex {
            hex_start + hex_width
        } else {
            gutter_width
        };
        HexLayout {
            bpr,
            show_line_numbers: opts.show_line_numbers,
            addr_hex: opts.line_number_hex,
            addr_digits,
            show_hex: opts.show_hex,
            show_ascii: opts.show_ascii,
            group,
            hex_start,
            ascii_start,
            total_cols,
        }
    }

    /// Total width of a row in character columns.
    pub fn total_cols(&self) -> usize {
        self.total_cols
    }

    pub fn show_line_numbers(&self) -> bool {
        self.show_line_numbers
    }

    pub fn show_hex(&self) -> bool {
        self.show_hex
    }

    pub fn show_ascii(&self) -> bool {
        self.show_ascii
    }

    /// Number of group separators inserted before hex cell `col`.
    fn seps_before(&self, col: usize) -> usize {
        self.group.map_or(0, |n| col / n)
    }

    /// Render an address into the gutter's fixed-width form.
    pub fn format_addr(&self, addr: usize) -> String {
        let w = self.addr_digits;
        if self.addr_hex {
            format!("{addr:0w$X}")
        } else {
            format!("{addr:0w$}")
        }
    }

    /// Character column where hex cell `col`'s two digits begin, or `None` when
    /// the hex columns are hidden.
    pub fn hex_cell_col(&self, col: usize) -> Option<usize> {
        self.show_hex
            .then(|| self.hex_start + col * HEX_DIGITS + self.seps_before(col))
    }

    /// Character column of ASCII cell `col`, or `None` when the gutter is
    /// hidden.
    pub fn ascii_cell_col(&self, col: usize) -> Option<usize> {
        self.show_ascii.then_some(self.ascii_start + col)
    }

    /// Map a character column to the byte column and region it points at.
    /// `None` for the gutter, inter-region gaps, or past the row's columns.
    pub fn byte_at_char(&self, ch: usize) -> Option<(usize, HexRegion)> {
        if self.show_ascii && ch >= self.ascii_start {
            let col = ch - self.ascii_start;
            return (col < self.bpr).then_some((col, HexRegion::Ascii));
        }
        if self.show_hex {
            // Each cell owns its two digits plus any following separator (so a
            // click in the gap maps to the preceding byte). Scan because spacing
            // is non-uniform once grouped.
            for col in 0..self.bpr {
                let start = self.hex_start + col * HEX_DIGITS + self.seps_before(col);
                let end = if col + 1 < self.bpr {
                    self.hex_start + (col + 1) * HEX_DIGITS + self.seps_before(col + 1)
                } else {
                    start + HEX_DIGITS
                };
                if ch >= start && ch < end {
                    return Some((col, HexRegion::Hex));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ByteGrouping;

    fn defaults() -> HexViewOptions {
        HexViewOptions::default()
    }

    #[test]
    fn default_group_of_one_spaces_each_byte() {
        // bpr=16, hex addresses, all regions on, default grouping (Of(1)):
        // "{:06X}  " + "AA BB .." + 1 gap + ascii.
        let l = HexLayout::new(&defaults(), 16, 0x100);
        // 6-digit address + 2-space pad = hex column 0 at char 8.
        assert_eq!(l.hex_cell_col(0), Some(8));
        assert_eq!(l.hex_cell_col(1), Some(11)); // 2 digits + 1 space
        assert_eq!(l.hex_cell_col(15), Some(8 + 15 * 3));
        // 16 bytes (32 chars) + 15 separators + 1 gap after the gutter.
        assert_eq!(l.ascii_cell_col(0), Some(8 + 32 + 15 + 1));
        assert_eq!(l.total_cols(), 8 + 32 + 15 + 1 + 16);
        assert_eq!(l.format_addr(0), "000000");
        assert_eq!(l.format_addr(0x1234), "001234");
    }

    #[test]
    fn ungrouped_packs_bytes_contiguously() {
        let opts = HexViewOptions {
            grouping: ByteGrouping::None,
            ..defaults()
        };
        let l = HexLayout::new(&opts, 16, 0);
        assert_eq!(l.hex_cell_col(0), Some(8));
        assert_eq!(l.hex_cell_col(1), Some(10)); // no space between bytes
        assert_eq!(l.hex_cell_col(2), Some(12));
        // Every char in the hex area lands on a byte (no gaps).
        assert_eq!(l.byte_at_char(10), Some((1, HexRegion::Hex)));
        assert_eq!(l.byte_at_char(11), Some((1, HexRegion::Hex)));
        assert_eq!(l.ascii_cell_col(0), Some(8 + 32 + 1));
    }

    #[test]
    fn click_round_trips_to_byte_in_default_layout() {
        let l = HexLayout::new(&defaults(), 16, 0);
        for col in 0..16 {
            let hx = l.hex_cell_col(col).unwrap();
            assert_eq!(l.byte_at_char(hx), Some((col, HexRegion::Hex)));
            assert_eq!(l.byte_at_char(hx + 1), Some((col, HexRegion::Hex)));
            // The separating space resolves to the preceding byte (not the last).
            if col < 15 {
                assert_eq!(l.byte_at_char(hx + 2), Some((col, HexRegion::Hex)));
            }
            let ax = l.ascii_cell_col(col).unwrap();
            assert_eq!(l.byte_at_char(ax), Some((col, HexRegion::Ascii)));
        }
        // The gutter and the hex/ascii gap are not clickable.
        assert_eq!(l.byte_at_char(0), None);
        assert_eq!(l.byte_at_char(8 + 32 + 15), None);
    }

    #[test]
    fn hidden_line_numbers_remove_the_gutter() {
        let opts = HexViewOptions {
            show_line_numbers: false,
            ..defaults()
        };
        let l = HexLayout::new(&opts, 16, 0);
        assert!(!l.show_line_numbers());
        assert_eq!(l.hex_cell_col(0), Some(0));
    }

    #[test]
    fn hidden_hex_puts_ascii_after_the_gutter() {
        let opts = HexViewOptions {
            show_hex: false,
            ..defaults()
        };
        let l = HexLayout::new(&opts, 16, 0);
        assert_eq!(l.hex_cell_col(0), None);
        assert_eq!(l.ascii_cell_col(0), Some(8));
        assert_eq!(l.byte_at_char(8), Some((0, HexRegion::Ascii)));
        assert_eq!(l.total_cols(), 8 + 16);
    }

    #[test]
    fn hidden_ascii_drops_the_gutter_and_rejects_clicks_past_hex() {
        let opts = HexViewOptions {
            show_ascii: false,
            ..defaults()
        };
        let l = HexLayout::new(&opts, 16, 0);
        assert_eq!(l.ascii_cell_col(0), None);
        // 16 bytes (32 chars) + 15 separators, no gutter gap, no ascii.
        assert_eq!(l.total_cols(), 8 + 32 + 15);
        assert_eq!(l.byte_at_char(8 + 32 + 15 + 5), None);
    }

    #[test]
    fn grouping_clusters_bytes_and_still_round_trips() {
        let opts = HexViewOptions {
            grouping: ByteGrouping::Of(4),
            ..defaults()
        };
        let l = HexLayout::new(&opts, 16, 0);
        // One separator before col 4, two before col 8, three before col 12.
        assert_eq!(l.hex_cell_col(4), Some(8 + 4 * 2 + 1));
        assert_eq!(l.hex_cell_col(8), Some(8 + 8 * 2 + 2));
        assert_eq!(l.hex_cell_col(12), Some(8 + 12 * 2 + 3));
        for col in 0..16 {
            let hx = l.hex_cell_col(col).unwrap();
            assert_eq!(l.byte_at_char(hx), Some((col, HexRegion::Hex)));
        }
        // ascii sits after the hex area (32 byte chars + 3 separators + gutter gap).
        assert_eq!(l.ascii_cell_col(0), Some(8 + 32 + 3 + 1));
    }

    #[test]
    fn decimal_addresses_size_and_format_the_gutter() {
        let opts = HexViewOptions {
            line_number_hex: false,
            ..defaults()
        };
        let l = HexLayout::new(&opts, 16, 737_280);
        // 737280 needs 6 digits; gutter is digits + 2 spaces → hex col 0 at 8.
        assert_eq!(l.hex_cell_col(0), Some(8));
        assert_eq!(l.format_addr(512), "000512");
        // A larger address widens the gutter beyond the 6-digit minimum.
        let big = HexLayout::new(&opts, 16, 12_345_678);
        assert_eq!(big.format_addr(12_345_678), "12345678");
    }
}
