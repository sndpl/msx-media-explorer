//! Persisted application settings: the recent-files list and the hex-view
//! display options driven by the View menu. Stored via eframe's storage
//! (`eframe::{get_value, set_value}`) so they survive across runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

/// Maximum number of entries kept in the recent-files list.
pub const MAX_RECENT: usize = 10;

/// Storage key under which [`Settings`] live in eframe's persisted key/value
/// store (`app.ron`).
pub const SETTINGS_KEY: &str = "settings";

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
    /// Chosen UI language. `None` means "follow the OS locale", resolved once at
    /// startup; a `Some` value is the user's explicit, persisted choice.
    pub language: Option<Lang>,
    /// Whether to check GitHub for a newer release on startup (opt-out).
    pub check_for_updates: bool,
    /// Unix time (seconds) of the last successful update check, for throttling.
    /// `None` means "never checked".
    pub last_update_check: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            recent: Vec::new(),
            show_status_bar: true,
            hex: HexViewOptions::default(),
            language: None,
            check_for_updates: true,
            last_update_check: None,
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

    /// Union another instance's recent list into this one: own entries keep
    /// their order in front, paths not already present are appended in `other`
    /// order, and the result is capped at [`MAX_RECENT`]. Used at save time so
    /// two concurrently running instances don't clobber each other's list.
    pub fn merge_recent(&mut self, other: &[PathBuf]) {
        for path in other {
            if !self.recent.contains(path) {
                self.recent.push(path.clone());
            }
        }
        self.recent.truncate(MAX_RECENT);
    }
}

/// Best-effort read of the settings currently persisted on disk, which may
/// have been written by *another running instance* since this one started.
/// eframe's own storage object is loaded once at startup and never re-read,
/// so concurrent writes are invisible through it — this goes to the file.
///
/// The file is eframe's `app.ron`: a RON map of `String -> String` whose
/// [`SETTINGS_KEY`] value is itself RON-serialized [`Settings`]. Any missing
/// file or parse failure yields `None` (the caller just skips merging).
pub fn read_from_storage_file(app_name: &str) -> Option<Settings> {
    let path = eframe::storage_dir(app_name)?.join("app.ron");
    parse_storage_ron(&std::fs::read_to_string(path).ok()?)
}

/// Extract [`Settings`] from the text of an eframe `app.ron` storage file.
fn parse_storage_ron(text: &str) -> Option<Settings> {
    let map: std::collections::HashMap<String, String> = ron::from_str(text).ok()?;
    ron::from_str(map.get(SETTINGS_KEY)?).ok()
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
    fn merge_recent_unions_keeping_own_order_first() {
        let mut s = Settings::default();
        s.push_recent(Path::new("/b.dsk"));
        s.push_recent(Path::new("/a.dsk")); // own list: a, b
        s.merge_recent(&[PathBuf::from("/c.dsk"), PathBuf::from("/b.dsk")]);
        // Own entries stay in front; only genuinely new paths are appended.
        assert_eq!(
            s.recent,
            vec![
                PathBuf::from("/a.dsk"),
                PathBuf::from("/b.dsk"),
                PathBuf::from("/c.dsk"),
            ]
        );
    }

    #[test]
    fn merge_recent_caps_at_max() {
        let mut s = Settings::default();
        for i in 0..MAX_RECENT {
            s.push_recent(Path::new(&format!("/own{i}.dsk")));
        }
        let other: Vec<PathBuf> = (0..5)
            .map(|i| PathBuf::from(format!("/o{i}.dsk")))
            .collect();
        s.merge_recent(&other);
        assert_eq!(s.recent.len(), MAX_RECENT);
        // A full own list leaves no room for the other instance's entries.
        assert!(s
            .recent
            .iter()
            .all(|p| p.to_string_lossy().starts_with("/own")));
    }

    #[test]
    fn merge_recent_with_empty_other_is_a_noop() {
        let mut s = Settings::default();
        s.push_recent(Path::new("/a.dsk"));
        let before = s.recent.clone();
        s.merge_recent(&[]);
        assert_eq!(s.recent, before);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let mut s = Settings::default();
        s.push_recent(Path::new("/a.dsk"));
        s.hex.line_number_hex = false;
        s.hex.grouping = ByteGrouping::Of(4);
        s.show_status_bar = false;
        s.language = Some(Lang::Dutch);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn language_defaults_to_none_and_old_configs_still_parse() {
        // Follow-the-OS is the default.
        assert_eq!(Settings::default().language, None);
        // A config written before the `language` field existed still loads
        // (serde-default fills it in), so there is no migration.
        let old = r#"{"recent":[],"show_status_bar":true,"hex":{}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.language, None);
    }

    #[test]
    fn update_settings_round_trip_and_default_for_old_configs() {
        // Defaults: the update check is on, and nothing has been checked yet.
        let mut s = Settings::default();
        assert!(s.check_for_updates);
        assert_eq!(s.last_update_check, None);
        // Both fields survive a serialization round-trip.
        s.check_for_updates = false;
        s.last_update_check = Some(1_700_000_000);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        // A config written before these fields existed still parses; the update
        // check defaults to on (serde-default fills the missing fields in).
        let old = r#"{"recent":[],"show_status_bar":true,"hex":{}}"#;
        let parsed: Settings = serde_json::from_str(old).unwrap();
        assert!(parsed.check_for_updates);
        assert_eq!(parsed.last_update_check, None);
    }

    #[test]
    fn parse_storage_ron_round_trips_eframe_format() {
        // Mirror eframe's storage layout exactly: a RON map of String -> String
        // whose "settings" value is itself RON-serialized Settings.
        let mut s = Settings::default();
        s.push_recent(Path::new("/other-instance.dsk"));
        let inner = ron::to_string(&s).unwrap();
        let map = std::collections::HashMap::from([(SETTINGS_KEY.to_string(), inner)]);
        let text = ron::to_string(&map).unwrap();
        assert_eq!(parse_storage_ron(&text), Some(s));
    }

    #[test]
    fn parse_storage_ron_rejects_garbage_gracefully() {
        assert_eq!(parse_storage_ron("not ron at all"), None);
        assert_eq!(parse_storage_ron("{}"), None); // valid map, no settings key
    }

    #[test]
    fn grouping_size_clamps_and_reports_none() {
        assert_eq!(ByteGrouping::None.size(), None);
        assert_eq!(ByteGrouping::Of(4).size(), Some(4));
        assert_eq!(ByteGrouping::Of(0).size(), Some(1));
    }
}
