//! Plain, UI-free data structures describing the contents of a disk.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Local, LocalResult, TimeZone};

use crate::charset::{self, MsxCharset};

/// A DOS-style file attribute set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attributes {
    pub read_only: bool,
    pub hidden: bool,
    pub system: bool,
    pub archive: bool,
}

/// A decoded directory-entry timestamp (minute resolution is plenty for MSX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

impl Timestamp {
    /// This timestamp as a [`SystemTime`], for stamping an extracted host file's
    /// modification time so it keeps its disk date instead of "now".
    ///
    /// FAT stores wall-clock time with no timezone, so the civil fields are
    /// interpreted in the host's local timezone: the date and time the OS then
    /// shows for the extracted file match what the app shows for it inside the
    /// image. Returns `None` for a date that does not exist — an out-of-range
    /// field, or a local time skipped by a daylight-saving transition.
    pub fn to_system_time(&self) -> Option<SystemTime> {
        let local = Local.with_ymd_and_hms(
            self.year as i32,
            self.month as u32,
            self.day as u32,
            self.hour as u32,
            self.minute as u32,
            0,
        );
        let secs = match local {
            LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => dt.timestamp(),
            LocalResult::None => return None,
        };
        Some(if secs >= 0 {
            UNIX_EPOCH + Duration::from_secs(secs as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
        })
    }
}

/// One entry (file or directory) in the disk's directory tree.
///
/// Directories carry their `children`, so a single [`DirEntry`] returned from
/// the root is a full recursive tree — including MSX-DOS 2 subdirectories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Bare 8.3 name and the fatfs access key, e.g. `GAME.COM`.
    ///
    /// ASCII for `< 0x80`; high bytes are carried losslessly in the Unicode
    /// Private Use Area (`U+F080..=U+F0FF`) so file I/O round-trips. Use
    /// [`DirEntry::display_name`] for anything user-facing.
    pub name: String,
    /// Full slash-separated path from the root, e.g. `UTILS/GAME.COM`.
    ///
    /// Same PUA encoding as [`DirEntry::name`]; this is the key passed to
    /// [`crate::fs::DiskFs::read_file`].
    pub path: String,
    pub is_dir: bool,
    /// File size in bytes (0 for directories).
    pub size: u64,
    pub attributes: Attributes,
    pub modified: Option<Timestamp>,
    /// Child entries, for directories.
    pub children: Vec<DirEntry>,
}

impl DirEntry {
    /// The human-readable file name, decoded for display under `charset`.
    ///
    /// Turns the PUA-encoded [`DirEntry::name`] back into real Unicode (kana,
    /// accented Latin, ...). Pure-ASCII names are returned unchanged.
    pub fn display_name(&self, charset: MsxCharset) -> String {
        charset::decode_fs_name(charset, &self.name)
    }

    /// Depth-first iterator over this entry and all of its descendants.
    pub fn walk(&self) -> impl Iterator<Item = &DirEntry> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            for child in next.children.iter().rev() {
                stack.push(child);
            }
            Some(next)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    fn ts(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Timestamp {
        Timestamp {
            year,
            month,
            day,
            hour,
            minute,
        }
    }

    #[test]
    fn to_system_time_round_trips_through_local_time() {
        // A mid-May afternoon is never inside a daylight-saving transition, so
        // the civil fields survive the round-trip whatever the test machine's
        // timezone is.
        let t = ts(1990, 5, 12, 14, 30);
        let secs = t
            .to_system_time()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let back = Local.timestamp_opt(secs, 0).single().unwrap();
        assert_eq!(
            (
                back.year(),
                back.month(),
                back.day(),
                back.hour(),
                back.minute()
            ),
            (1990, 5, 12, 14, 30)
        );
    }

    #[test]
    fn to_system_time_rejects_impossible_dates() {
        assert!(ts(1990, 0, 1, 0, 0).to_system_time().is_none());
        assert!(ts(1990, 13, 1, 0, 0).to_system_time().is_none());
        assert!(ts(1990, 2, 30, 0, 0).to_system_time().is_none());
        assert!(ts(1990, 1, 32, 0, 0).to_system_time().is_none());
        assert!(ts(1990, 1, 1, 24, 0).to_system_time().is_none());
        assert!(ts(1990, 1, 1, 0, 60).to_system_time().is_none());
    }
}
