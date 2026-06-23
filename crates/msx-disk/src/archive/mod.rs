//! Reading LHA/LZH (`.lzh`/`.lha`), LArc (`.lzs`), and PMarc (`.pma`) archive
//! containers — the compressed file bundles that frequently live *inside* MSX
//! disk and tape images.
//!
//! Listing and all decompression are delegated to the [`delharc`] crate, which
//! covers every method these containers use except PMarc's `-pm1-`/`-pm2-`.
//! Those two — the methods most real `.pma` files use — are decoded by the
//! hand-ported [`pma`] module. The container header format is shared across all
//! three archive families, so `delharc` parses the directory for every one of
//! them; only the payload codec differs.
//!
//! The module is UI-free: [`list`] returns plain [`ArchiveEntry`] values and
//! [`extract`] returns decompressed bytes, so the logic is testable without a
//! GUI and reusable from a CLI.

use std::io::{Cursor, Read};

use delharc::decode::LhaDecodeReader;
use delharc::header::LhaHeader;

use crate::error::{Error, Result};
use crate::fs::Timestamp;

mod pma;

/// A compression-method tag as stored in the header, e.g. `*b"-lh5-"` or
/// `*b"-pm2-"`. Kept as the raw five bytes so it round-trips exactly with the
/// container and with `delharc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Method(pub [u8; 5]);

impl Method {
    /// The raw five-byte tag as text, e.g. `"-lh5-"`.
    pub fn tag(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("?????")
    }

    /// A short uppercase label for display, e.g. `"LH5"`, `"PM2"`, `"LZS"`.
    pub fn label(&self) -> String {
        self.0
            .iter()
            .skip(1)
            .take(3)
            .map(|&b| (b as char).to_ascii_uppercase())
            .collect()
    }

    fn is_pmarc_packed(&self) -> bool {
        matches!(&self.0, b"-pm1-" | b"-pm2-")
    }
}

/// One member of an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Full member path as stored, slash-separated (subdirectories preserved).
    pub path: String,
    /// Decompressed size in bytes, from the header.
    pub original_size: u64,
    /// Compressed size in bytes, from the header.
    pub compressed_size: u64,
    /// Compression method.
    pub method: Method,
    /// Last-modified timestamp, when present and parseable.
    pub modified: Option<Timestamp>,
    /// A directory or symlink meta-entry (`-lhd-`): has no extractable content.
    pub is_directory: bool,
    /// Whether this build can decompress this member. `false` rows are shown
    /// but cannot be extracted (unknown/unsupported method).
    pub decodable: bool,
}

/// Extensions recognized as archive containers we can open.
const ARCHIVE_EXTS: &[&str] = &["lzh", "lha", "lzs", "pma"];

/// The lowercased extension of the final path component, or `""` if none.
fn extension(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Whether `name`'s extension marks it as a supported archive container.
pub fn is_archive(name: &str) -> bool {
    ARCHIVE_EXTS.contains(&extension(name).as_str())
}

/// Methods `delharc` itself can decompress, given our enabled features
/// (`std`, `lh1`, `lz`). Directory (`-lhd-`) and `-lhx-` are excluded.
fn delharc_supports(method: &[u8; 5]) -> bool {
    matches!(
        method,
        b"-lh0-"
            | b"-lh1-"
            | b"-lh4-"
            | b"-lh5-"
            | b"-lh6-"
            | b"-lh7-"
            | b"-lzs-"
            | b"-lz4-"
            | b"-lz5-"
            | b"-pm0-"
    )
}

/// Parse the MS-DOS packed date/time stored in level 0/1 headers. Level 2/3
/// headers store a Unix timestamp instead, which we report as `None` (rare for
/// MSX archives and only cosmetic in the listing).
fn parse_timestamp(header: &LhaHeader) -> Option<Timestamp> {
    if header.level >= 2 {
        return None;
    }
    let ts = header.last_modified;
    let minute = ((ts >> 5) & 0x3f) as u8;
    let hour = ((ts >> 11) & 0x1f) as u8;
    let day = ((ts >> 16) & 0x1f) as u8;
    let month = ((ts >> 21) & 0x0f) as u8;
    let year = 1980 + ((ts >> 25) & 0x7f) as u16;
    if month == 0 || day == 0 {
        return None;
    }
    Some(Timestamp {
        year,
        month,
        day,
        hour,
        minute,
    })
}

fn entry_from_header(header: &LhaHeader) -> ArchiveEntry {
    let method = Method(header.compression);
    let decodable = !header.is_directory()
        && (delharc_supports(&method.0) || pma::supports(&method.0));
    ArchiveEntry {
        path: header.parse_pathname_to_str(),
        original_size: header.original_size,
        compressed_size: header.compressed_size,
        method,
        modified: parse_timestamp(header),
        is_directory: header.is_directory(),
        decodable,
    }
}

/// An archive that is empty has no member headers — represented on disk by a
/// lone `0x00` terminator (or nothing at all). `delharc` treats a missing first
/// header as an error, so we detect the empty case up front.
fn is_effectively_empty(bytes: &[u8]) -> bool {
    bytes.first().is_none_or(|&b| b == 0)
}

/// List every member of an archive.
///
/// Returns `Ok(vec![])` for an empty archive and `Err` only when the container
/// is structurally unreadable. Per-member limitations (an unsupported method)
/// are reflected in [`ArchiveEntry::decodable`], not as an error.
pub fn list(bytes: &[u8]) -> Result<Vec<ArchiveEntry>> {
    if is_effectively_empty(bytes) {
        return Ok(Vec::new());
    }
    let mut reader = LhaDecodeReader::new(Cursor::new(bytes))
        .map_err(|e| Error::Malformed(e.to_string()))?;
    let mut entries = Vec::new();
    loop {
        entries.push(entry_from_header(reader.header()));
        if !reader
            .next_file()
            .map_err(|e| Error::Malformed(e.to_string()))?
        {
            break;
        }
    }
    Ok(entries)
}

/// Decompress a single member by its index in [`list`]'s output.
pub fn extract(bytes: &[u8], index: usize) -> Result<Vec<u8>> {
    let mut reader = LhaDecodeReader::new(Cursor::new(bytes))
        .map_err(|e| Error::Malformed(e.to_string()))?;
    for _ in 0..index {
        if !reader
            .next_file()
            .map_err(|e| Error::Malformed(e.to_string()))?
        {
            return Err(Error::Malformed(format!("no archive member at index {index}")));
        }
    }

    let method = Method(reader.header().compression);
    let original_size = reader.header().original_size as usize;
    let compressed_size = reader.header().compressed_size as usize;

    if reader.header().is_directory() {
        return Err(Error::Unsupported(
            "directory entries have no contents".into(),
        ));
    }

    if method.is_pmarc_packed() {
        // delharc cannot decode pm1/pm2. After parsing a header it positions
        // the inner reader exactly at the compressed payload (it constructs the
        // decoder lazily over `rd.take(compressed_size)` without consuming any
        // payload bytes), so the cursor offset marks the payload start.
        let cursor = reader
            .take_inner()
            .ok_or_else(|| Error::Malformed("archive reader is empty".into()))?;
        let start = cursor.position() as usize;
        let end = start
            .checked_add(compressed_size)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| Error::Malformed("member payload exceeds archive".into()))?;
        return pma::decompress(&method.0, &bytes[start..end], original_size);
    }

    if !delharc_supports(&method.0) {
        return Err(Error::Unsupported(format!(
            "compression method {} is not supported",
            method.tag()
        )));
    }

    let mut out = Vec::with_capacity(original_size);
    reader.read_to_end(&mut out)?;
    Ok(out)
}

/// CRC-16/ARC — the checksum LHA/LZS/PMA stores for each member's contents.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

/// Whether a member's decompressed contents match the CRC-16 recorded in its
/// header. Returns `None` if the member is out of range or cannot be decoded.
pub fn crc_ok(bytes: &[u8], index: usize) -> Option<bool> {
    let expected = {
        let mut reader = LhaDecodeReader::new(Cursor::new(bytes)).ok()?;
        for _ in 0..index {
            if !reader.next_file().ok()? {
                return None;
            }
        }
        reader.header().file_crc
    };
    let data = extract(bytes, index).ok()?;
    Some(crc16(&data) == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A level-0, single-member `-lh0-` (stored) archive holding
    // "HELLO, MSX WORLD!\n" as HELLO.TXT, produced by LHarc.
    const STORED_LH0: &[u8] = &[
        0x2b, 0xcf, 0x2d, 0x6c, 0x68, 0x30, 0x2d, 0x12, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00,
        0x80, 0x18, 0x22, 0x50, 0x20, 0x00, 0x09, 0x48, 0x45, 0x4c, 0x4c, 0x4f, 0x2e, 0x54, 0x58,
        0x54, 0x37, 0x73, 0x55, 0x00, 0x90, 0x4f, 0x0d, 0x5e, 0xa4, 0x81, 0xf5, 0x01, 0x14, 0x00,
        0x48, 0x45, 0x4c, 0x4c, 0x4f, 0x2c, 0x20, 0x4d, 0x53, 0x58, 0x20, 0x57, 0x4f, 0x52, 0x4c,
        0x44, 0x21, 0x0a, 0x00,
    ];

    // A level-2, two-member archive: BIG.TXT compressed with `-lh5-` (5400
    // bytes of "The quick brown fox jumps over the lazy dog. " repeated 120
    // times) followed by HELLO.TXT stored with `-lh0-`.
    const PACKED: &[u8] = &[
        0x35, 0x00, 0x2d, 0x6c, 0x68, 0x35, 0x2d, 0x4c, 0x00, 0x00, 0x00, 0x18, 0x15, 0x00, 0x00,
        0x90, 0x4f, 0x0d, 0x5e, 0x20, 0x02, 0x95, 0xb1, 0x55, 0x05, 0x00, 0x00, 0xc2, 0x3a, 0x0a,
        0x00, 0x01, 0x42, 0x49, 0x47, 0x2e, 0x54, 0x58, 0x54, 0x05, 0x00, 0x50, 0xa4, 0x81, 0x07,
        0x00, 0x51, 0x14, 0x00, 0xf5, 0x01, 0x00, 0x00, 0x00, 0x40, 0x48, 0xad, 0xb6, 0xa3, 0xfe,
        0xc0, 0xcf, 0x72, 0x98, 0x22, 0xe4, 0x8a, 0x0a, 0x8f, 0x91, 0x03, 0x1c, 0x59, 0xa6, 0xc0,
        0x0e, 0xb8, 0x00, 0x02, 0x6b, 0xc5, 0xeb, 0x6a, 0x65, 0x74, 0x55, 0xb3, 0x6e, 0xb4, 0x57,
        0xb7, 0x6a, 0x14, 0xcf, 0x5d, 0xd3, 0x72, 0xf9, 0xae, 0x3e, 0x74, 0xac, 0xf7, 0xc4, 0xe6,
        0xc2, 0xa4, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x58,
        0x58, 0x58, 0x58, 0x58, 0x58, 0x58, 0x59, 0xfd, 0x80, 0x37, 0x00, 0x2d, 0x6c, 0x68, 0x30,
        0x2d, 0x12, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x90, 0x4f, 0x0d, 0x5e, 0x20, 0x02,
        0x37, 0x73, 0x55, 0x05, 0x00, 0x00, 0xb8, 0xdc, 0x0c, 0x00, 0x01, 0x48, 0x45, 0x4c, 0x4c,
        0x4f, 0x2e, 0x54, 0x58, 0x54, 0x05, 0x00, 0x50, 0xa4, 0x81, 0x07, 0x00, 0x51, 0x14, 0x00,
        0xf5, 0x01, 0x00, 0x00, 0x48, 0x45, 0x4c, 0x4c, 0x4f, 0x2c, 0x20, 0x4d, 0x53, 0x58, 0x20,
        0x57, 0x4f, 0x52, 0x4c, 0x44, 0x21, 0x0a, 0x00,
    ];

    #[test]
    fn is_archive_matches_known_extensions() {
        assert!(is_archive("GAME.LZH"));
        assert!(is_archive("disk/UTIL.lha"));
        assert!(is_archive("a.LZS"));
        assert!(is_archive("SNOOPY.pma"));
        assert!(!is_archive("README.TXT"));
        assert!(!is_archive("NOEXT"));
        assert!(!is_archive(".lzh")); // no stem
        assert!(!is_archive("ARCHIVE.ZIP"));
    }

    #[test]
    fn lists_stored_single_member() {
        let entries = list(STORED_LH0).expect("list");
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.path, "HELLO.TXT");
        assert_eq!(e.original_size, 18);
        assert_eq!(e.method.tag(), "-lh0-");
        assert_eq!(e.method.label(), "LH0");
        assert!(e.decodable);
        assert!(!e.is_directory);
    }

    #[test]
    fn extracts_stored_member() {
        let out = extract(STORED_LH0, 0).expect("extract");
        assert_eq!(out, b"HELLO, MSX WORLD!\n");
        assert_eq!(crc_ok(STORED_LH0, 0), Some(true));
    }

    #[test]
    fn lists_multiple_members_with_methods() {
        let entries = list(PACKED).expect("list");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "BIG.TXT");
        assert_eq!(entries[0].method.tag(), "-lh5-");
        assert_eq!(entries[0].original_size, 5400);
        assert!(entries[0].decodable);
        assert_eq!(entries[1].path, "HELLO.TXT");
        assert_eq!(entries[1].method.tag(), "-lh0-");
        assert_eq!(entries[1].original_size, 18);
    }

    #[test]
    fn extracts_lh5_compressed_member() {
        let out = extract(PACKED, 0).expect("extract lh5");
        let expected = "The quick brown fox jumps over the lazy dog. ".repeat(120);
        assert_eq!(out.len(), 5400);
        assert_eq!(out, expected.as_bytes());
        assert_eq!(crc_ok(PACKED, 0), Some(true));
    }

    #[test]
    fn crc16_matches_known_vector() {
        // CRC-16/ARC of "123456789" is 0xBB3D.
        assert_eq!(crc16(b"123456789"), 0xBB3D);
    }

    #[test]
    fn extracts_second_stored_member() {
        let out = extract(PACKED, 1).expect("extract stored");
        assert_eq!(out, b"HELLO, MSX WORLD!\n");
    }

    #[test]
    fn extract_out_of_range_errors() {
        assert!(extract(STORED_LH0, 5).is_err());
    }

    #[test]
    fn empty_archive_lists_nothing() {
        assert!(list(&[]).unwrap().is_empty());
        assert!(list(&[0x00]).unwrap().is_empty());
    }

    #[test]
    fn garbage_is_not_a_valid_archive() {
        assert!(list(b"this is definitely not an LHA archive").is_err());
    }
}
