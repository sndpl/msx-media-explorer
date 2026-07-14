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
/// `#[non_exhaustive]` so more regions (e.g. German DIN, or per-machine Brazilian
/// variants) can be added later without breaking downstream `match`es.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MsxCharset {
    /// MSX International (Western), based on code page 437.
    #[default]
    International,
    /// MSX Japanese: JIS X 0201 half-width katakana plus hiragana and graphics.
    Japanese,
    /// MSX Russian: relocated graphics plus Cyrillic.
    Russian,
    /// MSX Korean: Hangul compatibility jamo plus precomposed Hangul syllables.
    Korean,
    /// MSX Arabic: Arabic-Indic digits and Arabic letters (unshaped, see table).
    Arabic,
    /// MSX Brazilian: the BRASCII/ABNT set (International plus Portuguese accents).
    Brazilian,
}

impl MsxCharset {
    /// All selectable charsets, in display (menu) order.
    pub const ALL: &'static [MsxCharset] = &[
        MsxCharset::International,
        MsxCharset::Japanese,
        MsxCharset::Russian,
        MsxCharset::Korean,
        MsxCharset::Arabic,
        MsxCharset::Brazilian,
    ];

    /// Human-readable label for a charset selector.
    pub fn label(self) -> &'static str {
        match self {
            MsxCharset::International => "International",
            MsxCharset::Japanese => "Japanese (Kana)",
            MsxCharset::Russian => "Russian",
            MsxCharset::Korean => "Korean",
            MsxCharset::Arabic => "Arabic",
            MsxCharset::Brazilian => "Brazilian",
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
        MsxCharset::Russian => &tables::RUSSIAN_HIGH,
        MsxCharset::Korean => &tables::KOREAN_HIGH,
        MsxCharset::Arabic => &tables::ARABIC_HIGH,
        MsxCharset::Brazilian => &tables::BRAZILIAN_HIGH,
    };
    table[(b - 0x80) as usize]
}

/// Decode a byte slice (filename, DATA/REM/string literal) to a `String`.
pub fn decode(charset: MsxCharset, bytes: &[u8]) -> String {
    bytes.iter().map(|&b| decode_byte(charset, b)).collect()
}

/// Encode one display scalar back to its MSX byte under `charset`, or `None` if
/// the charset cannot represent it. Inverse of [`decode_byte`]: ASCII scalars
/// map to themselves; high glyphs are looked up in the charset's high table.
pub fn encode_byte(charset: MsxCharset, c: char) -> Option<u8> {
    if c == '\u{FFFD}' {
        return None; // the replacement char is not a real, encodable glyph
    }
    if (c as u32) < 0x80 {
        return Some(c as u8);
    }
    (0x80u8..=0xFF).find(|&b| decode_byte(charset, b) == c)
}

/// Encode a display name (ASCII plus decoded high glyphs) back into the
/// PUA-encoded fatfs key form the filesystem stores. Inverse of
/// [`decode_fs_name`]. Returns `None` if any character is not representable in
/// `charset`.
pub fn encode_fs_name(charset: MsxCharset, name: &str) -> Option<String> {
    name.chars()
        .map(|c| encode_byte(charset, c).map(byte_to_pua))
        .collect()
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

/// Make a decoded display string safe to show by replacing control characters
/// with their visible Unicode Control Picture, leaving everything else
/// untouched.
///
/// MSX filenames can contain C0 control bytes; crafted "fake" directory entries
/// are sometimes built entirely from them (BEL, CR, LF, FF, ...) so that listing
/// the directory beeps and clears the screen on a real MSX. Rendered raw they
/// become garbled or invisible glyphs, and distinct names can look identical.
/// This maps `0x00..=0x1F` to `U+2400 + b` (BEL 0x07 -> ␇, CR -> ␍, FF -> ␌, ...)
/// and DEL `0x7F` to `U+2421` (␡) — one scalar per byte, so column-aligned tables
/// stay aligned. Ordinary names are returned unchanged.
pub fn display_control_safe(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{7F}' => '\u{2421}',
            c if (c as u32) < 0x20 => char::from_u32(0x2400 + c as u32).unwrap_or(c),
            c => c,
        })
        .collect()
}

/// A short "ABBR — effect" description of a control byte in MSX-console terms,
/// or `None` for a non-control byte. Explains what a control character in a
/// filename does when the directory is listed on an MSX.
pub fn control_char_effect(b: u8) -> Option<&'static str> {
    Some(match b {
        0x00 => "NUL — null",
        0x07 => "BEL — beep",
        0x08 => "BS — backspace",
        0x09 => "TAB — horizontal tab",
        0x0A => "LF — line feed (cursor down)",
        0x0B => "HOME — cursor to top-left",
        0x0C => "FF — clear screen",
        0x0D => "CR — carriage return",
        0x1A => "SUB — end-of-file (Ctrl-Z)",
        0x1B => "ESC — start escape sequence",
        0x1C => "RIGHT — cursor right",
        0x1D => "LEFT — cursor left",
        0x1E => "UP — cursor up",
        0x1F => "DOWN — cursor down",
        0x7F => "DEL — delete",
        b if b < 0x20 => "control character",
        _ => return None,
    })
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
    fn display_control_safe_maps_controls_to_pictures() {
        // No-op for ordinary names and decoded high glyphs (kana).
        assert_eq!(display_control_safe("HELLO.PIC"), "HELLO.PIC");
        assert_eq!(display_control_safe("\u{FF71}"), "\u{FF71}");
        // Individual codes -> their control pictures.
        assert_eq!(display_control_safe("\u{00}"), "\u{2400}"); // NUL -> ␀
        assert_eq!(display_control_safe("\u{07}"), "\u{2407}"); // BEL -> ␇
        assert_eq!(display_control_safe("\u{7F}"), "\u{2421}"); // DEL -> ␡
                                                                // The crafted jaarg-hw.di1 name: BEL CR CR CR CR LF FF FF . SUB FF FF.
        assert_eq!(
            display_control_safe("\u{07}\r\r\r\r\n\u{0C}\u{0C}.\u{1A}\u{0C}\u{0C}"),
            "\u{2407}\u{240D}\u{240D}\u{240D}\u{240D}\u{240A}\u{240C}\u{240C}.\u{241A}\u{240C}\u{240C}"
        );
    }

    #[test]
    fn control_char_effect_describes_known_codes() {
        assert!(control_char_effect(0x07).unwrap().contains("beep"));
        assert!(control_char_effect(0x0C).unwrap().contains("clear"));
        assert!(control_char_effect(0x7F).is_some());
        assert!(control_char_effect(0x03).is_some()); // generic C0 fallback
                                                      // Non-control bytes have no effect description.
        assert_eq!(control_char_effect(b'A'), None);
        assert_eq!(control_char_effect(0x20), None);
    }

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
    fn russian_high_bytes_map_to_cyrillic() {
        // 0xA0 = GREEK SMALL ALPHA (shared graphics), 0xC1 = а, 0xE1 = А,
        // 0xBF = CURRENCY SIGN, 0xFF = cursor (full block).
        assert_eq!(decode_byte(MsxCharset::Russian, 0xA0), '\u{03B1}');
        assert_eq!(decode_byte(MsxCharset::Russian, 0xC1), '\u{0430}');
        assert_eq!(decode_byte(MsxCharset::Russian, 0xE1), '\u{0410}');
        assert_eq!(decode_byte(MsxCharset::Russian, 0xBF), '\u{00A4}');
        assert_eq!(decode_byte(MsxCharset::Russian, 0xFF), '\u{2588}');
    }

    #[test]
    fn korean_high_bytes_map_to_hangul() {
        // 0x86 = HANGUL LETTER KIYEOK (jamo), 0xA7 = 고 (precomposed syllable),
        // 0xFF = cursor; 0xFC..0xFE are undefined.
        assert_eq!(decode_byte(MsxCharset::Korean, 0x86), '\u{3131}');
        assert_eq!(decode_byte(MsxCharset::Korean, 0xA7), '\u{ACE0}');
        assert_eq!(decode_byte(MsxCharset::Korean, 0xFC), '\u{FFFD}');
        assert_eq!(decode_byte(MsxCharset::Korean, 0xFF), '\u{2588}');
    }

    #[test]
    fn arabic_high_bytes_map_to_arabic() {
        // 0x90 = ARABIC-INDIC DIGIT ZERO, 0xB0 = ARABIC LETTER SEEN,
        // 0x80 = undefined (bidi control in source), 0xFF = cursor.
        assert_eq!(decode_byte(MsxCharset::Arabic, 0x90), '\u{0660}');
        assert_eq!(decode_byte(MsxCharset::Arabic, 0xB0), '\u{0633}');
        assert_eq!(decode_byte(MsxCharset::Arabic, 0x80), '\u{FFFD}');
        assert_eq!(decode_byte(MsxCharset::Arabic, 0xFF), '\u{2588}');
    }

    #[test]
    fn brazilian_high_bytes_map_to_portuguese() {
        // 0x84 = Á (vs ä in International), 0x9E = CRUZEIRO SIGN,
        // 0xC0 = shared graphics, 0xFF = cursor.
        assert_eq!(decode_byte(MsxCharset::Brazilian, 0x84), '\u{00C1}');
        assert_eq!(decode_byte(MsxCharset::Brazilian, 0x9E), '\u{20A2}');
        assert_eq!(decode_byte(MsxCharset::Brazilian, 0xC0), '\u{2582}');
        assert_eq!(decode_byte(MsxCharset::Brazilian, 0xFF), '\u{2588}');
    }

    #[test]
    fn every_charset_has_a_distinct_nonempty_label() {
        let labels: Vec<&str> = MsxCharset::ALL.iter().map(|cs| cs.label()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()));
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len(), "labels must be unique");
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
    fn encode_byte_round_trips_through_decode() {
        // For every defined cell, encoding the decoded glyph yields a byte that
        // decodes back to the same glyph.
        for &cs in MsxCharset::ALL {
            for b in 0u8..=0xFF {
                let c = decode_byte(cs, b);
                if c == '\u{FFFD}' {
                    assert_eq!(encode_byte(cs, c), None, "{cs:?} undefined {b:#04X}");
                    continue;
                }
                let enc = encode_byte(cs, c).expect("defined glyph is encodable");
                assert_eq!(decode_byte(cs, enc), c, "{cs:?} byte {b:#04X}");
                // Byte-level: encoding must give the original byte back. The one
                // sanctioned exception is the synthetic cursor cell at 0xFF,
                // whose full-block glyph collides with a semigraphic cell in
                // some sets (so renames re-encode it to that cell). Any other
                // duplicate glyph is a table bug.
                if enc != b {
                    assert_eq!(
                        b, 0xFF,
                        "{cs:?} unexpected duplicate glyph at {b:#04X} ({c:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn encode_fs_name_inverts_decode_for_kana() {
        // A PUA-encoded katakana name decodes to kana, then re-encodes to the
        // exact same fatfs key.
        let key = "\u{F0B1}\u{F0B2}\u{F0B3}.BAS";
        let display = decode_fs_name(MsxCharset::Japanese, key);
        assert_ne!(display, key, "should have decoded to kana");
        assert_eq!(
            encode_fs_name(MsxCharset::Japanese, &display),
            Some(key.to_string())
        );
    }

    #[test]
    fn encode_fs_name_rejects_unrepresentable_scalar() {
        // An emoji has no Japanese byte, so the name cannot be encoded.
        assert_eq!(encode_fs_name(MsxCharset::Japanese, "\u{1F600}.BIN"), None);
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
            assert_eq!(
                conv.encode(c),
                Some(b),
                "byte {b:#04X} failed to round-trip"
            );
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
