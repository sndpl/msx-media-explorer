//! ProTracker 3.x / Vortex Tracker II (`.pt3`).
//!
//! RoboPlay's `pt3.c` copies the title from file offset `0x1e` (32 bytes) and
//! the author from `0x42` (32 bytes). A 6-channel TurboSound module is detected
//! from the last 16 bytes: "PT3!" at the tail start and "02TS" at the tail end.

use super::{read_string, MusicInfo};

const NAME_OFFSET: usize = 0x1e;
const AUTHOR_OFFSET: usize = 0x42;
const NAME_LEN: usize = 32;

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    if !(bytes.starts_with(b"ProTracker") || bytes.starts_with(b"Vortex")) {
        return None;
    }
    let six_channel = is_turbosound(bytes);
    Some(MusicInfo {
        format: "ProTracker 3 / Vortex Tracker II",
        title: read_string(bytes, NAME_OFFSET, NAME_LEN),
        author: read_string(bytes, AUTHOR_OFFSET, NAME_LEN),
        positions: None,
        subsongs: Some(0),
        channels: Some(if six_channel { 6 } else { 3 }),
        extra: vec![(
            "Replay".to_string(),
            if six_channel {
                "6 channels (TurboSound)"
            } else {
                "3 channels"
            }
            .to_string(),
        )],
    })
}

/// A TurboSound (6-channel) module has "PT3!" + "02TS" markers in its tail.
fn is_turbosound(bytes: &[u8]) -> bool {
    let n = bytes.len();
    if n < 16 {
        return false;
    }
    let tail = &bytes[n - 16..];
    &tail[0..4] == b"PT3!" && &tail[12..16] == b"02TS"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(title: &str, author: &str) -> Vec<u8> {
        let mut buf = vec![0u8; 0x70];
        buf[..12].copy_from_slice(b"ProTracker 3");
        buf[NAME_OFFSET..NAME_OFFSET + title.len()].copy_from_slice(title.as_bytes());
        buf[AUTHOR_OFFSET..AUTHOR_OFFSET + author.len()].copy_from_slice(author.as_bytes());
        buf
    }

    #[test]
    fn parses_3channel() {
        let info = parse(&build("CHIPTUNE", "CODER")).unwrap();
        assert_eq!(info.title.as_deref(), Some("CHIPTUNE"));
        assert_eq!(info.author.as_deref(), Some("CODER"));
        assert_eq!(info.channels, Some(3));
    }

    #[test]
    fn detects_turbosound_6channel() {
        let mut buf = build("TS SONG", "X");
        buf.extend_from_slice(&[0u8; 16]); // 16-byte TurboSound tail
        let n = buf.len();
        buf[n - 16..n - 12].copy_from_slice(b"PT3!");
        buf[n - 4..n].copy_from_slice(b"02TS");
        assert_eq!(parse(&buf).unwrap().channels, Some(6));
    }

    #[test]
    fn rejects_unknown() {
        assert!(parse(b"not a pt3 file at all").is_none());
    }
}
