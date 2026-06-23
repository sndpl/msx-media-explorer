//! Metadata extraction for MSX music / tracker file formats.
//!
//! Each submodule parses one format's header and returns a [`MusicInfo`]. The
//! field offsets are derived from the RoboPlay players
//! (<https://gitlab.com/torihino/roboplay>, `players/src/<fmt>.{c,h}`), whose C
//! header structs are byte-packed (SDCC on Z80 inserts no alignment padding),
//! so a struct field's offset is simply the sum of the preceding field sizes.

mod amiga_mod;
mod etracker;
mod mbm;
mod mfm;
mod mid;
mod mus;
mod mwm;
mod pro;
mod pt3;
mod sbm;
mod sng;

/// Decoded metadata about a music file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MusicInfo {
    /// Human-readable format name, e.g. `MoonBlaster 1.4`.
    pub format: &'static str,
    /// The embedded song/track title, if the format carries one.
    pub title: Option<String>,
    /// The embedded author/composer, if present.
    pub author: Option<String>,
    /// Number of song positions (the order list length), if known.
    pub positions: Option<u32>,
    /// Number of subsongs, if the format supports them.
    pub subsongs: Option<u8>,
    /// Number of channels, if fixed/derivable.
    pub channels: Option<u8>,
    /// Extra format-specific facts (label, value) shown as-is.
    pub extra: Vec<(String, String)>,
}

/// Whether the lowercase extension names a music format we can describe.
pub fn is_music_ext(ext: &str) -> bool {
    matches!(
        ext,
        "mbm" | "mus" | "pro" | "mwm" | "mfm" | "mid" | "sbm" | "sng" | "pt3" | "mod" | "etc"
            | "cop" | "saa"
    )
}

/// Parse music metadata for a recognized extension, or `None`.
pub fn describe(ext: &str, bytes: &[u8]) -> Option<MusicInfo> {
    match ext {
        "mbm" => mbm::parse(bytes),
        "mus" => mus::parse(bytes),
        "pro" => pro::parse(bytes),
        "mwm" => mwm::parse(bytes),
        "mfm" => mfm::parse(bytes),
        "sbm" => sbm::parse(bytes),
        "sng" => sng::parse(bytes),
        "pt3" => pt3::parse(bytes),
        "mod" => amiga_mod::parse(bytes),
        "mid" => mid::parse(bytes),
        "etc" | "cop" | "saa" => etracker::parse(bytes),
        _ => None,
    }
}

/// Read a fixed-length field as a trimmed string.
///
/// Bytes are taken from `start` for up to `len`, cut at the first NUL, with
/// trailing spaces and control bytes trimmed. Non-ASCII bytes are kept (MSX
/// titles occasionally use the upper character set). Returns `None` when the
/// range is out of bounds or the result is empty.
pub(crate) fn read_string(bytes: &[u8], start: usize, len: usize) -> Option<String> {
    let end = start.checked_add(len)?;
    let field = bytes.get(start..end)?;
    let cut = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let s: String = field[..cut]
        .iter()
        .map(|&b| b as char)
        .collect::<String>()
        .trim_end_matches(|c: char| c == ' ' || c.is_control())
        .to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_string_trims_and_bounds_checks() {
        let buf = b"HELLO   \0\0rest";
        assert_eq!(read_string(buf, 0, 10).as_deref(), Some("HELLO"));
        assert_eq!(read_string(buf, 0, 3).as_deref(), Some("HEL"));
        assert!(read_string(b"   ", 0, 3).is_none());
        assert!(read_string(b"AB", 0, 10).is_none());
    }

    #[test]
    fn dispatch_unknown_is_none() {
        assert!(describe("zzz", &[0u8; 16]).is_none());
        assert!(is_music_ext("mbm"));
        assert!(!is_music_ext("txt"));
    }
}
