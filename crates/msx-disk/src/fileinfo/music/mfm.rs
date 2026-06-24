//! MoonBlaster for MoonSound FM (`.mfm`).
//!
//! RoboPlay's `mfm.c`: a 6-byte signature ("MBMS\x10\x01" USER, "MBMS\x10\x02"
//! EDIT, "MBMS\x10\x00" RAW) precedes the data. USER files put `MFM_HEADER` at
//! offset 6; EDIT and RAW files insert a 220-byte position table first, so the
//! header starts at 226. Within `MFM_HEADER`: `song_length` at 0,
//! `hz_equalizer` at 555, `nr_of_4op_channels` at 581, `song_name[50]` at 718,
//! `wave_kit_name[8]` at 804.

use super::{read_string, MusicInfo};

const SONG_NAME_IN_HEADER: usize = 718;
const WAVE_KIT_IN_HEADER: usize = 804;
const HZ_EQUALIZER_IN_HEADER: usize = 555;
const NR_4OP_IN_HEADER: usize = 581;
const POSITION_TABLE: usize = 220; // MAX_POSITION + 1

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    if bytes.get(0..5) != Some(b"MBMS\x10") {
        return None;
    }
    let (kind, shifted) = match bytes.get(5)? {
        0x01 => ("USER", false),
        0x02 => ("EDIT", true),
        0x00 => ("RAW", true),
        _ => return None,
    };
    let base = if shifted { 6 + POSITION_TABLE } else { 6 };

    let title = read_string(bytes, base + SONG_NAME_IN_HEADER, 50);
    let positions = (kind == "USER")
        .then(|| bytes.get(base).map(|&n| u32::from(n)))
        .flatten();

    let mut extra = vec![("File".to_string(), kind.to_string())];
    if let Some(&hz) = bytes.get(base + HZ_EQUALIZER_IN_HEADER) {
        extra.push((
            "Replay".to_string(),
            if hz != 0 { "50 Hz" } else { "60 Hz" }.to_string(),
        ));
    }
    if let Some(&n4) = bytes.get(base + NR_4OP_IN_HEADER) {
        extra.push(("4-op channels".to_string(), n4.to_string()));
    }
    if let Some(kit) = read_string(bytes, base + WAVE_KIT_IN_HEADER, 8) {
        extra.push(("Wave kit".to_string(), kit));
    }

    Some(MusicInfo {
        format: "MoonBlaster for MoonSound FM",
        title,
        author: None,
        positions,
        subsongs: Some(0),
        channels: Some(6),
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(variant: u8, song_len: u8, hz: u8, n4: u8, name: &str) -> Vec<u8> {
        let shifted = variant != 0x01;
        let base = if shifted { 6 + POSITION_TABLE } else { 6 };
        let mut buf = vec![0u8; base + 820];
        buf[0..5].copy_from_slice(b"MBMS\x10");
        buf[5] = variant;
        buf[base] = song_len;
        buf[base + HZ_EQUALIZER_IN_HEADER] = hz;
        buf[base + NR_4OP_IN_HEADER] = n4;
        let no = base + SONG_NAME_IN_HEADER;
        buf[no..no + name.len()].copy_from_slice(name.as_bytes());
        buf
    }

    #[test]
    fn parses_user_file() {
        let info = parse(&build(0x01, 12, 1, 3, "FM SONG")).unwrap();
        assert_eq!(info.title.as_deref(), Some("FM SONG"));
        assert_eq!(info.positions, Some(12));
        assert_eq!(info.channels, Some(6));
        assert!(info
            .extra
            .contains(&("Replay".to_string(), "50 Hz".to_string())));
        assert!(info
            .extra
            .contains(&("4-op channels".to_string(), "3".to_string())));
    }

    #[test]
    fn parses_raw_file_with_shifted_header() {
        let info = parse(&build(0x00, 0, 0, 0, "RAW FM")).unwrap();
        assert_eq!(info.title.as_deref(), Some("RAW FM"));
        assert!(info
            .extra
            .contains(&("File".to_string(), "RAW".to_string())));
    }

    #[test]
    fn rejects_unknown_signature() {
        assert!(parse(b"not a moonblaster fm file here!!").is_none());
    }
}
