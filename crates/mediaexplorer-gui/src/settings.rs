//! Persisted application settings: the recent-files list and the hex-view
//! display options driven by the View menu. Stored via eframe's storage
//! (`eframe::{get_value, set_value}`) so they survive across runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum number of entries kept in the recent-files list.
pub const MAX_RECENT: usize = 10;

/// Selectable bytes-per-row sizes offered by the View menu.
pub const ROW_SIZES: [usize; 4] = [8, 16, 24, 32];

/// All persisted settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Recently opened image paths, most-recent-first, capped at [`MAX_RECENT`].
    pub recent: Vec<PathBuf>,
    /// Whether the bottom status bar is shown.
    pub show_status_bar: bool,
    /// Hex-view display options.
    pub hex: HexViewOptions,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            recent: Vec::new(),
            show_status_bar: true,
            hex: HexViewOptions::default(),
        }
    }
}

impl Settings {
    /// Record `path` as the most recently opened image: move it (or insert it)
    /// to the front, drop duplicates, and cap the list at [`MAX_RECENT`].
    pub fn push_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }

    /// Forget all recent files.
    pub fn clear_recent(&mut self) {
        self.recent.clear();
    }
}

/// How the hex view draws each row. Each field maps to a View-menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HexViewOptions {
    /// Show the address/offset gutter ("Line numbers").
    pub show_line_numbers: bool,
    /// Address gutter base: `true` = hexadecimal, `false` = decimal.
    pub line_number_hex: bool,
    /// Show the hexadecimal byte columns.
    pub show_hex: bool,
    /// Show the plain-text (ASCII) gutter.
    pub show_ascii: bool,
    /// Show the column-offset header ruler.
    pub show_columns: bool,
    /// Bytes shown per row in the file hex view (one of [`ROW_SIZES`]).
    pub bytes_per_row: usize,
    /// How hex bytes are clustered: ungrouped (no spaces) or N contiguous
    /// bytes per group, separated by a single space.
    pub grouping: ByteGrouping,
    /// Render `0x00` bytes as blanks in the hex (and ASCII) columns.
    pub hide_null_bytes: bool,
}

impl Default for HexViewOptions {
    fn default() -> Self {
        HexViewOptions {
            show_line_numbers: true,
            line_number_hex: true,
            show_hex: true,
            show_ascii: true,
            show_columns: true,
            bytes_per_row: 16,
            // One byte per group = the classic space-separated hex dump.
            grouping: ByteGrouping::Of(1),
            hide_null_bytes: false,
        }
    }
}

/// Byte-grouping for the hex columns: either ungrouped (bytes printed with no
/// spaces) or `N` contiguous bytes per group with a single space between groups
/// (`N` in 1/2/3/4/8/16/32). `Of(1)` is the classic one-space-per-byte dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ByteGrouping {
    None,
    Of(usize),
}

impl ByteGrouping {
    /// The selectable grouping sizes offered by the View menu.
    pub const SIZES: [usize; 7] = [1, 2, 3, 4, 8, 16, 32];

    /// Group size as a count of bytes, or `None` when ungrouped.
    pub fn size(self) -> Option<usize> {
        match self {
            ByteGrouping::None => None,
            ByteGrouping::Of(n) => Some(n.max(1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_recent_moves_existing_to_front_without_duplicating() {
        let mut s = Settings::default();
        s.push_recent(Path::new("/a.dsk"));
        s.push_recent(Path::new("/b.dsk"));
        s.push_recent(Path::new("/a.dsk"));
        assert_eq!(
            s.recent,
            vec![PathBuf::from("/a.dsk"), PathBuf::from("/b.dsk")]
        );
    }

    #[test]
    fn push_recent_caps_at_max() {
        let mut s = Settings::default();
        for i in 0..(MAX_RECENT + 5) {
            s.push_recent(Path::new(&format!("/disk{i}.dsk")));
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
        // The most recent push sits at the front.
        assert_eq!(s.recent[0], PathBuf::from("/disk14.dsk"));
    }

    #[test]
    fn clear_recent_empties_the_list() {
        let mut s = Settings::default();
        s.push_recent(Path::new("/a.dsk"));
        s.clear_recent();
        assert!(s.recent.is_empty());
    }

    #[test]
    fn settings_round_trip_through_json() {
        let mut s = Settings::default();
        s.push_recent(Path::new("/a.dsk"));
        s.hex.line_number_hex = false;
        s.hex.grouping = ByteGrouping::Of(4);
        s.show_status_bar = false;
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn grouping_size_clamps_and_reports_none() {
        assert_eq!(ByteGrouping::None.size(), None);
        assert_eq!(ByteGrouping::Of(4).size(), Some(4));
        assert_eq!(ByteGrouping::Of(0).size(), Some(1));
    }
}
