//! FAC SoundTracker (`.mus`), versions 1.0 / 2.0 / PRO.
//!
//! `.mus` files are BSAVE'd (7-byte BLOAD header) with `0x3F00` bytes of song
//! data followed by a 256-byte header — RoboPlay's `mus.c` sets
//! `g_mus_header = DATA_SEGMENT_BASE + 0x3F00`, where the data is loaded from
//! file offset 7. So the header sits at the fixed file offset `7 + 0x3F00`.
//! Field offsets within that block: `sample_kit_name[8]` at 132,
//! `version` at 140, `song_name[40]` at 166, `author[40]` at 207, `device_id`
//! at 247, `number_of_tracks` at 248. For PRO (version 3) the name and author
//! fields are 5 bytes shorter.

use super::{read_string, MusicInfo};

/// Fixed file offset of the header: 7-byte BLOAD header + 0x3F00 song data.
const HEADER_BASE: usize = 7 + 0x3F00;
const SAMPLE_KIT_IN_HEADER: usize = 132;
const VERSION_IN_HEADER: usize = 140;
const SONG_NAME_IN_HEADER: usize = 166;
const AUTHOR_IN_HEADER: usize = 207;
const DEVICE_ID_IN_HEADER: usize = 247;
const TRACKS_IN_HEADER: usize = 248;
const NAME_LEN: usize = 40;

const DEVICE_MUSIC: u8 = 0x01;

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    let base = HEADER_BASE;
    let &version = bytes.get(base + VERSION_IN_HEADER)?;
    if version > 3 {
        return None; // not a recognizable FAC SoundTracker header
    }
    // PRO repurposes the last 5 bytes of the name/author fields.
    let name_len = if version >= 3 { NAME_LEN - 5 } else { NAME_LEN };

    let title = read_string(bytes, base + SONG_NAME_IN_HEADER, name_len);
    let author = read_string(bytes, base + AUTHOR_IN_HEADER, name_len);
    let tracks = bytes.get(base + TRACKS_IN_HEADER).map(|&n| u32::from(n));
    let device_music = bytes.get(base + DEVICE_ID_IN_HEADER) == Some(&DEVICE_MUSIC);

    let version_name = match version {
        0 | 1 => "FAC SoundTracker 1.0",
        2 => "FAC SoundTracker 2.0",
        _ => "FAC SoundTracker PRO",
    };
    let voices = if device_music {
        "6CH MSX-MUSIC + percussion"
    } else {
        "9CH MSX-AUDIO"
    };

    let mut extra = vec![("Voices".to_string(), voices.to_string())];
    if let Some(t) = tracks {
        extra.push(("Tracks".to_string(), t.to_string()));
    }
    if let Some(kit) = read_string(bytes, base + SAMPLE_KIT_IN_HEADER, 8) {
        extra.push(("Sample kit".to_string(), kit));
    }

    Some(MusicInfo {
        format: version_name,
        title,
        author,
        positions: None,
        subsongs: Some(0),
        channels: None,
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a standard-size `.mus`: 7-byte BLOAD header, 0x3F00 song data, then
    /// the 256-byte trailing header.
    fn build(version: u8, device: u8, tracks: u8, name: &str, author: &str, kit: &str) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_BASE + 256];
        buf[0] = 0xFE; // BLOAD marker
        let base = HEADER_BASE;
        buf[base + VERSION_IN_HEADER] = version;
        buf[base + DEVICE_ID_IN_HEADER] = device;
        buf[base + TRACKS_IN_HEADER] = tracks;
        let put = |buf: &mut [u8], off: usize, s: &str| {
            buf[base + off..base + off + s.len()].copy_from_slice(s.as_bytes());
        };
        put(&mut buf, SONG_NAME_IN_HEADER, name);
        put(&mut buf, AUTHOR_IN_HEADER, author);
        put(&mut buf, SAMPLE_KIT_IN_HEADER, kit);
        buf
    }

    #[test]
    fn parses_v20_audio_with_trailing_header() {
        let info = parse(&build(2, 0x00, 83, "MY TUNE", "ME", "DRUMS")).unwrap();
        assert_eq!(info.format, "FAC SoundTracker 2.0");
        assert_eq!(info.title.as_deref(), Some("MY TUNE"));
        assert_eq!(info.author.as_deref(), Some("ME"));
        assert!(info
            .extra
            .contains(&("Tracks".to_string(), "83".to_string())));
        assert!(info
            .extra
            .contains(&("Sample kit".to_string(), "DRUMS".to_string())));
        assert!(info.extra.iter().any(|(_, v)| v.contains("9CH MSX-AUDIO")));
    }

    #[test]
    fn parses_pro_music() {
        let info = parse(&build(3, DEVICE_MUSIC, 4, "PRO SONG", "AUTHOR", "")).unwrap();
        assert_eq!(info.format, "FAC SoundTracker PRO");
        assert_eq!(info.title.as_deref(), Some("PRO SONG"));
        assert!(info.extra.iter().any(|(_, v)| v.contains("MSX-MUSIC")));
    }

    #[test]
    fn rejects_bad_version() {
        let mut buf = vec![0u8; HEADER_BASE + 256];
        buf[HEADER_BASE + VERSION_IN_HEADER] = 0x55;
        assert!(parse(&buf).is_none());
    }

    #[test]
    fn too_short_is_none() {
        assert!(parse(&[0u8; 100]).is_none());
    }
}
