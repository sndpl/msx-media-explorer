//! Amiga module (`.mod`).
//!
//! Layout (Amiga ProTracker, as used by RoboPlay's `mod.c`):
//! `song_name[20]` at 0, 31 sample headers (30 bytes each), `song_length` at
//! 950, restart at 951, `pattern_table[128]` at 952, and the 4-byte format tag
//! at 1080. The tag gives the channel count.

use super::{read_string, MusicInfo};

const TAG_OFFSET: usize = 1080;
const SONG_LENGTH_OFFSET: usize = 950;

/// Known format tags from RoboPlay's `g_file_formats` table.
const KNOWN_TAGS: &[(&str, u8, &str)] = &[
    ("M.K.", 4, "Standard 4-channel module"),
    ("M!K!", 4, "4-channel module (extended length)"),
    ("FLT4", 4, "StarTrekker 4-channel module"),
    ("FLT8", 8, "StarTrekker 8-channel module"),
    ("2CHN", 2, "FastTracker 2-channel module"),
    ("4CHN", 4, "FastTracker 4-channel module"),
    ("6CHN", 6, "FastTracker 6-channel module"),
    ("8CHN", 8, "FastTracker 8-channel module"),
    ("16CH", 16, "FastTracker 16-channel module"),
    ("OCTA", 8, "Octalyzer module"),
];

pub(super) fn parse(bytes: &[u8]) -> Option<MusicInfo> {
    let title = read_string(bytes, 0, 20);

    let (channels, kind) = bytes
        .get(TAG_OFFSET..TAG_OFFSET + 4)
        .and_then(channels_from_tag)
        .unwrap_or((4, "4-channel module (15 or 31 samples)".to_string()));

    let positions = bytes.get(SONG_LENGTH_OFFSET).map(|&n| u32::from(n));

    let mut extra = vec![("Format".to_string(), kind)];
    extra.push(("Samples".to_string(), "31".to_string()));

    Some(MusicInfo {
        format: "Amiga module",
        title,
        author: None,
        positions,
        subsongs: Some(0),
        channels: Some(channels),
        extra,
    })
}

/// Resolve a 4-byte tag to `(channels, description)`.
fn channels_from_tag(tag: &[u8]) -> Option<(u8, String)> {
    let s = std::str::from_utf8(tag).ok()?;
    if let Some((_, ch, desc)) = KNOWN_TAGS.iter().find(|(t, _, _)| *t == s) {
        return Some((*ch, (*desc).to_string()));
    }
    // Generic tags such as "24CH", "32CH", "5CHN".
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let rest = &s[digits.len()..];
    if (rest == "CH" || rest == "CHN") && !digits.is_empty() {
        if let Ok(n) = digits.parse::<u8>() {
            return Some((n, format!("{n}-channel module")));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(title: &str, tag: &[u8; 4], song_len: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 1084];
        buf[..title.len()].copy_from_slice(title.as_bytes());
        buf[SONG_LENGTH_OFFSET] = song_len;
        buf[TAG_OFFSET..TAG_OFFSET + 4].copy_from_slice(tag);
        buf
    }

    #[test]
    fn parses_mk_4channel() {
        let info = parse(&build("MY SONG", b"M.K.", 12)).unwrap();
        assert_eq!(info.title.as_deref(), Some("MY SONG"));
        assert_eq!(info.channels, Some(4));
        assert_eq!(info.positions, Some(12));
        assert_eq!(info.format, "Amiga module");
    }

    #[test]
    fn parses_6chn_and_generic() {
        assert_eq!(parse(&build("X", b"6CHN", 1)).unwrap().channels, Some(6));
        assert_eq!(parse(&build("X", b"24CH", 1)).unwrap().channels, Some(24));
    }

    #[test]
    fn short_file_defaults_to_four_channels() {
        let info = parse(b"TINY MODULE NAME!!!!").unwrap();
        assert_eq!(info.channels, Some(4));
        assert_eq!(info.title.as_deref(), Some("TINY MODULE NAME!!!!"));
    }
}
