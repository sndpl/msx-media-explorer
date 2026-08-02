//! ZIP archives holding disk images.
//!
//! MSX disk images are very often distributed zipped, sometimes several disks
//! of one release per archive. This reads the container, inflates every member
//! whose extension is a disk-image format, normalizes each through
//! [`DiskImage::open_bytes`] (so a zipped `.xsa` or `.dmk` is handled exactly as
//! it would be on its own), and lays the results end to end. Members that are
//! not disk images — readmes, scans — are ignored; real archives are full of
//! them.
//!
//! Only the parts of the format an MSX archive uses are supported: stored and
//! deflated entries, no encryption, no zip64. Anything else is reported rather
//! than guessed at. The `zip` crate is deliberately not used; that surface
//! (encryption, zip64, bzip2/zstd) would never be exercised here, and the
//! project already hand-rolls its other container formats.

use super::{DiskImage, ImageFormat};
use crate::error::{Error, Result};
use crate::image::geometry::SECTOR_SIZE;

/// End of central directory record.
const EOCD_SIGNATURE: [u8; 4] = *b"PK\x05\x06";
/// Central directory file header.
const CENTRAL_SIGNATURE: [u8; 4] = *b"PK\x01\x02";
/// Local file header. Also the first bytes of any non-empty archive.
pub(super) const LOCAL_SIGNATURE: [u8; 4] = *b"PK\x03\x04";

const EOCD_LEN: usize = 22;
const CENTRAL_LEN: usize = 46;
const LOCAL_LEN: usize = 30;

/// The archive comment that may follow the EOCD is a 16-bit length, so the
/// record starts at most this far from the end.
const MAX_COMMENT: usize = u16::MAX as usize;

/// Compression methods we can read.
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

/// General-purpose bit 0: the entry is encrypted.
const FLAG_ENCRYPTED: u16 = 1 << 0;

/// A size field of all-ones means the real value lives in a zip64 extra field.
const ZIP64_SENTINEL: u32 = 0xFFFF_FFFF;

/// Where one disk image extracted from an archive lives in the normalized
/// buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The member's name inside the archive, without any directory part.
    pub name: String,
    /// First sector (LBA) of this image within [`Archive::data`].
    pub lba_start: usize,
    /// Length of this image in sectors.
    pub sector_count: usize,
}

impl Member {
    /// Byte length of this image.
    pub fn byte_len(&self) -> usize {
        self.sector_count * SECTOR_SIZE
    }
}

/// Every disk image in an archive, normalized and laid end to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub data: Vec<u8>,
    pub members: Vec<Member>,
}

/// Whether `bytes` begins like a ZIP archive.
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(&LOCAL_SIGNATURE)
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    let raw = bytes
        .get(at..at + 2)
        .ok_or_else(|| Error::Malformed("zip: truncated header".into()))?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    let raw = bytes
        .get(at..at + 4)
        .ok_or_else(|| Error::Malformed("zip: truncated header".into()))?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

/// One entry as described by the central directory.
struct Entry {
    name: String,
    method: u16,
    compressed_size: usize,
    uncompressed_size: usize,
    local_header: usize,
}

/// Offset of the central directory, from the end-of-central-directory record.
fn central_directory_start(bytes: &[u8]) -> Result<(usize, usize)> {
    let window = bytes.len().min(EOCD_LEN + MAX_COMMENT);
    let search_from = bytes.len() - window;
    // Scan back: the last match wins, so a stored file that happens to contain
    // the signature cannot shadow the real record.
    let eocd = (search_from..=bytes.len().saturating_sub(EOCD_LEN))
        .rev()
        .find(|&at| bytes[at..at + 4] == EOCD_SIGNATURE)
        .ok_or_else(|| Error::Malformed("zip: no end-of-central-directory record".into()))?;
    let count = u16_at(bytes, eocd + 10)? as usize;
    let offset = u32_at(bytes, eocd + 16)? as usize;
    if offset > bytes.len() {
        return Err(Error::Malformed("zip: central directory past end".into()));
    }
    Ok((offset, count))
}

/// Read the central directory into one [`Entry`] per member.
fn entries(bytes: &[u8]) -> Result<Vec<Entry>> {
    if bytes.len() < EOCD_LEN {
        return Err(Error::Malformed("zip: file too short".into()));
    }
    let (mut at, count) = central_directory_start(bytes)?;
    let mut out = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        if bytes.get(at..at + CENTRAL_LEN).is_none() {
            return Err(Error::Malformed("zip: truncated central directory".into()));
        }
        if bytes[at..at + 4] != CENTRAL_SIGNATURE {
            return Err(Error::Malformed("zip: bad central directory entry".into()));
        }
        let flags = u16_at(bytes, at + 8)?;
        if flags & FLAG_ENCRYPTED != 0 {
            return Err(Error::Unsupported("zip: encrypted archives".into()));
        }
        let method = u16_at(bytes, at + 10)?;
        let compressed = u32_at(bytes, at + 20)?;
        let uncompressed = u32_at(bytes, at + 24)?;
        if compressed == ZIP64_SENTINEL || uncompressed == ZIP64_SENTINEL {
            return Err(Error::Unsupported("zip: zip64 archives".into()));
        }
        let name_len = u16_at(bytes, at + 28)? as usize;
        let extra_len = u16_at(bytes, at + 30)? as usize;
        let comment_len = u16_at(bytes, at + 32)? as usize;
        let local_header = u32_at(bytes, at + 42)? as usize;
        let name_raw = bytes
            .get(at + CENTRAL_LEN..at + CENTRAL_LEN + name_len)
            .ok_or_else(|| Error::Malformed("zip: truncated entry name".into()))?;
        out.push(Entry {
            // Names are stored as UTF-8 or CP437; either way the lossy read is
            // only used for display and extension matching.
            name: String::from_utf8_lossy(name_raw).into_owned(),
            method,
            compressed_size: compressed as usize,
            uncompressed_size: uncompressed as usize,
            local_header,
        });
        at += CENTRAL_LEN + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// Decompress one entry's bytes.
///
/// The data offset comes from the *local* header, whose extra field routinely
/// differs in length from the central one.
fn read_entry(bytes: &[u8], entry: &Entry) -> Result<Vec<u8>> {
    let at = entry.local_header;
    if bytes.get(at..at + LOCAL_LEN).is_none() || bytes[at..at + 4] != LOCAL_SIGNATURE {
        return Err(Error::Malformed(format!(
            "zip: bad local header for '{}'",
            entry.name
        )));
    }
    let name_len = u16_at(bytes, at + 26)? as usize;
    let extra_len = u16_at(bytes, at + 28)? as usize;
    let start = at + LOCAL_LEN + name_len + extra_len;
    let data = bytes
        .get(start..start + entry.compressed_size)
        .ok_or_else(|| Error::Malformed(format!("zip: truncated data for '{}'", entry.name)))?;

    match entry.method {
        METHOD_STORED => Ok(data.to_vec()),
        METHOD_DEFLATE => {
            // Capped at the declared size, so a hostile archive cannot inflate
            // without bound.
            miniz_oxide::inflate::decompress_to_vec_with_limit(data, entry.uncompressed_size)
                .map_err(|e| {
                    Error::Malformed(format!("zip: cannot inflate '{}': {e:?}", entry.name))
                })
        }
        other => Err(Error::Unsupported(format!(
            "zip: compression method {other} for '{}'",
            entry.name
        ))),
    }
}

/// The member's file name without any directory part.
fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Whether this entry is a file (not a directory) whose extension names a disk
/// image format.
fn image_format_of(entry: &Entry) -> Option<ImageFormat> {
    let name = base_name(&entry.name);
    if name.is_empty() || entry.name.ends_with('/') {
        return None;
    }
    ImageFormat::from_extension(name.rsplit_once('.')?.1)
}

/// Read every disk image in `bytes` and normalize them into one buffer.
///
/// Fails when the archive holds no disk image at all, naming what it did hold
/// so the user knows why (a tape-only or scans-only archive is a real case).
pub fn open(bytes: &[u8]) -> Result<Archive> {
    let entries = entries(bytes)?;
    let mut data = Vec::new();
    let mut members = Vec::new();
    for entry in &entries {
        let Some(format) = image_format_of(entry) else {
            continue;
        };
        let raw = read_entry(bytes, entry)?;
        let image = DiskImage::open_bytes(format, raw)?;
        let lba_start = data.len() / SECTOR_SIZE;
        data.extend_from_slice(image.data());
        members.push(Member {
            name: base_name(&entry.name).to_string(),
            lba_start,
            sector_count: image.sector_count(),
        });
    }
    if members.is_empty() {
        let held: Vec<&str> = entries
            .iter()
            .map(|e| base_name(&e.name))
            .filter(|n| !n.is_empty())
            .collect();
        return Err(Error::Unsupported(format!(
            "zip: no disk image in the archive (it holds: {})",
            if held.is_empty() {
                "nothing".to_string()
            } else {
                held.join(", ")
            }
        )));
    }
    Ok(Archive { data, members })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::geometry::SIZE_720K;

    /// Build a one-entry-per-file archive with everything stored (method 0), so
    /// the tests need no compressor.
    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body) in files {
            let local_at = out.len() as u32;
            let crc = 0u32; // unchecked by the reader
            out.extend_from_slice(&LOCAL_SIGNATURE);
            out.extend_from_slice(&[0x0A, 0x00]); // version needed
            out.extend_from_slice(&[0x00, 0x00]); // flags
            out.extend_from_slice(&METHOD_STORED.to_le_bytes());
            out.extend_from_slice(&[0u8; 4]); // time, date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0x00, 0x00]); // extra len
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(body);

            central.extend_from_slice(&CENTRAL_SIGNATURE);
            central.extend_from_slice(&[0x14, 0x00, 0x0A, 0x00]); // versions
            central.extend_from_slice(&[0x00, 0x00]); // flags
            central.extend_from_slice(&METHOD_STORED.to_le_bytes());
            central.extend_from_slice(&[0u8; 4]); // time, date
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            // extra(2) + comment(2) + disk(2) + internal(2) + external(4): the
            // fixed part of a central header is 46 bytes, name included at 46.
            central.extend_from_slice(&[0u8; 12]);
            central.extend_from_slice(&local_at.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_at = out.len() as u32;
        let central_len = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIGNATURE);
        out.extend_from_slice(&[0u8; 4]); // disk numbers
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&central_len.to_le_bytes());
        out.extend_from_slice(&central_at.to_le_bytes());
        out.extend_from_slice(&[0x00, 0x00]); // comment len
        out
    }

    /// A 720 kB disk whose every byte encodes its sector index, so a misplaced
    /// member is detectable.
    fn disk(tag: u8) -> Vec<u8> {
        (0..SIZE_720K)
            .map(|i| tag.wrapping_add((i / SECTOR_SIZE) as u8))
            .collect()
    }

    #[test]
    fn a_single_stored_disk_comes_out_byte_for_byte() {
        let source = disk(0);
        let archive = open(&zip_of(&[("GAME.dsk", &source)])).expect("open");
        assert_eq!(archive.data, source);
        assert_eq!(
            archive.members,
            vec![Member {
                name: "GAME.dsk".into(),
                lba_start: 0,
                sector_count: 1440,
            }]
        );
    }

    #[test]
    fn two_disks_are_laid_end_to_end() {
        let (a, b) = (disk(0), disk(9));
        let archive = open(&zip_of(&[("A.dsk", &a), ("B.DSK", &b)])).expect("open");
        assert_eq!(archive.members.len(), 2);
        assert_eq!(archive.members[0].lba_start, 0);
        assert_eq!(archive.members[1].lba_start, 1440);
        assert_eq!(&archive.data[..SIZE_720K], &a[..]);
        assert_eq!(&archive.data[SIZE_720K..], &b[..]);
    }

    /// Real archives carry readmes and scans; they must be skipped without
    /// shifting the images that follow them.
    #[test]
    fn non_image_members_are_skipped_without_shifting_the_rest() {
        let (a, b) = (disk(0), disk(9));
        let archive = open(&zip_of(&[
            ("readme.txt", b"hello"),
            ("A.dsk", &a),
            ("scan.png", b"\x89PNG"),
            ("B.dsk", &b),
        ]))
        .expect("open");
        assert_eq!(archive.members.len(), 2);
        assert_eq!(archive.members[0].name, "A.dsk");
        assert_eq!(archive.members[1].name, "B.dsk");
        assert_eq!(archive.members[1].lba_start, 1440);
        assert_eq!(&archive.data[SIZE_720K..], &b[..]);
    }

    #[test]
    fn a_member_in_a_subdirectory_keeps_only_its_file_name() {
        let archive = open(&zip_of(&[("disks/GAME.dsk", &disk(0))])).expect("open");
        assert_eq!(archive.members[0].name, "GAME.dsk");
    }

    /// A tape-only or scans-only archive is a real case; the error has to say
    /// what was actually in there.
    #[test]
    fn an_archive_without_a_disk_image_says_what_it_holds() {
        let err = open(&zip_of(&[("song.cas", b"x"), ("readme.txt", b"y")])).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no disk image"), "{msg}");
        assert!(msg.contains("song.cas"), "{msg}");
    }

    #[test]
    fn an_empty_archive_is_an_error() {
        assert!(open(&zip_of(&[])).is_err());
    }

    #[test]
    fn encrypted_and_zip64_entries_are_refused_by_name() {
        let mut data = zip_of(&[("GAME.dsk", &disk(0))]);
        let central = find_central(&data);
        // General-purpose bit 0 in the central header marks encryption.
        data[central + 8] = 0x01;
        let msg = open(&data).unwrap_err().to_string();
        assert!(msg.contains("encrypted"), "{msg}");

        let mut data = zip_of(&[("GAME.dsk", &disk(0))]);
        let central = find_central(&data);
        data[central + 24..central + 28].copy_from_slice(&ZIP64_SENTINEL.to_le_bytes());
        let msg = open(&data).unwrap_err().to_string();
        assert!(msg.contains("zip64"), "{msg}");
    }

    #[test]
    fn an_unsupported_compression_method_is_named() {
        let mut data = zip_of(&[("GAME.dsk", &disk(0))]);
        let central = find_central(&data);
        data[central + 10..central + 12].copy_from_slice(&12u16.to_le_bytes()); // bzip2
        let msg = open(&data).unwrap_err().to_string();
        assert!(msg.contains("method 12"), "{msg}");
    }

    /// Offset of the first central-directory header.
    fn find_central(data: &[u8]) -> usize {
        data.windows(4)
            .position(|w| w == CENTRAL_SIGNATURE)
            .expect("central directory")
    }

    #[test]
    fn malformed_archives_error_rather_than_panic() {
        assert!(open(&[]).is_err());
        assert!(open(b"not a zip at all").is_err());
        // Valid archive with its central directory lopped off.
        let data = zip_of(&[("GAME.dsk", &disk(0))]);
        assert!(open(&data[..data.len() / 2]).is_err());
        // Every truncation of a real archive must fail cleanly.
        for cut in [1, 8, 64, 512, data.len() - 1] {
            let _ = open(&data[..cut.min(data.len())]);
        }
    }

    #[test]
    fn is_zip_recognises_the_local_header_magic() {
        assert!(is_zip(&zip_of(&[("GAME.dsk", &disk(0))])));
        assert!(!is_zip(b"MV - CPCEMU Disk-File"));
        assert!(!is_zip(&[]));
    }

    /// A member whose length is not a whole number of sectors is not a disk
    /// image, and the normalizer must say so rather than produce a short image.
    #[test]
    fn a_member_that_is_not_sector_aligned_is_rejected() {
        assert!(open(&zip_of(&[("GAME.dsk", b"three")])).is_err());
    }
}
