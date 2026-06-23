//! Tyfoon Pro-Tracker (`.pro`).
//!
//! From RoboPlay's `pro.c`/`pro.h`: `PRO_HEADER` at offset 0 begins with the
//! `signature[4]` "PT10", then `song_name[16]` at 4, `author[16]` at 20, and
//! `song_length` at 322.

use super::{read_string, MusicInfo};

const SIGNATURE: &[u8; 4] = b"PT10";
const SONG_NAME_OFFSET: usize = 4;
const AUTHOR_OFFSET: usize = 20;
const SONG_LENGTH_OFFSET: usize = 322;
const NAME_LEN: usize = 16;

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    if bytes.get(0..4) != Some(SIGNATURE) {
        return None;
    }
    Some(MusicInfo {
        format: "Tyfoon Pro-Tracker",
        title: read_string(bytes, SONG_NAME_OFFSET, NAME_LEN),
        author: read_string(bytes, AUTHOR_OFFSET, NAME_LEN),
        positions: bytes.get(SONG_LENGTH_OFFSET).map(|&n| u32::from(n)),
        subsongs: Some(0),
        channels: Some(6),
        extra: vec![("Replay".to_string(), "stereo MSX-MUSIC".to_string())],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_signed_header() {
        let mut buf = vec![0u8; 325];
        buf[0..4].copy_from_slice(SIGNATURE);
        buf[SONG_NAME_OFFSET..SONG_NAME_OFFSET + 4].copy_from_slice(b"TUNE");
        buf[AUTHOR_OFFSET..AUTHOR_OFFSET + 3].copy_from_slice(b"BOB");
        buf[SONG_LENGTH_OFFSET] = 20;

        let info = parse(&buf).unwrap();
        assert_eq!(info.title.as_deref(), Some("TUNE"));
        assert_eq!(info.author.as_deref(), Some("BOB"));
        assert_eq!(info.positions, Some(20));
        assert_eq!(info.channels, Some(6));
    }

    #[test]
    fn rejects_without_signature() {
        assert!(parse(&[0u8; 325]).is_none());
    }
}
