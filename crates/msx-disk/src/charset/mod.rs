//! MSX character-set decoding.
//!
//! MSX disks store filenames and BASIC/text literals in a **single-byte**,
//! region-specific character set. Bytes `0x00..=0x7F` are ASCII; bytes
//! `0x80..=0xFF` map to different glyphs depending on the machine's region
//! (Japanese kana, International accented/graphic glyphs, ...). This module
//! turns those bytes into real Unicode so they can be displayed with an
//! ordinary font, instead of the Latin-1 mojibake the views produced before.
//!
//! The byte->Unicode tables are sourced from the MSX Technical Data Book and
//! the Unicode L2/19-025 proposal (MSX.TXT); see [`tables`].

mod tables;

/// A single-byte MSX character set (code page).
///
/// `#[non_exhaustive]` so more regions (Korean, Arabic, Russian, Brazilian,
/// German) can be added later without breaking downstream `match`es.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MsxCharset {
    /// MSX International (Western), based on code page 437.
    #[default]
    International,
    /// MSX Japanese: JIS X 0201 half-width katakana plus hiragana and graphics.
    Japanese,
}

impl MsxCharset {
    /// All selectable charsets, in display order (for a UI dropdown).
    pub const ALL: &'static [MsxCharset] = &[MsxCharset::International, MsxCharset::Japanese];

    /// Human-readable label for a charset selector.
    pub fn label(self) -> &'static str {
        match self {
            MsxCharset::International => "International",
            MsxCharset::Japanese => "Japanese (Kana)",
        }
    }
}

/// Decode one MSX byte to a Unicode scalar under `charset`.
///
/// Bytes `0x00..=0x7F` are returned unchanged (ASCII / C0 control codes — the
/// caller decides how to render control codes). Bytes `0x80..=0xFF` are mapped
/// through the per-charset table. Genuinely undefined cells map to `U+FFFD`.
pub fn decode_byte(charset: MsxCharset, b: u8) -> char {
    if b < 0x80 {
        return b as char;
    }
    let table = match charset {
        MsxCharset::International => &tables::INTERNATIONAL_HIGH,
        MsxCharset::Japanese => &tables::JAPANESE_HIGH,
    };
    table[(b - 0x80) as usize]
}

/// Decode a byte slice (filename, DATA/REM/string literal) to a `String`.
pub fn decode(charset: MsxCharset, bytes: &[u8]) -> String {
    bytes.iter().map(|&b| decode_byte(charset, b)).collect()
}

/// Decode a `fatfs` name string into a display string under `charset`.
///
/// Names produced via [`PuaOemCpConverter`] are ASCII for `< 0x80` and carry
/// high bytes in the Private Use Area (`U+F080..=U+F0FF`). This recovers the
/// original byte for each PUA scalar and decodes it through `charset`, so the
/// stable fatfs access key (which stays PUA-encoded) can be turned into kana,
/// accented Latin, etc. for display. ASCII names pass through unchanged, so
/// synthetic entries (e.g. `P0` partition nodes) are unaffected.
pub fn decode_fs_name(charset: MsxCharset, name: &str) -> String {
    name.chars()
        .map(|c| match c as u32 {
            u @ 0xF080..=0xF0FF => decode_byte(charset, (u - 0xF000) as u8),
            u if u < 0x80 => decode_byte(charset, u as u8),
            _ => c, // already a decoded scalar; leave it be
        })
        .collect()
}

/// Recover the original MSX byte for one scalar of a PUA-encoded fatfs name.
///
/// Inverse of the per-byte mapping done by [`PuaOemCpConverter`]: ASCII scalars
/// and PUA scalars (`U+F080..=U+F0FF`) yield their byte; any other scalar (a
/// name that was never PUA-encoded) yields `None`.
pub fn fs_name_byte(c: char) -> Option<u8> {
    match c as u32 {
        u @ 0xF080..=0xF0FF => Some((u - 0xF000) as u8),
        u if u < 0x80 => Some(u as u8),
        _ => None,
    }
}

/// Best-effort auto-detection of the charset from a byte sample.
///
/// Single-byte sets overlap, so this is a heuristic, not a guarantee: a sample
/// whose high bytes cluster in the Japanese kana/hiragana bands is reported as
/// [`MsxCharset::Japanese`]; otherwise [`MsxCharset::International`] (also the
/// default when the sample is pure ASCII).
pub fn detect(bytes: &[u8]) -> MsxCharset {
    /// Minimum share of high bytes in the kana/hiragana bands to call it Japanese.
    const THRESHOLD: f32 = 0.6;
    /// Require at least this many katakana-band bytes, so a single stray byte
    /// (e.g. one accented Western letter) is never read as Japanese.
    const MIN_KANA: usize = 2;

    let mut high = 0usize;
    let mut kana = 0usize; // JIS X 0201 katakana band
    let mut hira = 0usize; // MSX-Japanese hiragana band
    for &b in bytes {
        if b < 0x80 {
            continue;
        }
        high += 1;
        match b {
            0xA1..=0xDF => kana += 1,
            0xE0..=0xFF => hira += 1,
            _ => {}
        }
    }

    if high == 0 || kana < MIN_KANA {
        return MsxCharset::International;
    }
    if (kana + hira) as f32 / high as f32 >= THRESHOLD {
        MsxCharset::Japanese
    } else {
        MsxCharset::International
    }
}

/// A lossless, charset-independent `fatfs` OEM code-page converter.
///
/// The default `fatfs` converter turns every byte `>= 0x80` into `U+FFFD`,
/// destroying non-ASCII filename bytes before we can decode them. This maps
/// `0x80..=0xFF` reversibly into the Unicode Private Use Area (`U+F080..=U+F0FF`)
/// so the original bytes survive on `DirEntry` and the fatfs open key round-trips
/// (`decode`/`encode` are exact inverses over `0x00..=0xFF`).
#[derive(Debug)]
pub struct PuaOemCpConverter;

/// Shared `'static` instance, as required by `fatfs::FsOptions::oem_cp_converter`.
pub static PUA_OEM_CP_CONVERTER: PuaOemCpConverter = PuaOemCpConverter;

/// Map an MSX OEM byte to a lossless fatfs-name scalar: ASCII stays ASCII,
/// `0x80..=0xFF` go into the Private Use Area (`U+F080..=U+F0FF`). This is the
/// canonical encoding used both by [`PuaOemCpConverter`] (the fatfs floppy path)
/// and by the raw directory reader [`crate::fs::map::entry_name`] (the
/// partition/disk-map path), so paths from either source match and decode the
/// same way. Inverse of [`fs_name_byte`].
pub fn byte_to_pua(b: u8) -> char {
    if b < 0x80 {
        b as char
    } else {
        char::from_u32(0xF000 + b as u32).expect("PUA codepoint is always valid")
    }
}

impl fatfs::OemCpConverter for PuaOemCpConverter {
    fn decode(&self, oem_char: u8) -> char {
        byte_to_pua(oem_char)
    }

    fn encode(&self, uni_char: char) -> Option<u8> {
        let u = uni_char as u32;
        if u < 0x80 {
            Some(u as u8)
        } else if (0xF080..=0xF0FF).contains(&u) {
            Some((u - 0xF000) as u8)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fatfs::OemCpConverter;

    #[test]
    fn ascii_is_identity_in_every_charset() {
        for &cs in MsxCharset::ALL {
            assert_eq!(decode_byte(cs, b'A'), 'A');
            assert_eq!(decode_byte(cs, b'0'), '0');
            assert_eq!(decode_byte(cs, b' '), ' ');
        }
    }

    #[test]
    fn international_high_bytes_map_to_western_glyphs() {
        // 0x80 = LATIN CAPITAL LETTER C WITH CEDILLA, 0xE1 = SHARP S,
        // 0x9D = YEN SIGN, 0xFF = cursor (rendered as full block).
        assert_eq!(decode_byte(MsxCharset::International, 0x80), '\u{00C7}');
        assert_eq!(decode_byte(MsxCharset::International, 0xE1), '\u{00DF}');
        assert_eq!(decode_byte(MsxCharset::International, 0x9D), '\u{00A5}');
        assert_eq!(decode_byte(MsxCharset::International, 0xFF), '\u{2588}');
    }

    #[test]
    fn japanese_high_bytes_map_to_kana() {
        // 0xA1 = HALFWIDTH IDEOGRAPHIC FULL STOP, 0xB1 = KATAKANA A,
        // 0xDF = SEMI-VOICED SOUND MARK, 0x86 = HIRAGANA WO, 0xE0 = HIRAGANA TA.
        assert_eq!(decode_byte(MsxCharset::Japanese, 0xA1), '\u{FF61}');
        assert_eq!(decode_byte(MsxCharset::Japanese, 0xB1), '\u{FF71}');
        assert_eq!(decode_byte(MsxCharset::Japanese, 0xDF), '\u{FF9F}');
        assert_eq!(decode_byte(MsxCharset::Japanese, 0x86), '\u{3092}');
        assert_eq!(decode_byte(MsxCharset::Japanese, 0xE0), '\u{305F}');
    }

    #[test]
    fn japanese_undefined_cells_map_to_replacement() {
        // 0x90, 0xA0 and 0xFE are undefined in the MSX-Japanese set.
        assert_eq!(decode_byte(MsxCharset::Japanese, 0x90), '\u{FFFD}');
        assert_eq!(decode_byte(MsxCharset::Japanese, 0xA0), '\u{FFFD}');
        assert_eq!(decode_byte(MsxCharset::Japanese, 0xFE), '\u{FFFD}');
    }

    #[test]
    fn decodes_real_japanese_data_line() {
        // The exact bytes from the user's reported `DATA67,DISK...` mojibake.
        let bytes = [0xE9, 0xCC, 0xA7, 0xB2, 0xD9, 0xDE, 0xEA, 0xDF, 0xE3];
        assert_eq!(
            decode(MsxCharset::Japanese, &bytes),
            "\u{306E}\u{FF8C}\u{FF67}\u{FF72}\u{FF99}\u{FF9E}\u{306F}\u{FF9F}\u{3066}"
        );
        // And it must never produce the replacement char for these defined bytes.
        assert!(!decode(MsxCharset::Japanese, &bytes).contains('\u{FFFD}'));
    }

    #[test]
    fn detect_pure_ascii_is_international() {
        assert_eq!(detect(b"GAME.BAS HELLO.TXT"), MsxCharset::International);
        assert_eq!(detect(b""), MsxCharset::International);
    }

    #[test]
    fn detect_kana_heavy_sample_is_japanese() {
        // Filename bytes dominated by the katakana/hiragana bands.
        let bytes = [0xC3, 0xB1, 0xC6, 0xEA, 0xE9, 0xCC, b'.', b'B', b'A', b'S'];
        assert_eq!(detect(&bytes), MsxCharset::Japanese);
    }

    #[test]
    fn detect_few_western_accents_stays_international() {
        // An accented Western name should not be mistaken for Japanese.
        let bytes = [b'C', b'A', b'F', 0x82, b'.', b'T', b'X', b'T']; // 0x82 = é (Intl)
        assert_eq!(detect(&bytes), MsxCharset::International);
    }

    #[test]
    fn decode_fs_name_turns_pua_into_kana() {
        // "<F0B1><F0B2><F0B3>.BAS" is the PUA form of katakana bytes B1 B2 B3.
        let name = "\u{F0B1}\u{F0B2}\u{F0B3}.BAS";
        assert_eq!(
            decode_fs_name(MsxCharset::Japanese, name),
            "\u{FF71}\u{FF72}\u{FF73}.BAS"
        );
        // Pure-ASCII names are untouched in every charset.
        assert_eq!(decode_fs_name(MsxCharset::Japanese, "GAME.COM"), "GAME.COM");
        assert_eq!(decode_fs_name(MsxCharset::International, "P0"), "P0");
    }

    #[test]
    fn fs_name_byte_recovers_pua_and_ascii() {
        assert_eq!(fs_name_byte('A'), Some(0x41));
        assert_eq!(fs_name_byte('\u{F0CC}'), Some(0xCC));
        assert_eq!(fs_name_byte('\u{FF61}'), None); // a real decoded glyph
    }

    #[test]
    fn pua_converter_round_trips_every_byte() {
        let conv = &PUA_OEM_CP_CONVERTER;
        for b in 0u8..=0xFF {
            let c = conv.decode(b);
            assert_eq!(conv.encode(c), Some(b), "byte {b:#04X} failed to round-trip");
        }
    }

    #[test]
    fn pua_converter_keeps_ascii_readable() {
        let conv = &PUA_OEM_CP_CONVERTER;
        assert_eq!(conv.decode(b'A'), 'A');
        assert_eq!(conv.decode(0xCC), '\u{F0CC}');
        assert_eq!(conv.encode('A'), Some(b'A'));
        assert_eq!(conv.encode('\u{FF61}'), None); // real kana is not a PUA key
    }
}
