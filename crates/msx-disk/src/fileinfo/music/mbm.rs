//! MoonBlaster 1.4 (`.mbm`).
//!
//! From RoboPlay's `mbm.c`/`mbm.h`, the `MBM_HEADER` maps to file offset 0 for
//! both USER and EDIT files. Byte 0 is `song_length` (0xFF marks an EDIT file).
//! Computed field offsets (packed struct): `sustain` at 206, `track_name[41]`
//! at 207, `sample_kit_name[8]` at 248.

use super::{read_string, MusicInfo};

const SUSTAIN_OFFSET: usize = 206;
const TRACK_NAME_OFFSET: usize = 207;
const TRACK_NAME_LEN: usize = 40;
const SAMPLE_KIT_OFFSET: usize = 248;

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    if bytes.len() < TRACK_NAME_OFFSET + TRACK_NAME_LEN {
        return None;
    }
    let song_length = bytes[0];
    let edit = song_length == 0xFF;

    let title = read_string(bytes, TRACK_NAME_OFFSET, TRACK_NAME_LEN);
    let positions = (!edit).then(|| u32::from(song_length));

    let percussion = bytes[SUSTAIN_OFFSET] & 0x20 != 0;
    let voices = if percussion {
        "9CH MSX-AUDIO + 6CH MSX-MUSIC + percussion"
    } else {
        "9CH MSX-AUDIO + 9CH MSX-MUSIC"
    };

    let mut extra = vec![
        (
            "File".to_string(),
            if edit { "EDIT" } else { "USER" }.to_string(),
        ),
        ("Voices".to_string(), voices.to_string()),
    ];
    if let Some(kit) = read_string(bytes, SAMPLE_KIT_OFFSET, 8) {
        extra.push(("Sample kit".to_string(), kit));
    }

    Some(MusicInfo {
        format: "MoonBlaster 1.4",
        title,
        author: None,
        positions,
        subsongs: Some(0),
        channels: None,
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(song_length: u8, sustain: u8, name: &str) -> Vec<u8> {
        let mut buf = vec![0u8; 256];
        buf[0] = song_length;
        buf[SUSTAIN_OFFSET] = sustain;
        buf[TRACK_NAME_OFFSET..TRACK_NAME_OFFSET + name.len()].copy_from_slice(name.as_bytes());
        buf
    }

    #[test]
    fn parses_user_file() {
        let info = parse(&build(0x10, 0x00, "SONG TITLE")).unwrap();
        assert_eq!(info.title.as_deref(), Some("SONG TITLE"));
        assert_eq!(info.positions, Some(0x10));
        assert!(info.extra.contains(&("File".to_string(), "USER".to_string())));
        assert!(info
            .extra
            .iter()
            .any(|(k, v)| k == "Voices" && v.contains("9CH MSX-MUSIC")));
    }

    #[test]
    fn edit_file_has_no_positions_and_percussion_flag() {
        let info = parse(&build(0xFF, 0x20, "EDIT SONG")).unwrap();
        assert_eq!(info.positions, None);
        assert!(info.extra.contains(&("File".to_string(), "EDIT".to_string())));
        assert!(info
            .extra
            .iter()
            .any(|(_, v)| v.contains("percussion")));
    }

    #[test]
    fn too_short_is_none() {
        assert!(parse(&[0u8; 100]).is_none());
    }
}
