//! `.cas` — MSX cassette tape images.
//!
//! A `.cas` file is a sequence of blocks, each preceded by the 8-byte sync
//! header `1F A6 DE BA CC 13 7D 74` (aligned to 8-byte boundaries). A file is
//! stored as a *header* block — ten copies of a file-type byte followed by a
//! 6-character name — immediately followed by a *data* block holding the
//! contents. We treat sync headers as block delimiters: a block runs from just
//! after its sync header to the start of the next one (or end of file).

/// The 8-byte block sync header.
const SYNC: [u8; 8] = [0x1F, 0xA6, 0xDE, 0xBA, 0xCC, 0x13, 0x7D, 0x74];

/// The kind of a tape file, from its header type byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasFileKind {
    /// Binary (BLOAD) — data begins with `0xFE` + begin/end/exec addresses.
    Binary,
    /// Tokenized BASIC.
    Basic,
    /// Plain ASCII.
    Ascii,
}

impl CasFileKind {
    fn from_byte(b: u8) -> Option<CasFileKind> {
        match b {
            0xD0 => Some(CasFileKind::Binary),
            0xD3 => Some(CasFileKind::Basic),
            0xEA => Some(CasFileKind::Ascii),
            _ => None,
        }
    }

    /// Short human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            CasFileKind::Binary => "Binary",
            CasFileKind::Basic => "BASIC",
            CasFileKind::Ascii => "ASCII",
        }
    }
}

/// One file extracted from a tape image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasFile {
    pub name: String,
    pub kind: CasFileKind,
    /// The raw data block contents (for Binary this includes the `0xFE` header).
    pub data: Vec<u8>,
}

/// Offsets where a sync header begins.
fn block_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + SYNC.len() <= bytes.len() {
        if bytes[i..i + SYNC.len()] == SYNC {
            starts.push(i);
            i += SYNC.len();
        } else {
            i += 1;
        }
    }
    starts
}

/// The content of each tape block: the bytes between one sync header and the
/// next (or end of file). These are the same logical blocks a TSX `#4B` block
/// carries, so the tape layer can treat CAS and TSX uniformly.
pub fn blocks(bytes: &[u8]) -> Vec<&[u8]> {
    let starts = block_starts(bytes);
    starts
        .iter()
        .enumerate()
        .map(|(idx, &start)| {
            let content_start = start + SYNC.len();
            let content_end = starts.get(idx + 1).copied().unwrap_or(bytes.len());
            &bytes[content_start..content_end]
        })
        .collect()
}

/// If `block` is a file header (ten identical known type bytes + a 6-char name),
/// return its kind and trimmed name.
pub fn header_of(block: &[u8]) -> Option<(CasFileKind, String)> {
    if block.len() < 16 {
        return None;
    }
    let type_byte = block[0];
    if !block[..10].iter().all(|&b| b == type_byte) {
        return None;
    }
    let kind = CasFileKind::from_byte(type_byte)?;
    let name = String::from_utf8_lossy(&block[10..16])
        .trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .to_string();
    Some((kind, name))
}

/// Pair header blocks with the data block(s) that follow them, concatenating
/// consecutive data blocks (ASCII files are split into 256-byte chunks).
pub fn files_from_blocks(blocks: &[&[u8]]) -> Vec<CasFile> {
    let mut files = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        if let Some((kind, name)) = header_of(blocks[i]) {
            let mut data = Vec::new();
            let mut j = i + 1;
            while j < blocks.len() && header_of(blocks[j]).is_none() {
                data.extend_from_slice(blocks[j]);
                j += 1;
            }
            files.push(CasFile { name, kind, data });
            i = j;
        } else {
            i += 1;
        }
    }
    files
}

/// List the files contained in a `.cas` tape image.
pub fn list(bytes: &[u8]) -> Vec<CasFile> {
    files_from_blocks(&blocks(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_block(type_byte: u8, name: &[u8; 6]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&SYNC);
        b.extend_from_slice(&[type_byte; 10]);
        b.extend_from_slice(name);
        b
    }

    #[test]
    fn parses_a_binary_file() {
        let mut cas = header_block(0xD0, b"HELLO ");
        cas.extend_from_slice(&SYNC);
        cas.extend_from_slice(b"\xFE\x00\x80\x05\x80\x00\x00payload");

        let files = list(&cas);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "HELLO");
        assert_eq!(files[0].kind, CasFileKind::Binary);
        assert!(files[0].data.starts_with(&[0xFE]));
        assert!(files[0].data.ends_with(b"payload"));
    }

    #[test]
    fn parses_multiple_files_of_different_kinds() {
        let mut cas = header_block(0xD3, b"PROG  ");
        cas.extend_from_slice(&SYNC);
        cas.extend_from_slice(b"basicdata");
        cas.extend_from_slice(&header_block(0xEA, b"READ  "));
        cas.extend_from_slice(&SYNC);
        cas.extend_from_slice(b"text\x1a");

        let files = list(&cas);
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].name.as_str(), files[0].kind),
            ("PROG", CasFileKind::Basic)
        );
        assert_eq!(
            (files[1].name.as_str(), files[1].kind),
            ("READ", CasFileKind::Ascii)
        );
    }

    #[test]
    fn empty_or_garbage_yields_no_files() {
        assert!(list(b"").is_empty());
        assert!(list(b"not a tape image at all").is_empty());
    }
}
