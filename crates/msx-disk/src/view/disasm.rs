//! Z80 (and R800) disassembler view model.
//!
//! Turns a machine-code file into an assembly listing. The decoder follows the
//! regular octal structure of the Z80 opcode map (the `x`/`y`/`z`/`p`/`q`
//! bit-field method described at <http://www.z80.info/decoding.htm>), so the
//! full documented *and* undocumented instruction set — `SLL`, the `IXH`/`IXL`
//! register halves, the undocumented `DDCB`/`FDCB` "result also stored in a
//! register" forms, the duplicated `ED` opcodes — falls out of a handful of
//! small tables rather than a 256-entry literal map. Mnemonics and operand
//! syntax follow the reference table at
//! <https://map.grauw.nl/resources/z80instr.php>.
//!
//! R800 is opcode-compatible with the Z80 and only adds `MULUB A,r` / `MULUW
//! HL,ss` in `ED` slots that are undefined no-ops on the Z80, so they are always
//! recognised — decoding the R800 superset never mis-reads real Z80 code.
//!
//! The view layer is UI-free: [`disassemble`] returns a plain `String`, exactly
//! like the BASIC detokenizer. [`locate`] derives the load address and code
//! window from the file's name and any BSAVE header.

use crate::fileinfo::{self, bload};
use std::fmt::Write;

const R: [&str; 8] = ["B", "C", "D", "E", "H", "L", "(HL)", "A"];
const RP: [&str; 4] = ["BC", "DE", "HL", "SP"];
const RP2: [&str; 4] = ["BC", "DE", "HL", "AF"];
const CC: [&str; 8] = ["NZ", "Z", "NC", "C", "PO", "PE", "P", "M"];
const ALU: [&str; 8] = [
    "ADD A,", "ADC A,", "SUB ", "SBC A,", "AND ", "XOR ", "OR ", "CP ",
];
const ROT: [&str; 8] = ["RLC", "RRC", "RL", "RR", "SLA", "SRA", "SLL", "SRL"];
const IM: [&str; 8] = ["0", "0/1", "1", "2", "0", "0/1", "1", "2"];
const BLI: [[&str; 4]; 4] = [
    ["LDI", "CPI", "INI", "OUTI"],
    ["LDD", "CPD", "IND", "OUTD"],
    ["LDIR", "CPIR", "INIR", "OTIR"],
    ["LDDR", "CPDR", "INDR", "OTDR"],
];

/// The execution origin and code window for a file, derived from its name and
/// any BSAVE header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeImage<'a> {
    /// Address the first byte of `code` loads at.
    pub origin: u16,
    /// The bytes to disassemble (BSAVE header skipped when present).
    pub code: &'a [u8],
    /// Execution entry address, when the file declares one (BSAVE `exec`).
    pub exec: Option<u16>,
}

/// Decide where a file's code loads and which bytes are code:
/// - `.com`/`.cpm` → origin `0x0100` (MSX-DOS TPA), whole file is code;
/// - `.bin` with a BSAVE header → origin = `start`, the 7-byte header skipped,
///   `exec` carried through when non-zero;
/// - anything else (incl. headerless `.bin`) → origin `0x0000`, whole file.
pub fn locate<'a>(name: &str, bytes: &'a [u8]) -> CodeImage<'a> {
    match fileinfo::extension(name).as_deref() {
        Some("com") | Some("cpm") => CodeImage {
            origin: 0x0100,
            code: bytes,
            exec: None,
        },
        Some("bin") => match bload::parse(bytes) {
            Some(h) => CodeImage {
                origin: h.start,
                code: bytes.get(7..).unwrap_or(&[]),
                exec: (h.exec != 0).then_some(h.exec),
            },
            None => CodeImage {
                origin: 0,
                code: bytes,
                exec: None,
            },
        },
        _ => CodeImage {
            origin: 0,
            code: bytes,
            exec: None,
        },
    }
}

/// Disassemble `code` as Z80/R800, with the first byte at `origin`. A leading
/// comment block records the origin and (when known) the execution address.
pub fn disassemble(code: &[u8], origin: u16, exec: Option<u16>) -> String {
    // Each line is roughly "XXXX  HH HH ...  MNEMONIC\n"; pre-size to avoid
    // reallocations as the listing grows over a large code window.
    let mut out = String::with_capacity(code.len() * 12 + 32);
    let _ = writeln!(out, "; origin 0x{origin:04X}");
    if let Some(e) = exec {
        let _ = writeln!(out, "; exec   0x{e:04X}");
    }
    let mut pos = 0usize;
    let mut pc = origin;
    let mut hex = String::new();
    while pos < code.len() {
        let (len, text) = decode(&code[pos..], pc);
        let end = (pos + len).min(code.len());
        hex.clear();
        for b in &code[pos..end] {
            let _ = write!(hex, "{b:02X} ");
        }
        let _ = writeln!(out, "{pc:04X}  {hex:<13}{text}");
        pos += len;
        pc = pc.wrapping_add(len as u16);
    }
    out
}

/// A byte cursor over the instruction stream; reads past the end return `None`,
/// which propagates up to a `DB` fallback for the truncated tail.
struct Cur<'a> {
    code: &'a [u8],
    pos: usize,
}

impl Cur<'_> {
    fn peek(&self) -> Option<u8> {
        self.code.get(self.pos).copied()
    }
    fn u8(&mut self) -> Option<u8> {
        let b = self.code.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }
    fn nn(&mut self) -> Option<u16> {
        let lo = self.u8()? as u16;
        let hi = self.u8()? as u16;
        Some(lo | (hi << 8))
    }
}

/// Decode one instruction at the front of `code`, where the first byte sits at
/// address `pc`. Returns the byte length consumed and the mnemonic text;
/// unrecognised or truncated input becomes a `DB` data directive.
fn decode(code: &[u8], pc: u16) -> (usize, String) {
    match try_decode(code, pc) {
        Some((len, text)) if len > 0 => (len, text),
        _ => db_fallback(code),
    }
}

fn try_decode(code: &[u8], pc: u16) -> Option<(usize, String)> {
    let mut cur = Cur { code, pos: 0 };
    let text = match cur.peek()? {
        0xCB => {
            cur.u8();
            let op2 = cur.u8()?;
            decode_cb(op2)
        }
        0xED => {
            cur.u8();
            decode_ed(&mut cur)?
        }
        0xDD => {
            cur.u8();
            decode_indexed(&mut cur, pc, 0xDD)?
        }
        0xFD => {
            cur.u8();
            decode_indexed(&mut cur, pc, 0xFD)?
        }
        _ => decode_main(&mut cur, pc, None)?,
    };
    Some((cur.pos, text))
}

/// Emit the remaining bytes as a `DB` directive (unknown opcode or truncation).
fn db_fallback(code: &[u8]) -> (usize, String) {
    let mut s = String::from("DB ");
    for (i, b) in code.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let _ = write!(s, "0x{b:02X}");
    }
    (code.len().max(1), s)
}

/// Relative-jump target: address after the 2-byte instruction plus the signed
/// displacement.
fn rel(pc: u16, d: i8) -> u16 {
    pc.wrapping_add(2).wrapping_add(d as i16 as u16)
}

/// `(IX+d)` / `(IY-d)` with a signed-hex displacement.
fn disp(idx: &str, d: i8) -> String {
    if d >= 0 {
        format!("({idx}+0x{:02X})", d as u8)
    } else {
        format!("({idx}-0x{:02X})", d.unsigned_abs())
    }
}

/// 16-bit register pair, substituting `HL`→`IX`/`IY` under an index prefix.
fn rp(p: usize, idx: Option<&str>) -> &str {
    match (p, idx) {
        (2, Some(x)) => x,
        _ => RP[p],
    }
}

fn rp2(p: usize, idx: Option<&str>) -> &str {
    match (p, idx) {
        (2, Some(x)) => x,
        _ => RP2[p],
    }
}

/// A register operand, reading the `(IX+d)` displacement when index `i` selects
/// `(HL)` under a prefix. `suppress_half` keeps `H`/`L` un-renamed when the
/// instruction's other operand is the indexed memory cell (e.g. `LD H,(IX+d)`).
fn reg(cur: &mut Cur, i: usize, idx: Option<&str>, suppress_half: bool) -> Option<String> {
    if i == 6 {
        return match idx {
            None => Some("(HL)".to_string()),
            Some(x) => Some(disp(x, cur.u8()? as i8)),
        };
    }
    Some(reg_half(i, idx, suppress_half))
}

fn reg_half(i: usize, idx: Option<&str>, suppress_half: bool) -> String {
    if let Some(x) = idx {
        if !suppress_half && i == 4 {
            return format!("{x}H");
        }
        if !suppress_half && i == 5 {
            return format!("{x}L");
        }
    }
    R[i].to_string()
}

/// Main (unprefixed) table; `idx` carries `IX`/`IY` substitution under a DD/FD
/// prefix. Reads the opcode and any operand bytes from `cur`.
fn decode_main(cur: &mut Cur, pc: u16, idx: Option<&str>) -> Option<String> {
    let op = cur.u8()?;
    let x = op >> 6;
    let y = ((op >> 3) & 7) as usize;
    let z = (op & 7) as usize;
    let p = y >> 1;
    let q = y & 1;
    let hl = idx.unwrap_or("HL");

    let s = match x {
        0 => match z {
            0 => match y {
                0 => "NOP".to_string(),
                1 => "EX AF,AF'".to_string(),
                2 => format!("DJNZ 0x{:04X}", rel(pc, cur.u8()? as i8)),
                3 => format!("JR 0x{:04X}", rel(pc, cur.u8()? as i8)),
                _ => format!("JR {},0x{:04X}", CC[y - 4], rel(pc, cur.u8()? as i8)),
            },
            1 if q == 0 => format!("LD {},0x{:04X}", rp(p, idx), cur.nn()?),
            1 => format!("ADD {hl},{}", rp(p, idx)),
            2 => match (q, p) {
                (0, 0) => "LD (BC),A".to_string(),
                (0, 1) => "LD (DE),A".to_string(),
                (0, 2) => format!("LD (0x{:04X}),{hl}", cur.nn()?),
                (0, _) => format!("LD (0x{:04X}),A", cur.nn()?),
                (1, 0) => "LD A,(BC)".to_string(),
                (1, 1) => "LD A,(DE)".to_string(),
                (1, 2) => format!("LD {hl},(0x{:04X})", cur.nn()?),
                (_, _) => format!("LD A,(0x{:04X})", cur.nn()?),
            },
            3 if q == 0 => format!("INC {}", rp(p, idx)),
            3 => format!("DEC {}", rp(p, idx)),
            4 => format!("INC {}", reg(cur, y, idx, false)?),
            5 => format!("DEC {}", reg(cur, y, idx, false)?),
            6 => {
                let r = reg(cur, y, idx, false)?;
                format!("LD {r},0x{:02X}", cur.u8()?)
            }
            _ => match y {
                0 => "RLCA",
                1 => "RRCA",
                2 => "RLA",
                3 => "RRA",
                4 => "DAA",
                5 => "CPL",
                6 => "SCF",
                _ => "CCF",
            }
            .to_string(),
        },
        1 => {
            if y == 6 && z == 6 {
                "HALT".to_string()
            } else if z == 6 {
                let m = reg(cur, 6, idx, false)?;
                format!("LD {},{m}", reg_half(y, idx, true))
            } else if y == 6 {
                let m = reg(cur, 6, idx, false)?;
                format!("LD {m},{}", reg_half(z, idx, true))
            } else {
                let d = reg(cur, y, idx, false)?;
                let s = reg(cur, z, idx, false)?;
                format!("LD {d},{s}")
            }
        }
        2 => format!("{}{}", ALU[y], reg(cur, z, idx, false)?),
        _ => match z {
            0 => format!("RET {}", CC[y]),
            1 if q == 0 => format!("POP {}", rp2(p, idx)),
            1 => match p {
                0 => "RET".to_string(),
                1 => "EXX".to_string(),
                2 => format!("JP ({hl})"),
                _ => format!("LD SP,{hl}"),
            },
            2 => format!("JP {},0x{:04X}", CC[y], cur.nn()?),
            3 => match y {
                0 => format!("JP 0x{:04X}", cur.nn()?),
                2 => format!("OUT (0x{:02X}),A", cur.u8()?),
                3 => format!("IN A,(0x{:02X})", cur.u8()?),
                4 => format!("EX (SP),{hl}"),
                5 => "EX DE,HL".to_string(),
                6 => "DI".to_string(),
                _ => "EI".to_string(),
            },
            4 => format!("CALL {},0x{:04X}", CC[y], cur.nn()?),
            5 if q == 0 => format!("PUSH {}", rp2(p, idx)),
            5 => format!("CALL 0x{:04X}", cur.nn()?),
            6 => format!("{}0x{:02X}", ALU[y], cur.u8()?),
            _ => format!("RST 0x{:02X}", (y as u8) * 8),
        },
    };
    Some(s)
}

/// `CB`-prefixed: rotates/shifts (incl. undocumented `SLL`) and bit ops.
fn decode_cb(op: u8) -> String {
    let x = op >> 6;
    let y = ((op >> 3) & 7) as usize;
    let z = (op & 7) as usize;
    match x {
        0 => format!("{} {}", ROT[y], R[z]),
        1 => format!("BIT {y},{}", R[z]),
        2 => format!("RES {y},{}", R[z]),
        _ => format!("SET {y},{}", R[z]),
    }
}

/// `DDCB`/`FDCB`: a rotate/shift/bit op on `(IX+d)`/`(IY+d)`. For the
/// undocumented `z != 6` forms the result is also stored in register `R[z]`.
fn decode_ddcb(d: i8, op: u8, idx: &str) -> String {
    let x = op >> 6;
    let y = ((op >> 3) & 7) as usize;
    let z = (op & 7) as usize;
    let m = disp(idx, d);
    match x {
        0 if z != 6 => format!("{} {m},{}", ROT[y], R[z]),
        0 => format!("{} {m}", ROT[y]),
        1 => format!("BIT {y},{m}"),
        2 if z != 6 => format!("RES {y},{m},{}", R[z]),
        2 => format!("RES {y},{m}"),
        _ if z != 6 => format!("SET {y},{m},{}", R[z]),
        _ => format!("SET {y},{m}"),
    }
}

/// `ED`-prefixed: block ops, 16-bit arithmetic, I/O, plus the R800 multiplies.
fn decode_ed(cur: &mut Cur) -> Option<String> {
    let op = cur.u8()?;
    let x = op >> 6;
    let y = ((op >> 3) & 7) as usize;
    let z = (op & 7) as usize;
    let p = y >> 1;
    let q = y & 1;

    let s = match x {
        1 => match z {
            0 if y == 6 => "IN F,(C)".to_string(),
            0 => format!("IN {},(C)", R[y]),
            1 if y == 6 => "OUT (C),0".to_string(),
            1 => format!("OUT (C),{}", R[y]),
            2 if q == 0 => format!("SBC HL,{}", RP[p]),
            2 => format!("ADC HL,{}", RP[p]),
            3 if q == 0 => format!("LD (0x{:04X}),{}", cur.nn()?, RP[p]),
            3 => format!("LD {},(0x{:04X})", RP[p], cur.nn()?),
            4 => "NEG".to_string(),
            5 if y == 1 => "RETI".to_string(),
            5 => "RETN".to_string(),
            6 => format!("IM {}", IM[y]),
            _ => match y {
                0 => "LD I,A",
                1 => "LD R,A",
                2 => "LD A,I",
                3 => "LD A,R",
                4 => "RRD",
                5 => "RLD",
                _ => "NOP",
            }
            .to_string(),
        },
        2 if z <= 3 && y >= 4 => BLI[y - 4][z].to_string(),
        // R800 multiply opcodes live in the otherwise-undefined `x == 3` block.
        3 if z == 1 && y <= 3 => format!("MULUB A,{}", R[y]),
        3 if z == 3 && q == 0 && (p == 0 || p == 3) => format!("MULUW HL,{}", RP[p]),
        _ => format!("DB 0xED, 0x{op:02X}"),
    };
    Some(s)
}

/// `DD`/`FD`-prefixed dispatch. A `DDCB`/`FDCB` reads displacement-then-opcode;
/// a prefix immediately followed by another prefix re-syncs (the leading prefix
/// is shown as a `DB` and the next byte decodes on its own).
fn decode_indexed(cur: &mut Cur, pc: u16, pfx: u8) -> Option<String> {
    let idx = if pfx == 0xDD { "IX" } else { "IY" };
    match cur.peek()? {
        0xCB => {
            cur.u8();
            let d = cur.u8()? as i8;
            let op = cur.u8()?;
            Some(decode_ddcb(d, op, idx))
        }
        0xDD | 0xFD | 0xED => Some(format!("DB 0x{pfx:02X}")),
        _ => decode_main(cur, pc, Some(idx)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Disassemble one instruction's worth of bytes at `pc`, returning just the
    /// mnemonic text.
    fn one(bytes: &[u8]) -> String {
        decode(bytes, 0x0100).1
    }

    fn one_at(bytes: &[u8], pc: u16) -> String {
        decode(bytes, pc).1
    }

    #[test]
    fn main_table_basics() {
        assert_eq!(one(&[0x00]), "NOP");
        assert_eq!(one(&[0x76]), "HALT");
        assert_eq!(one(&[0x3E, 0x05]), "LD A,0x05");
        assert_eq!(one(&[0x01, 0x34, 0x12]), "LD BC,0x1234");
        assert_eq!(one(&[0x41]), "LD B,C");
        assert_eq!(one(&[0x46]), "LD B,(HL)");
        assert_eq!(one(&[0x77]), "LD (HL),A");
        assert_eq!(one(&[0x04]), "INC B");
        assert_eq!(one(&[0x09]), "ADD HL,BC");
        assert_eq!(one(&[0x32, 0x00, 0xC0]), "LD (0xC000),A");
        assert_eq!(one(&[0x2A, 0x00, 0xC0]), "LD HL,(0xC000)");
        assert_eq!(one(&[0xC9]), "RET");
        assert_eq!(one(&[0xE9]), "JP (HL)");
        assert_eq!(one(&[0xEB]), "EX DE,HL");
        assert_eq!(one(&[0x08]), "EX AF,AF'");
        assert_eq!(one(&[0xD9]), "EXX");
        assert_eq!(one(&[0xF3]), "DI");
        assert_eq!(one(&[0xFF]), "RST 0x38");
        assert_eq!(one(&[0xC7]), "RST 0x00");
        assert_eq!(one(&[0xD3, 0xA8]), "OUT (0xA8),A");
        assert_eq!(one(&[0xDB, 0xA8]), "IN A,(0xA8)");
    }

    #[test]
    fn alu_and_immediate() {
        assert_eq!(one(&[0x80]), "ADD A,B");
        assert_eq!(one(&[0x90]), "SUB B");
        assert_eq!(one(&[0xA8]), "XOR B");
        assert_eq!(one(&[0xBE]), "CP (HL)");
        assert_eq!(one(&[0xC6, 0x10]), "ADD A,0x10");
        assert_eq!(one(&[0xFE, 0x00]), "CP 0x00");
    }

    #[test]
    fn calls_and_conditional_jumps() {
        assert_eq!(one(&[0xCD, 0x00, 0x40]), "CALL 0x4000");
        assert_eq!(one(&[0xC3, 0x00, 0x40]), "JP 0x4000");
        assert_eq!(one(&[0xC2, 0x34, 0x12]), "JP NZ,0x1234");
        assert_eq!(one(&[0xCC, 0x34, 0x12]), "CALL Z,0x1234");
        assert_eq!(one(&[0xC0]), "RET NZ");
    }

    #[test]
    fn relative_jumps_resolve_to_absolute_targets() {
        // JR +0 at 0x0100: 0x0100 + 2 + 0 = 0x0102.
        assert_eq!(one_at(&[0x18, 0x00], 0x0100), "JR 0x0102");
        // JR -2 (tight loop): 0x0100 + 2 - 2 = 0x0100.
        assert_eq!(one_at(&[0x18, 0xFE], 0x0100), "JR 0x0100");
        // DJNZ +5 at 0x0200: 0x0200 + 2 + 5 = 0x0207.
        assert_eq!(one_at(&[0x10, 0x05], 0x0200), "DJNZ 0x0207");
        // JR NZ backwards across a page boundary wraps in u16.
        assert_eq!(one_at(&[0x20, 0x80], 0x0050), "JR NZ,0xFFD2");
    }

    #[test]
    fn cb_prefixed() {
        assert_eq!(one(&[0xCB, 0x00]), "RLC B");
        assert_eq!(one(&[0xCB, 0x06]), "RLC (HL)");
        assert_eq!(one(&[0xCB, 0x3F]), "SRL A");
        // Undocumented SLL (a.k.a. SL1) sits between SRA and SRL.
        assert_eq!(one(&[0xCB, 0x30]), "SLL B");
        assert_eq!(one(&[0xCB, 0x7E]), "BIT 7,(HL)");
        assert_eq!(one(&[0xCB, 0x47]), "BIT 0,A");
        assert_eq!(one(&[0xCB, 0x86]), "RES 0,(HL)");
        assert_eq!(one(&[0xCB, 0xFF]), "SET 7,A");
    }

    #[test]
    fn ed_prefixed() {
        assert_eq!(one(&[0xED, 0xB0]), "LDIR");
        assert_eq!(one(&[0xED, 0xA0]), "LDI");
        assert_eq!(one(&[0xED, 0xB8]), "LDDR");
        assert_eq!(one(&[0xED, 0x42]), "SBC HL,BC");
        assert_eq!(one(&[0xED, 0x4A]), "ADC HL,BC");
        assert_eq!(one(&[0xED, 0x43, 0x00, 0xC0]), "LD (0xC000),BC");
        assert_eq!(one(&[0xED, 0x4B, 0x00, 0xC0]), "LD BC,(0xC000)");
        assert_eq!(one(&[0xED, 0x44]), "NEG");
        assert_eq!(one(&[0xED, 0x4D]), "RETI");
        assert_eq!(one(&[0xED, 0x45]), "RETN");
        assert_eq!(one(&[0xED, 0x5E]), "IM 2");
        assert_eq!(one(&[0xED, 0x47]), "LD I,A");
        assert_eq!(one(&[0xED, 0x67]), "RRD");
        // Undocumented ED I/O forms.
        assert_eq!(one(&[0xED, 0x70]), "IN F,(C)");
        assert_eq!(one(&[0xED, 0x71]), "OUT (C),0");
        // Genuinely-undefined ED opcode becomes data.
        assert_eq!(one(&[0xED, 0x00]), "DB 0xED, 0x00");
    }

    #[test]
    fn r800_multiplies() {
        assert_eq!(one(&[0xED, 0xC1]), "MULUB A,B");
        assert_eq!(one(&[0xED, 0xC9]), "MULUB A,C");
        assert_eq!(one(&[0xED, 0xD1]), "MULUB A,D");
        assert_eq!(one(&[0xED, 0xD9]), "MULUB A,E");
        assert_eq!(one(&[0xED, 0xC3]), "MULUW HL,BC");
        assert_eq!(one(&[0xED, 0xF3]), "MULUW HL,SP");
    }

    #[test]
    fn dd_fd_indexed() {
        assert_eq!(one(&[0xDD, 0x21, 0x00, 0xC0]), "LD IX,0xC000");
        assert_eq!(one(&[0xFD, 0x21, 0x00, 0xC0]), "LD IY,0xC000");
        assert_eq!(one(&[0xDD, 0x09]), "ADD IX,BC");
        assert_eq!(one(&[0xDD, 0x29]), "ADD IX,IX");
        assert_eq!(one(&[0xDD, 0xE5]), "PUSH IX");
        assert_eq!(one(&[0xDD, 0xE9]), "JP (IX)");
        // (IX+d) memory operands with signed displacements.
        assert_eq!(one(&[0xDD, 0x7E, 0x05]), "LD A,(IX+0x05)");
        assert_eq!(one(&[0xDD, 0x70, 0xFB]), "LD (IX-0x05),B");
        assert_eq!(one(&[0xDD, 0x36, 0x02, 0xFF]), "LD (IX+0x02),0xFF");
        assert_eq!(one(&[0xDD, 0x34, 0x00]), "INC (IX+0x00)");
        assert_eq!(one(&[0xDD, 0x86, 0x04]), "ADD A,(IX+0x04)");
        // Undocumented IX register halves.
        assert_eq!(one(&[0xDD, 0x26, 0x10]), "LD IXH,0x10");
        assert_eq!(one(&[0xDD, 0x65]), "LD IXH,IXL");
        // (HL)-form keeps the other register un-renamed: LD H,(IX+d), not IXH.
        assert_eq!(one(&[0xDD, 0x66, 0x01]), "LD H,(IX+0x01)");
    }

    #[test]
    fn ddcb_fdcb() {
        assert_eq!(one(&[0xDD, 0xCB, 0x05, 0x46]), "BIT 0,(IX+0x05)");
        assert_eq!(one(&[0xFD, 0xCB, 0xFE, 0x7E]), "BIT 7,(IY-0x02)");
        assert_eq!(one(&[0xDD, 0xCB, 0x05, 0x06]), "RLC (IX+0x05)");
        // Undocumented "result also stored in register" form.
        assert_eq!(one(&[0xDD, 0xCB, 0x05, 0x00]), "RLC (IX+0x05),B");
        assert_eq!(one(&[0xDD, 0xCB, 0x05, 0xC0]), "SET 0,(IX+0x05),B");
    }

    #[test]
    fn truncated_and_unknown_become_data() {
        // CALL needs two address bytes; only one present.
        assert_eq!(one(&[0xCD, 0x00]), "DB 0xCD, 0x00");
        // Lone DD at end of stream.
        assert_eq!(one(&[0xDD]), "DB 0xDD");
        // DD followed by another prefix re-syncs: only the DD is consumed here.
        assert_eq!(decode(&[0xDD, 0xFD, 0x00], 0).0, 1);
        assert_eq!(decode(&[0xDD, 0xFD, 0x00], 0).1, "DB 0xDD");
    }

    #[test]
    fn instruction_lengths_advance_correctly() {
        assert_eq!(decode(&[0x00], 0).0, 1);
        assert_eq!(decode(&[0x3E, 0x05], 0).0, 2);
        assert_eq!(decode(&[0xCD, 0x00, 0x40], 0).0, 3);
        assert_eq!(decode(&[0xDD, 0xCB, 0x05, 0x46], 0).0, 4);
        assert_eq!(decode(&[0xED, 0x43, 0x00, 0xC0], 0).0, 4);
    }

    #[test]
    fn disassemble_lists_addresses_and_bytes() {
        // LD A,5 ; RET  at origin 0x0100.
        let out = disassemble(&[0x3E, 0x05, 0xC9], 0x0100, Some(0x0100));
        assert!(out.contains("; origin 0x0100"));
        assert!(out.contains("; exec   0x0100"));
        let mut lines = out.lines().filter(|l| !l.starts_with(';'));
        assert_eq!(lines.next().unwrap(), "0100  3E 05        LD A,0x05");
        assert_eq!(lines.next().unwrap(), "0102  C9           RET");
    }

    #[test]
    fn locate_picks_origin_by_type() {
        // .com: TPA origin, whole file is code.
        let img = locate("GAME.COM", &[0xC9]);
        assert_eq!(img.origin, 0x0100);
        assert_eq!(img.code, &[0xC9]);
        assert_eq!(img.exec, None);

        // .bin with BSAVE header: origin = start, header skipped, exec carried.
        let bin = [0xFE, 0x00, 0x80, 0x02, 0x80, 0x00, 0x80, 0xC9, 0xC9, 0xC9];
        let img = locate("CODE.BIN", &bin);
        assert_eq!(img.origin, 0x8000);
        assert_eq!(img.exec, Some(0x8000));
        assert_eq!(img.code, &bin[7..]);

        // .bin without a header falls back to 0x0000.
        let img = locate("RAW.BIN", &[0x00, 0x01, 0x02]);
        assert_eq!(img.origin, 0);
        assert_eq!(img.code, &[0x00, 0x01, 0x02]);

        // Unknown extension: origin 0x0000, whole file.
        let img = locate("DATA.XYZ", &[0xAA]);
        assert_eq!(img.origin, 0);
        assert_eq!(img.code, &[0xAA]);
    }
}
