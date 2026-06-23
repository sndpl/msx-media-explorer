//! Unified tape model for `.cas` and `.tsx` images.
//!
//! Both formats ultimately carry the same MSX data blocks; `.tsx` (TZX 1.21)
//! wraps them in `#4B` blocks and adds informational blocks. This module
//! presents an ordered list of [`TapeBlock`]s for the block-overview view and
//! derives logical files (header + data) for extraction and viewing.

use crate::cas::{self, CasFile, CasFileKind};

/// Which on-disk tape container a [`Tape`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapeFormat {
    Cas,
    Tsx,
}

impl TapeFormat {
    /// Map a file extension (any case) to a tape format.
    pub fn from_extension(ext: &str) -> Option<TapeFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "cas" => Some(TapeFormat::Cas),
            "tsx" => Some(TapeFormat::Tsx),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TapeFormat::Cas => "CAS",
            TapeFormat::Tsx => "TSX",
        }
    }
}

/// A file header detected inside an MSX data block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapeHeader {
    pub kind: CasFileKind,
    pub name: String,
}

/// One block as recorded on the tape, in order, for the block overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapeBlock {
    /// TSX `#35` custom-info block: an identifier and free text.
    CustomInfo { id: String, text: String },
    /// TSX `#32` archive-info block: `(field, value)` pairs.
    ArchiveInfo(Vec<(String, String)>),
    /// An MSX data block (a CAS block or a TSX `#4B` block). `header` is set
    /// when the block is a file header rather than payload.
    Msx {
        header: Option<TapeHeader>,
        data: Vec<u8>,
    },
    /// A recognized TZX block we list but do not decode.
    Other { id: u8, len: usize },
}

/// A parsed tape: its format and ordered blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tape {
    pub format: TapeFormat,
    pub blocks: Vec<TapeBlock>,
}

impl Tape {
    /// Derive logical files (a header plus the data blocks following it) for
    /// extraction and viewing, reusing the CAS pairing rules.
    pub fn files(&self) -> Vec<CasFile> {
        let msx: Vec<&[u8]> = self
            .blocks
            .iter()
            .filter_map(|b| match b {
                TapeBlock::Msx { data, .. } => Some(data.as_slice()),
                _ => None,
            })
            .collect();
        cas::files_from_blocks(&msx)
    }

    /// Total payload bytes across all derived files.
    pub fn total_file_bytes(&self) -> usize {
        self.files().iter().map(|f| f.data.len()).sum()
    }
}

/// Parse `bytes` as a tape of the given `format`.
pub fn open(bytes: &[u8], format: TapeFormat) -> Tape {
    let blocks = match format {
        TapeFormat::Cas => cas_blocks(bytes),
        TapeFormat::Tsx => crate::tsx::parse(bytes),
    };
    Tape { format, blocks }
}

/// Every CAS block becomes an MSX tape block, flagged as a header when it is
/// one.
fn cas_blocks(bytes: &[u8]) -> Vec<TapeBlock> {
    cas::blocks(bytes)
        .into_iter()
        .map(|b| TapeBlock::Msx {
            header: cas::header_of(b).map(|(kind, name)| TapeHeader { kind, name }),
            data: b.to_vec(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNC: [u8; 8] = [0x1F, 0xA6, 0xDE, 0xBA, 0xCC, 0x13, 0x7D, 0x74];

    fn cas_with_ascii_split() -> Vec<u8> {
        let mut cas = Vec::new();
        // ASCII header "DOC".
        cas.extend_from_slice(&SYNC);
        cas.extend_from_slice(&[0xEA; 10]);
        cas.extend_from_slice(b"DOC   ");
        // Two 256-byte ASCII data blocks (split, as real tapes store them).
        cas.extend_from_slice(&SYNC);
        cas.extend_from_slice(&[b'A'; 256]);
        cas.extend_from_slice(&SYNC);
        cas.extend_from_slice(&[b'B'; 100]);
        cas
    }

    #[test]
    fn cas_concatenates_split_ascii_data() {
        let tape = open(&cas_with_ascii_split(), TapeFormat::Cas);
        let files = tape.files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "DOC");
        assert_eq!(files[0].kind, CasFileKind::Ascii);
        // Both data blocks are concatenated (256 + 100).
        assert_eq!(files[0].data.len(), 356);
    }

    #[test]
    fn cas_blocks_flag_headers() {
        let tape = open(&cas_with_ascii_split(), TapeFormat::Cas);
        let headers = tape
            .blocks
            .iter()
            .filter(|b| {
                matches!(
                    b,
                    TapeBlock::Msx {
                        header: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(headers, 1, "one header block");
        assert_eq!(tape.blocks.len(), 3, "header + two data blocks");
    }
}
