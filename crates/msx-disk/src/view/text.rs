//! Plain-text view model.
//!
//! Renders a file's bytes as text. By default control characters become `.`
//! (matching the classic DskExplorer behaviour) while line breaks are
//! normalized so the view shows real lines; a "show all" mode instead shows
//! control bytes raw so nothing is hidden. Bytes `>= 0x80` are always decoded
//! to real glyphs through the selected MSX [`charset`](crate::charset).

/// How to treat control / non-printable bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMode {
    /// Replace control bytes with `.` (line breaks still become newlines).
    Dots,
    /// Show every byte via its Latin-1 character.
    ShowAll,
}

/// Column width of a tab stop: the M80 / MSX-DOS `TYPE` convention.
const TAB_STOP: usize = 8;

/// Decode `data` to a displayable string under `charset`.
///
/// `CR`, `LF`, and `CRLF` all collapse to a single `\n`; `TAB` expands to the
/// next [`TAB_STOP`] column (egui draws a tab as a fixed four-space advance, so
/// the stop math has to happen here for assembler sources to line up).
/// Bytes `>= 0x80` decode to real glyphs via `charset` regardless of `mode`;
/// `mode` only governs whether other control bytes are hidden as `.` or shown.
pub fn to_text(data: &[u8], mode: ControlMode, charset: crate::charset::MsxCharset) -> String {
    let mut out = String::with_capacity(data.len());
    let mut column = 0;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            0x0D => {
                out.push('\n');
                column = 0;
                if data.get(i + 1) == Some(&0x0A) {
                    i += 1; // collapse CRLF into one newline
                }
            }
            0x0A => {
                out.push('\n');
                column = 0;
            }
            0x09 => {
                let pad = TAB_STOP - column % TAB_STOP;
                out.extend(std::iter::repeat_n(' ', pad));
                column += pad;
            }
            _ => {
                out.push(match b {
                    0x20..=0x7E => b as char,
                    0x80..=0xFF => crate::charset::decode_byte(charset, b),
                    _ => match mode {
                        ControlMode::Dots => '.',
                        ControlMode::ShowAll => b as char,
                    },
                });
                column += 1;
            }
        }
        i += 1;
    }
    out
}

/// Does `data` look like a text file (English, MSX graphic characters, or
/// Japanese kana / Shift-JIS) rather than binary data?
///
/// The body is everything before the first `^Z` (0x1A), the CP/M-style end
/// marker MSX-DOS editors write; only `^Z` / NUL padding may follow it, since
/// in machine code 0x1A is just `LD A,(DE)`. The body must contain at least
/// one line break and no control bytes other than TAB, LF, FF, CR, ESC and the
/// two-byte MSX graphic sequence `0x01 0x40..=0x5F`. Every byte `>= 0x80` is a
/// real glyph under both MSX charsets, so high bytes never disqualify a file;
/// the line-break rule is what keeps `0xFF` filler out.
pub fn looks_like_text(data: &[u8]) -> bool {
    let (body, tail) = match data.iter().position(|&b| b == 0x1A) {
        Some(i) => (&data[..i], &data[i..]),
        None => (data, &data[..0]),
    };
    if tail.iter().any(|&b| b != 0x1A && b != 0x00) {
        return false;
    }
    let mut has_line_break = false;
    let mut i = 0;
    while i < body.len() {
        match body[i] {
            0x0A | 0x0D => has_line_break = true,
            0x09 | 0x0C | 0x1B | 0x20..=0x7E | 0x80..=0xFF => {}
            0x01 if matches!(body.get(i + 1), Some(0x40..=0x5F)) => i += 1,
            _ => return false,
        }
        i += 1;
    }
    has_line_break
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charset::MsxCharset;

    const INTL: MsxCharset = MsxCharset::International;
    const JP: MsxCharset = MsxCharset::Japanese;

    #[test]
    fn printable_passthrough() {
        assert_eq!(to_text(b"Hello", ControlMode::Dots, INTL), "Hello");
    }

    #[test]
    fn control_becomes_dot_in_default_mode() {
        assert_eq!(to_text(&[0x00, b'A', 0x07], ControlMode::Dots, INTL), ".A.");
    }

    #[test]
    fn high_bytes_decode_to_glyphs_even_in_dots_mode() {
        // High bytes are real glyphs, not control codes: katakana under Japanese.
        assert_eq!(
            to_text(&[0xB1, 0xB2, 0xB3], ControlMode::Dots, JP),
            "\u{FF71}\u{FF72}\u{FF73}"
        );
        // ...and accented Latin under International.
        assert_eq!(to_text(&[0x82], ControlMode::Dots, INTL), "\u{00E9}");
    }

    #[test]
    fn show_all_keeps_control_but_decodes_high_bytes() {
        // ShowAll only differs from Dots for control bytes (< 0x20 / 0x7F).
        let s = to_text(&[0x00, 0xB1], ControlMode::ShowAll, JP);
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars, vec!['\u{0}', '\u{FF71}']);
    }

    #[test]
    fn crlf_collapses_to_single_newline() {
        assert_eq!(
            to_text(b"a\r\nb\rc\nd", ControlMode::Dots, INTL),
            "a\nb\nc\nd"
        );
    }

    #[test]
    fn looks_like_text_accepts_english_japanese_and_cpm_padding() {
        assert!(looks_like_text(b"HELLO\r\nWORLD\r\n"));
        assert!(looks_like_text(b"LOOP:\tLD A,1\tGO\n"));
        // CP/M-style ^Z end marker with padding after it.
        assert!(looks_like_text(b"LINE\r\n\x1A\x1A\x1A\x00\x00"));
        // Half-width katakana (single high bytes).
        assert!(looks_like_text(&[0xB1, 0xB2, 0xB3, 0x0D, 0x0A]));
        // Shift-JIS kanji pairs.
        assert!(looks_like_text(&[0x93, 0xFA, 0x96, 0x7B, 0x0D, 0x0A]));
        // MSX two-byte graphic characters (0x01 + 0x40..0x5F) and ESC / FF.
        assert!(looks_like_text(&[
            0x01, 0x43, 0x01, 0x5F, b'x', 0x1B, b'E', 0x0C, 0x0A
        ]));
    }

    #[test]
    fn looks_like_text_rejects_binary_and_lineless_data() {
        assert!(!looks_like_text(b""));
        assert!(!looks_like_text(&[0x1A, 0x1A]));
        // BSAVE header: control bytes in the body.
        assert!(!looks_like_text(&[
            0xFE, 0x00, 0x80, 0xFF, 0xBF, 0x00, 0x80, b'\n'
        ]));
        // Erased / filler data: valid glyphs but no line structure at all.
        assert!(!looks_like_text(&[0xFF; 64]));
        assert!(!looks_like_text(b"NO LINE BREAK"));
        // 0x01 not followed by a graphic-character selector.
        assert!(!looks_like_text(&[0x01, 0x20, b'\n']));
        // 0x1A followed by anything but padding is `LD A,(DE)`, not an EOF mark.
        assert!(!looks_like_text(b"LD\r\n\x1A\xC9\x21\x00"));
    }

    #[test]
    fn tab_expands_to_eight_column_stops() {
        // egui draws a tab as a fixed 4-space advance, so the view model
        // expands to real 8-column stops (the M80 / MSX-DOS convention).
        assert_eq!(to_text(b"a\tb", ControlMode::Dots, INTL), "a       b");
        assert_eq!(
            to_text(b"LOOP:\tLD A,1", ControlMode::Dots, INTL),
            "LOOP:   LD A,1"
        );
        assert_eq!(
            to_text(b"12345678\tx", ControlMode::Dots, INTL),
            "12345678        x"
        );
        // The column resets on every line break.
        assert_eq!(
            to_text(b"ab\r\n\tc", ControlMode::Dots, INTL),
            "ab\n        c"
        );
        assert_eq!(
            to_text(b"\t\tx", ControlMode::ShowAll, INTL),
            "                x"
        );
    }
}
