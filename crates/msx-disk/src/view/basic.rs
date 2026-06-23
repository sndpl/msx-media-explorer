//! MSX-BASIC detokenizer.
//!
//! Renders a tokenized `.BAS` program as a readable listing. Token values and
//! parsing rules are ported from `mc68-net/r8format` (`bastok`), which in turn
//! cites the MSX2 Technical Handbook table 2.20.
//!
//! A tokenized file starts with a `0xFF` marker, then a sequence of lines:
//! `link(2) | line-number(2) | tokens... | 0x00`, ending when the link pointer
//! is zero. Crucially, line boundaries are found by *parsing* — a `0x00` only
//! ends a line when seen at the top level, never when it is a byte inside an
//! int/float literal. Files without the `0xFF` marker are plain ASCII and are
//! returned (lossily) as text.

use crate::charset::{decode_byte, MsxCharset};
use crate::view::text::{to_text, ControlMode};

/// Detokenize a `.BAS` file into a listing, decoding literal bytes (strings,
/// REM/DATA text) through the selected MSX `charset`.
pub fn detokenize(data: &[u8], charset: MsxCharset) -> String {
    if data.first() != Some(&0xFF) {
        // ASCII-saved BASIC: already text.
        return to_text(data, ControlMode::Dots, charset);
    }
    let mut p = Parser {
        data,
        pos: 1, // skip the 0xFF marker
        out: String::new(),
        charset,
    };
    p.parse_program();
    p.out
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    out: String,
    charset: MsxCharset,
}

impl Parser<'_> {
    fn parse_program(&mut self) {
        loop {
            // Each line begins with a 2-byte link pointer; zero means end.
            if self.pos + 4 > self.data.len() {
                break;
            }
            let link = self.read_u16();
            if link == 0 {
                break;
            }
            let line_no = self.read_u16();
            self.out.push_str(&line_no.to_string());
            self.out.push(' ');
            self.parse_line();
            self.out.push('\n');
        }
    }

    /// Parse one line's tokens until the top-level `0x00` terminator (consumed)
    /// or end of input.
    fn parse_line(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                0x00 => {
                    self.pos += 1;
                    return;
                }
                0x0B => {
                    self.pos += 1;
                    let v = self.read_u16();
                    self.out.push_str(&format!("&O{v:o}"));
                }
                0x0C => {
                    self.pos += 1;
                    let v = self.read_u16();
                    self.out.push_str(&format!("&H{v:X}"));
                }
                // 0x0D (runtime line address) shouldn't appear in saved files;
                // treat like a line-number reference for leniency.
                0x0D | 0x0E => {
                    self.pos += 1;
                    let v = self.read_u16();
                    self.out.push_str(&v.to_string());
                }
                0x0F => {
                    self.pos += 1;
                    let v = self.read_byte();
                    self.out.push_str(&v.to_string());
                }
                0x11..=0x1A => {
                    self.pos += 1;
                    self.out.push(char::from(b'0' + (b - 0x11)));
                }
                0x1C => {
                    self.pos += 1;
                    let v = self.read_u16();
                    self.out.push_str(&v.to_string());
                }
                0x1D => {
                    self.pos += 1;
                    self.real(4);
                }
                0x1F => {
                    self.pos += 1;
                    self.real(8);
                }
                0x22 => {
                    self.pos += 1;
                    self.out.push('"');
                    self.quoted();
                }
                0x3A => self.colon(),
                0x84 => {
                    self.pos += 1;
                    self.out.push_str("DATA");
                    self.data_args();
                }
                0x8F => {
                    self.pos += 1;
                    self.out.push_str("REM");
                    self.rem();
                }
                0xFF => {
                    self.pos += 1;
                    let b2 = self.read_byte();
                    match ff_token(b2) {
                        Some(kw) => self.out.push_str(kw),
                        None => self.out.push(decode_byte(self.charset, b2)),
                    }
                }
                0x81..=0xFC => {
                    self.pos += 1;
                    match single_token(b) {
                        Some(kw) => self.out.push_str(kw),
                        None => self.out.push(decode_byte(self.charset, b)),
                    }
                }
                0x20..=0x7E => {
                    self.pos += 1;
                    self.out.push(b as char);
                }
                _ => {
                    // Lenient: high bytes decode through the charset; other
                    // (control) bytes pass through as-is.
                    self.pos += 1;
                    if b >= 0x80 {
                        self.out.push(decode_byte(self.charset, b));
                    } else {
                        self.out.push(char::from(b));
                    }
                }
            }
        }
    }

    /// Handle a colon, including the `:ELSE` and `:'` (alternate REM) forms.
    fn colon(&mut self) {
        self.pos += 1; // consume ':'
        match self.peek() {
            Some(0xA1) => {
                self.pos += 1;
                self.out.push_str("ELSE");
            }
            Some(0x8F) => {
                self.pos += 1;
                if self.peek() == Some(0xE6) {
                    self.pos += 1;
                    self.out.push('\'');
                } else {
                    self.out.push(':');
                    self.out.push_str("REM");
                }
                self.rem();
            }
            _ => self.out.push(':'),
        }
    }

    /// Consume the rest of the line literally (REM comment).
    fn rem(&mut self) {
        while let Some(b) = self.peek() {
            if b == 0x00 {
                break;
            }
            self.pos += 1;
            self.out.push(decode_byte(self.charset, b));
        }
    }

    /// Consume a quoted string, including the trailing quote if present.
    fn quoted(&mut self) {
        while let Some(b) = self.peek() {
            self.pos += 1;
            self.out.push(decode_byte(self.charset, b));
            if b == 0x22 {
                return;
            }
        }
    }

    /// Consume a DATA statement's arguments up to (not including) a top-level
    /// colon or the line terminator.
    fn data_args(&mut self) {
        let mut in_quote = false;
        while let Some(b) = self.peek() {
            if b == 0x00 {
                return;
            }
            if b == 0x3A && !in_quote {
                return;
            }
            if b == 0x22 {
                in_quote = !in_quote;
            }
            self.pos += 1;
            self.out.push(decode_byte(self.charset, b));
        }
    }

    /// Decode a BCD real (single=4, double=8 bytes) and append it.
    fn real(&mut self, blen: usize) {
        if self.pos + blen > self.data.len() {
            self.pos = self.data.len();
            return;
        }
        let bs = &self.data[self.pos..self.pos + blen];
        self.pos += blen;
        let prec = if blen == 4 { '!' } else { '#' };
        let expchar = if blen == 4 { 'E' } else { 'D' };

        if bs.iter().all(|&x| x == 0) {
            self.out.push('0');
            self.out.push(prec);
            return;
        }

        let exponent = (bs[0] & 0x7F) as i32 - 0x40;
        let mut significand = String::new();
        for &byte in &bs[1..] {
            significand.push(char::from(b'0' + ((byte >> 4) & 0x0F).min(9)));
            significand.push(char::from(b'0' + (byte & 0x0F).min(9)));
        }
        let sigdigs = significand.len() as i32;

        let formatted = if exponent > 14 || exponent <= -2 {
            let fraction = significand[1..].trim_end_matches('0');
            let frac = if fraction.is_empty() {
                String::new()
            } else {
                format!(".{fraction}")
            };
            format!(
                "{}{}{}{:+}",
                &significand[0..1],
                frac,
                expchar,
                exponent - 1
            )
        } else if exponent == -1 {
            format!(".0{}{}", significand.trim_end_matches('0'), prec)
        } else if exponent == 0 {
            format!(".{}{}", significand.trim_end_matches('0'), prec)
        } else if exponent <= sigdigs {
            let e = exponent as usize;
            let v = format!("{}.{}", &significand[..e], &significand[e..]);
            format!("{}{}", v.trim_end_matches('0').trim_end_matches('.'), prec)
        } else {
            let n: u64 = significand.parse().unwrap_or(0);
            let mult = 10u64.pow((exponent - 6).max(0) as u32);
            format!("{}{}", n.saturating_mul(mult), prec)
        };
        self.out.push_str(&formatted);
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn read_byte(&mut self) -> u8 {
        let b = self.peek().unwrap_or(0);
        if self.pos < self.data.len() {
            self.pos += 1;
        }
        b
    }

    fn read_u16(&mut self) -> u16 {
        let lo = self.read_byte();
        let hi = self.read_byte();
        u16::from_le_bytes([lo, hi])
    }
}

/// Single-byte keyword tokens (0x81..=0xFC). `0xA1`/`0xE6` are only meaningful
/// after a colon and are handled there.
fn single_token(b: u8) -> Option<&'static str> {
    Some(match b {
        0x81 => "END",
        0x82 => "FOR",
        0x83 => "NEXT",
        0x84 => "DATA",
        0x85 => "INPUT",
        0x86 => "DIM",
        0x87 => "READ",
        0x88 => "LET",
        0x89 => "GOTO",
        0x8A => "RUN",
        0x8B => "IF",
        0x8C => "RESTORE",
        0x8D => "GOSUB",
        0x8E => "RETURN",
        0x8F => "REM",
        0x90 => "STOP",
        0x91 => "PRINT",
        0x92 => "CLEAR",
        0x93 => "LIST",
        0x94 => "NEW",
        0x95 => "ON",
        0x96 => "WAIT",
        0x97 => "DEF",
        0x98 => "POKE",
        0x99 => "CONT",
        0x9A => "CSAVE",
        0x9B => "CLOAD",
        0x9C => "OUT",
        0x9D => "LPRINT",
        0x9E => "LLIST",
        0x9F => "CLS",
        0xA0 => "WIDTH",
        0xA2 => "TRON",
        0xA3 => "TROFF",
        0xA4 => "SWAP",
        0xA5 => "ERASE",
        0xA6 => "ERROR",
        0xA7 => "RESUME",
        0xA8 => "DELETE",
        0xA9 => "AUTO",
        0xAA => "RENUM",
        0xAB => "DEFSTR",
        0xAC => "DEFINT",
        0xAD => "DEFSNG",
        0xAE => "DEFDBL",
        0xAF => "LINE",
        0xB0 => "OPEN",
        0xB1 => "FIELD",
        0xB2 => "GET",
        0xB3 => "PUT",
        0xB4 => "CLOSE",
        0xB5 => "LOAD",
        0xB6 => "MERGE",
        0xB7 => "FILES",
        0xB8 => "LSET",
        0xB9 => "RSET",
        0xBA => "SAVE",
        0xBB => "LFILES",
        0xBC => "CIRCLE",
        0xBD => "COLOR",
        0xBE => "DRAW",
        0xBF => "PAINT",
        0xC0 => "BEEP",
        0xC1 => "PLAY",
        0xC2 => "PSET",
        0xC3 => "PRESET",
        0xC4 => "SOUND",
        0xC5 => "SCREEN",
        0xC6 => "VPOKE",
        0xC7 => "SPRITE",
        0xC8 => "VDP",
        0xC9 => "BASE",
        0xCA => "CALL",
        0xCB => "TIME",
        0xCC => "KEY",
        0xCD => "MAX",
        0xCE => "MOTOR",
        0xCF => "BLOAD",
        0xD0 => "BSAVE",
        0xD1 => "DSKO$",
        0xD2 => "SET",
        0xD3 => "NAME",
        0xD4 => "KILL",
        0xD5 => "IPL",
        0xD6 => "COPY",
        0xD7 => "CMD",
        0xD8 => "LOCATE",
        0xD9 => "TO",
        0xDA => "THEN",
        0xDB => "TAB(",
        0xDC => "STEP",
        0xDD => "USR",
        0xDE => "FN",
        0xDF => "SPC(",
        0xE0 => "NOT",
        0xE1 => "ERL",
        0xE2 => "ERR",
        0xE3 => "STRING$",
        0xE4 => "USING",
        0xE5 => "INSTR",
        0xE7 => "VARPTR",
        0xE8 => "CSRLIN",
        0xE9 => "ATTR$",
        0xEA => "DSKI$",
        0xEB => "OFF",
        0xEC => "INKEY$",
        0xED => "POINT",
        0xEE => ">",
        0xEF => "=",
        0xF0 => "<",
        0xF1 => "+",
        0xF2 => "-",
        0xF3 => "*",
        0xF4 => "/",
        0xF5 => "^",
        0xF6 => "AND",
        0xF7 => "OR",
        0xF8 => "XOR",
        0xF9 => "EQV",
        0xFA => "IMP",
        0xFB => "MOD",
        0xFC => "\\",
        _ => return None,
    })
}

/// Two-byte function tokens (after a `0xFF` prefix), 0x81..=0xB0.
fn ff_token(b: u8) -> Option<&'static str> {
    Some(match b {
        0x81 => "LEFT$",
        0x82 => "RIGHT$",
        0x83 => "MID$",
        0x84 => "SGN",
        0x85 => "INT",
        0x86 => "ABS",
        0x87 => "SQR",
        0x88 => "RND",
        0x89 => "SIN",
        0x8A => "LOG",
        0x8B => "EXP",
        0x8C => "COS",
        0x8D => "TAN",
        0x8E => "ATN",
        0x8F => "FRE",
        0x90 => "INP",
        0x91 => "POS",
        0x92 => "LEN",
        0x93 => "STR$",
        0x94 => "VAL",
        0x95 => "ASC",
        0x96 => "CHR$",
        0x97 => "PEEK",
        0x98 => "VPEEK",
        0x99 => "SPACE$",
        0x9A => "OCT$",
        0x9B => "HEX$",
        0x9C => "LPOS",
        0x9D => "BIN$",
        0x9E => "CINT",
        0x9F => "CSNG",
        0xA0 => "CDBL",
        0xA1 => "FIX",
        0xA2 => "STICK",
        0xA3 => "STRIG",
        0xA4 => "PDL",
        0xA5 => "PAD",
        0xA6 => "DSKF",
        0xA7 => "FPOS",
        0xA8 => "CVI",
        0xA9 => "CVS",
        0xAA => "CVD",
        0xAB => "EOF",
        0xAC => "LOC",
        0xAD => "LOF",
        0xAE => "MKI$",
        0xAF => "MKS$",
        0xB0 => "MKD$",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTL: MsxCharset = MsxCharset::International;

    /// Build a one-line tokenized program around `body`.
    fn program(line_no: u16, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF];
        v.extend_from_slice(&[0x07, 0x80]); // arbitrary nonzero link
        v.extend_from_slice(&line_no.to_le_bytes());
        v.extend_from_slice(body);
        v.push(0x00); // line terminator
        v.extend_from_slice(&[0x00, 0x00]); // end-of-program link
        v
    }

    #[test]
    fn detokenizes_print_string() {
        // 10 PRINT"HI"
        let prog = program(10, &[0x91, 0x22, b'H', b'I', 0x22]);
        assert_eq!(detokenize(&prog, INTL), "10 PRINT\"HI\"\n");
    }

    #[test]
    fn zero_byte_inside_int_is_not_a_terminator() {
        // 10 GOTO 256  (256 = 0x0100, contains a 0x00 byte inside the int16)
        let prog = program(10, &[0x89, 0x20, 0x0E, 0x00, 0x01]);
        assert_eq!(detokenize(&prog, INTL), "10 GOTO 256\n");
    }

    #[test]
    fn handles_else_token() {
        // 20 IF A THEN B ELSE C  (ELSE is encoded as ':' + 0xA1; spaces are
        // stored as literal 0x20 bytes by MSX-BASIC).
        let prog = program(
            20,
            &[
                0x8B, 0x20, b'A', 0x20, 0xDA, 0x20, b'B', 0x20, 0x3A, 0xA1, 0x20, b'C',
            ],
        );
        assert_eq!(detokenize(&prog, INTL), "20 IF A THEN B ELSE C\n");
    }

    #[test]
    fn two_byte_function_token() {
        // 30 PRINT LEFT$("X",1) -> LEFT$ is 0xFF 0x81
        let prog = program(
            30,
            &[0x91, 0xFF, 0x81, 0x28, 0x22, b'X', 0x22, 0x2C, 0x12, 0x29],
        );
        assert_eq!(detokenize(&prog, INTL), "30 PRINTLEFT$(\"X\",1)\n");
    }

    #[test]
    fn single_precision_float() {
        // 1.5 single-precision: exponent 0x41, BCD 15 00 00
        let prog = program(40, &[0x1D, 0x41, 0x15, 0x00, 0x00]);
        assert_eq!(detokenize(&prog, INTL), "40 1.5!\n");
    }

    #[test]
    fn ascii_saved_basic_passes_through() {
        let out = detokenize(b"10 PRINT\r\n20 END\r\n", INTL);
        assert_eq!(out, "10 PRINT\n20 END\n");
    }

    #[test]
    fn rem_keeps_rest_of_line_literal() {
        // 10 REM hi:there  (colon inside REM stays literal)
        let prog = program(10, &[0x8F, b' ', b'h', b'i', 0x3A, b't']);
        assert_eq!(detokenize(&prog, INTL), "10 REM hi:t\n");
    }

    #[test]
    fn data_args_decode_japanese_kana() {
        // 10 DATA<kana>  with the user's reported high bytes in a DATA statement.
        let prog = program(10, &[0x84, 0xE9, 0xCC, 0xA7, 0xB2, 0xD9]);
        assert_eq!(
            detokenize(&prog, MsxCharset::Japanese),
            "10 DATA\u{306E}\u{FF8C}\u{FF67}\u{FF72}\u{FF99}\n"
        );
    }

    #[test]
    fn quoted_string_decodes_kana() {
        // 10 PRINT"<kana>"  -> katakana inside a string literal.
        let prog = program(10, &[0x91, 0x22, 0xB1, 0xB2, 0x22]);
        assert_eq!(
            detokenize(&prog, MsxCharset::Japanese),
            "10 PRINT\"\u{FF71}\u{FF72}\"\n"
        );
    }
}
