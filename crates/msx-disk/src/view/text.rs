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

/// Decode `data` to a displayable string under `charset`.
///
/// `CR`, `LF`, and `CRLF` all collapse to a single `\n`; `TAB` is preserved.
/// Bytes `>= 0x80` decode to real glyphs via `charset` regardless of `mode`;
/// `mode` only governs whether other control bytes are hidden as `.` or shown.
pub fn to_text(data: &[u8], mode: ControlMode, charset: crate::charset::MsxCharset) -> String {
    let mut out = String::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            0x0D => {
                out.push('\n');
                if data.get(i + 1) == Some(&0x0A) {
                    i += 1; // collapse CRLF into one newline
                }
            }
            0x0A => out.push('\n'),
            0x09 => out.push('\t'),
            0x20..=0x7E => out.push(b as char),
            0x80..=0xFF => out.push(crate::charset::decode_byte(charset, b)),
            _ => match mode {
                ControlMode::Dots => out.push('.'),
                ControlMode::ShowAll => out.push(b as char),
            },
        }
        i += 1;
    }
    out
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
    fn tab_preserved() {
        assert_eq!(to_text(b"a\tb", ControlMode::Dots, INTL), "a\tb");
    }
}
