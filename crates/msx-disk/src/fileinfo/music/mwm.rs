//! MoonBlaster for MoonSound Wave (`.mwm`).
//!
//! RoboPlay's `mwm.c`: a 6-byte signature ("MBMS\x10\x08" USER, "MBMS\x10\x07"
//! EDIT) precedes the data. For USER files `MWM_HEADER` follows immediately at
//! offset 6; for EDIT files a 220-byte position table comes first, so the header
//! starts at 226. Within `MWM_HEADER`: `song_length` at 0, `base_frequency` at
//! 27, `song_name[50]` at 220, `wave_kit_name[8]` at 270.

use super::{read_string, MusicInfo};

const SONG_NAME_IN_HEADER: usize = 220;
const WAVE_KIT_IN_HEADER: usize = 270;
const BASE_FREQ_IN_HEADER: usize = 27;
const POSITION_TABLE: usize = 220; // MAX_POSITION + 1

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    if bytes.get(0..5) != Some(b"MBMS\x10") {
        return None;
    }
    let edit = match bytes.get(5)? {
        0x08 => false,
        0x07 => true,
        _ => return None,
    };
    let base = if edit { 6 + POSITION_TABLE } else { 6 };

    let title = read_string(bytes, base + SONG_NAME_IN_HEADER, 50);
    let positions = (!edit)
        .then(|| bytes.get(base).map(|&n| u32::from(n)))
        .flatten();

    let mut extra = vec![(
        "File".to_string(),
        if edit { "EDIT" } else { "USER" }.to_string(),
    )];
    if let Some(&bf) = bytes.get(base + BASE_FREQ_IN_HEADER) {
        let hz = match bf {
            0 => "60 Hz".to_string(),
            1 => "50 Hz".to_string(),
            n => format!("custom ({n})"),
        };
        extra.push(("Replay".to_string(), hz));
    }
    if let Some(kit) = read_string(bytes, base + WAVE_KIT_IN_HEADER, 8) {
        extra.push(("Wave kit".to_string(), kit));
    }

    Some(MusicInfo {
        format: "MoonBlaster for MoonSound Wave",
        title,
        author: None,
        positions,
        subsongs: Some(0),
        channels: Some(24),
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(variant: u8, song_len: u8, base_freq: u8, name: &str, kit: &str) -> Vec<u8> {
        let edit = variant == 0x07;
        let base = if edit { 6 + POSITION_TABLE } else { 6 };
        let mut buf = vec![0u8; base + 300];
        buf[0..5].copy_from_slice(b"MBMS\x10");
        buf[5] = variant;
        buf[base] = song_len;
        buf[base + BASE_FREQ_IN_HEADER] = base_freq;
        let no = base + SONG_NAME_IN_HEADER;
        buf[no..no + name.len()].copy_from_slice(name.as_bytes());
        let ko = base + WAVE_KIT_IN_HEADER;
        buf[ko..ko + kit.len()].copy_from_slice(kit.as_bytes());
        buf
    }

    #[test]
    fn parses_user_file() {
        let info = parse(&build(0x08, 24, 1, "WAVE SONG", "MYKIT")).unwrap();
        assert_eq!(info.title.as_deref(), Some("WAVE SONG"));
        assert_eq!(info.positions, Some(24));
        assert_eq!(info.channels, Some(24));
        assert!(info
            .extra
            .contains(&("Replay".to_string(), "50 Hz".to_string())));
        assert!(info
            .extra
            .contains(&("Wave kit".to_string(), "MYKIT".to_string())));
    }

    #[test]
    fn parses_edit_file_with_shifted_header() {
        let info = parse(&build(0x07, 0, 0, "EDIT WAVE", "KIT")).unwrap();
        assert_eq!(info.title.as_deref(), Some("EDIT WAVE"));
        assert_eq!(info.positions, None);
        assert!(info
            .extra
            .contains(&("File".to_string(), "EDIT".to_string())));
    }

    #[test]
    fn rejects_unknown_signature() {
        assert!(parse(b"NOPEthis is not moonblaster wave").is_none());
    }
}
