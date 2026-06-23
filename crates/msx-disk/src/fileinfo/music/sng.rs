//! SCC-Musixx (`.sng`).
//!
//! From RoboPlay's `sng.c`/`sng.h`: `SNG_HEADER` at offset 0 is 48 instruments
//! of 40 bytes each (`wave[32]` + `name[8]`) = 1920 bytes, then `song_length`
//! at 1920 and `patterns[100]`. The format carries no song title.

use super::MusicInfo;

const SONG_LENGTH_OFFSET: usize = 48 * 40; // 1920
const MAX_POSITIONS: u8 = 100;

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    let &song_length = bytes.get(SONG_LENGTH_OFFSET)?;
    // No signature exists; sanity-check the position count.
    let positions = (1..=MAX_POSITIONS).contains(&song_length).then(|| u32::from(song_length));

    Some(MusicInfo {
        format: "SCC-Musixx",
        title: None,
        author: None,
        positions,
        subsongs: Some(0),
        channels: Some(5),
        extra: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_song_length() {
        let mut buf = vec![0u8; 2021];
        buf[SONG_LENGTH_OFFSET] = 16;
        let info = parse(&buf).unwrap();
        assert_eq!(info.positions, Some(16));
        assert_eq!(info.channels, Some(5));
        assert_eq!(info.title, None);
    }

    #[test]
    fn implausible_position_count_is_dropped() {
        let mut buf = vec![0u8; 2021];
        buf[SONG_LENGTH_OFFSET] = 200;
        assert_eq!(parse(&buf).unwrap().positions, None);
    }

    #[test]
    fn too_short_is_none() {
        assert!(parse(&[0u8; 100]).is_none());
    }
}
