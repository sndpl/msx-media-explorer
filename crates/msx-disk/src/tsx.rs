//! `.tsx` — MSX tape images: TZX 1.21 plus the MSX-specific `#4B` (Kansas City
//! Standard) block.
//!
//! We decode the `#4B` data blocks (the same logical byte stream a CAS block
//! holds) and the `#35` custom-info and `#32` archive-info blocks for the block
//! overview, and skip the remaining standard TZX blocks. Parsing stops at the
//! first block id we do not recognize rather than risk mis-reading the stream.
//!
//! Reference: openMSX `TsxParser`, the World of Spectrum TZX spec, and the
//! makeTSX `#4B` block definition.

use crate::cas::header_of;
use crate::tape::{TapeBlock, TapeHeader};

const MAGIC: &[u8] = b"ZXTape!\x1A";
/// Magic (8) + version major/minor (2).
const HEADER_LEN: usize = 10;

/// Parse a TSX/TZX file into ordered tape blocks. Returns empty if the magic
/// header is absent.
pub fn parse(bytes: &[u8]) -> Vec<TapeBlock> {
    if bytes.len() < HEADER_LEN || !bytes.starts_with(MAGIC) {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    let mut pos = HEADER_LEN;
    while pos < bytes.len() {
        let Some((block, next)) = parse_block(bytes, pos) else {
            break; // unknown block id or truncation: stop cleanly
        };
        if next <= pos {
            break; // guard against a zero-length advance
        }
        if let Some(b) = block {
            blocks.push(b);
        }
        pos = next;
    }
    blocks
}

/// Parse the block at `pos`, returning the block (if displayed) and the start of
/// the next block, or `None` to stop.
fn parse_block(bytes: &[u8], pos: usize) -> Option<(Option<TapeBlock>, usize)> {
    let id = *bytes.get(pos)?;
    match id {
        // MSX Kansas City Standard data block.
        0x4B => {
            let len = rd_u32(bytes, pos + 1)?;
            let body = pos + 5;
            let data = bytes.get(body + 12..body + len)?.to_vec();
            let header = header_of(&data).map(|(kind, name)| TapeHeader { kind, name });
            Some((Some(TapeBlock::Msx { header, data }), body + len))
        }
        // Custom info block (e.g. ripper / loader notes).
        0x35 => {
            let id_bytes = bytes.get(pos + 1..pos + 17)?;
            let len = rd_u32(bytes, pos + 17)?;
            let data = bytes.get(pos + 21..pos + 21 + len)?;
            Some((
                Some(TapeBlock::CustomInfo {
                    id: ascii_trim(id_bytes),
                    text: ascii_trim(data),
                }),
                pos + 21 + len,
            ))
        }
        // Archive info block (title / publisher / year / ...).
        0x32 => {
            let len = rd_u16(bytes, pos + 1)?;
            let body = bytes.get(pos + 3..pos + 3 + len)?;
            Some((
                Some(TapeBlock::ArchiveInfo(parse_archive_info(body))),
                pos + 3 + len,
            ))
        }
        // Recognized blocks we list but do not decode; compute their length.
        _ => {
            let len = block_len(bytes, pos, id)?;
            Some((Some(TapeBlock::Other { id, len }), pos + len))
        }
    }
}

/// Total length in bytes (including the id byte) of a standard TZX block we skip.
fn block_len(bytes: &[u8], pos: usize, id: u8) -> Option<usize> {
    let len = match id {
        0x10 => 5 + rd_u16(bytes, pos + 3)?,
        0x11 => 19 + rd_u24(bytes, pos + 16)?,
        0x12 => 5,
        0x13 => 2 + 2 * (*bytes.get(pos + 1)? as usize),
        0x14 => 11 + rd_u24(bytes, pos + 8)?,
        0x15 => 9 + rd_u24(bytes, pos + 6)?,
        0x20 | 0x23 | 0x24 => 3,
        0x21 | 0x30 => 2 + (*bytes.get(pos + 1)? as usize),
        0x22 | 0x25 | 0x27 => 1,
        0x26 => 3 + 2 * (rd_u16(bytes, pos + 1)?),
        0x28 | 0x2B => 5 + rd_u16(bytes, pos + 1)?,
        0x2A => 5,
        0x31 => 3 + (*bytes.get(pos + 2)? as usize),
        0x33 => 2 + 3 * (*bytes.get(pos + 1)? as usize),
        0x5A => 10,
        _ => return None, // unknown: stop
    };
    Some(len)
}

/// Parse a `#32` archive-info body into `(field, value)` pairs.
fn parse_archive_info(body: &[u8]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let Some((&count, mut rest)) = body.split_first() else {
        return pairs;
    };
    for _ in 0..count {
        let (&text_id, after) = match rest.split_first() {
            Some(v) => v,
            None => break,
        };
        let (&text_len, after) = match after.split_first() {
            Some(v) => v,
            None => break,
        };
        let Some(text) = after.get(..text_len as usize) else {
            break;
        };
        rest = &after[text_len as usize..];
        pairs.push((archive_field(text_id).to_string(), ascii_trim(text)));
    }
    pairs
}

fn archive_field(id: u8) -> &'static str {
    match id {
        0x00 => "Title",
        0x01 => "Publisher",
        0x02 => "Author",
        0x03 => "Year",
        0x04 => "Language",
        0x05 => "Type",
        0x06 => "Price",
        0x07 => "Protection",
        0x08 => "Origin",
        0xFF => "Comment",
        _ => "Info",
    }
}

fn ascii_trim(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches(|c: char| c.is_control() || c == '\0' || c == ' ')
        .to_string()
}

fn rd_u16(bytes: &[u8], pos: usize) -> Option<usize> {
    Some(u16::from_le_bytes([*bytes.get(pos)?, *bytes.get(pos + 1)?]) as usize)
}

fn rd_u24(bytes: &[u8], pos: usize) -> Option<usize> {
    Some(
        *bytes.get(pos)? as usize
            | (*bytes.get(pos + 1)? as usize) << 8
            | (*bytes.get(pos + 2)? as usize) << 16,
    )
}

fn rd_u32(bytes: &[u8], pos: usize) -> Option<usize> {
    Some(u32::from_le_bytes([
        *bytes.get(pos)?,
        *bytes.get(pos + 1)?,
        *bytes.get(pos + 2)?,
        *bytes.get(pos + 3)?,
    ]) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::CasFileKind;

    /// Append a `#4B` block carrying `data` to `out`.
    fn push_4b(out: &mut Vec<u8>, data: &[u8]) {
        out.push(0x4B);
        let len = (data.len() + 12) as u32;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&[0u8; 12]); // pilot/pulses/bit0/bit1/cfg/pause
        out.extend_from_slice(data);
    }

    fn push_35(out: &mut Vec<u8>, id: &str, text: &str) {
        out.push(0x35);
        let mut id16 = [0x20u8; 16];
        id16[..id.len().min(16)].copy_from_slice(&id.as_bytes()[..id.len().min(16)]);
        out.extend_from_slice(&id16);
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
    }

    fn header_block(type_byte: u8, name: &[u8; 6]) -> Vec<u8> {
        let mut b = vec![type_byte; 10];
        b.extend_from_slice(name);
        b
    }

    fn tsx_header() -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&[0x01, 0x15]); // version 1.21
        v
    }

    #[test]
    fn empty_without_magic() {
        assert!(parse(b"not a tsx file").is_empty());
    }

    #[test]
    fn parses_custom_info_and_kcs_blocks() {
        let mut tsx = tsx_header();
        push_35(&mut tsx, "TSX.RIPPER", "makeTSX v0.8.5b");
        push_4b(&mut tsx, &header_block(0xD3, b"PROG  ")); // BASIC header
        push_4b(&mut tsx, b"basic-body");

        let blocks = parse(&tsx);
        assert_eq!(blocks.len(), 3);
        match &blocks[0] {
            TapeBlock::CustomInfo { id, text } => {
                assert_eq!(id, "TSX.RIPPER");
                assert_eq!(text, "makeTSX v0.8.5b");
            }
            other => panic!("expected custom info, got {other:?}"),
        }
        match &blocks[1] {
            TapeBlock::Msx {
                header: Some(h), ..
            } => {
                assert_eq!(h.kind, CasFileKind::Basic);
                assert_eq!(h.name, "PROG");
            }
            other => panic!("expected header block, got {other:?}"),
        }
    }

    #[test]
    fn parses_archive_info_fields() {
        let mut tsx = tsx_header();
        // #32: count=2, (Title="Game"), (Year="1987")
        let mut body = vec![2u8];
        body.extend_from_slice(&[0x00, 4]);
        body.extend_from_slice(b"Game");
        body.extend_from_slice(&[0x03, 4]);
        body.extend_from_slice(b"1987");
        tsx.push(0x32);
        tsx.extend_from_slice(&(body.len() as u16).to_le_bytes());
        tsx.extend_from_slice(&body);

        let blocks = parse(&tsx);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            TapeBlock::ArchiveInfo(pairs) => {
                assert_eq!(pairs[0], ("Title".to_string(), "Game".to_string()));
                assert_eq!(pairs[1], ("Year".to_string(), "1987".to_string()));
            }
            other => panic!("expected archive info, got {other:?}"),
        }
    }

    #[test]
    fn skips_unrecognized_via_known_length() {
        let mut tsx = tsx_header();
        // #20 pause (2-byte payload) then a #4B block: the pause must be skipped.
        tsx.push(0x20);
        tsx.extend_from_slice(&[0xE8, 0x03]);
        push_4b(&mut tsx, &header_block(0xD0, b"BIN   "));

        let blocks = parse(&tsx);
        // Pause recorded as Other, then the header block.
        assert!(matches!(blocks[0], TapeBlock::Other { id: 0x20, .. }));
        assert!(matches!(
            blocks[1],
            TapeBlock::Msx {
                header: Some(_),
                ..
            }
        ));
    }
}
