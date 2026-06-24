//! SCC Blaffer NT (`.sbm`).
//!
//! From RoboPlay's `sbm.c`/`sbm.h`: a 16-byte signature "Blaf NT Song    "
//! precedes `SBM_HEADER`. Header field offsets (base = 16): `song_name[67]` at
//! 16, `instrument_kit_name[8]`+`instrument_kit_ext[3]` at 83, `last_position`
//! at 94, `tempo` at 353.

use super::{read_string, MusicInfo};

const SIGNATURE: &[u8] = b"Blaf NT Song    ";
const BASE: usize = 16;
const SONG_NAME_OFFSET: usize = BASE; // 16
const KIT_NAME_OFFSET: usize = BASE + 67; // 83
const KIT_EXT_OFFSET: usize = BASE + 75; // 91
const LAST_POSITION_OFFSET: usize = BASE + 78; // 94
const TEMPO_OFFSET: usize = BASE + 337; // 353

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    if bytes.get(0..SIGNATURE.len()) != Some(SIGNATURE) {
        return None;
    }
    let title = read_string(bytes, SONG_NAME_OFFSET, 67);
    let positions = bytes.get(LAST_POSITION_OFFSET).map(|&n| u32::from(n) + 1);

    let mut extra = vec![("Drums".to_string(), "PSG".to_string())];
    if let Some(name) = read_string(bytes, KIT_NAME_OFFSET, 8) {
        let ext = read_string(bytes, KIT_EXT_OFFSET, 3).unwrap_or_default();
        let kit = if ext.is_empty() {
            name
        } else {
            format!("{name}.{ext}")
        };
        extra.push(("Instrument kit".to_string(), kit));
    }
    if let Some(&tempo) = bytes.get(TEMPO_OFFSET) {
        extra.push(("Tempo".to_string(), tempo.to_string()));
    }

    Some(MusicInfo {
        format: "SCC Blaffer NT",
        title,
        author: None,
        positions,
        subsongs: Some(0),
        channels: Some(5),
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_signed_header() {
        let mut buf = vec![0u8; 360];
        buf[0..SIGNATURE.len()].copy_from_slice(SIGNATURE);
        buf[SONG_NAME_OFFSET..SONG_NAME_OFFSET + 9].copy_from_slice(b"SCC SONG!");
        buf[KIT_NAME_OFFSET..KIT_NAME_OFFSET + 4].copy_from_slice(b"KICK");
        buf[KIT_EXT_OFFSET..KIT_EXT_OFFSET + 3].copy_from_slice(b"SBK");
        buf[LAST_POSITION_OFFSET] = 7;
        buf[TEMPO_OFFSET] = 12;

        let info = parse(&buf).unwrap();
        assert_eq!(info.title.as_deref(), Some("SCC SONG!"));
        assert_eq!(info.positions, Some(8));
        assert_eq!(info.channels, Some(5));
        assert!(info
            .extra
            .contains(&("Instrument kit".to_string(), "KICK.SBK".to_string())));
        assert!(info
            .extra
            .contains(&("Tempo".to_string(), "12".to_string())));
    }

    #[test]
    fn rejects_without_signature() {
        assert!(parse(&[0u8; 360]).is_none());
    }
}
