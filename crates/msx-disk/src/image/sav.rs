//! `.sav` — MSXPLAYer virtual floppy disk.
//!
//! MSXPLAYer (the official emulator shipped with ASCII's *MSX MAGAZINE
//! 永久保存版*, 2003) stores each virtual floppy as a diff journal against a
//! base image: a sequence of `[u32 little-endian sector number][512-byte
//! sector data]` records, applied in file order. For the emulator's writable
//! drives the base is an empty 720KB 2DD disk, so replaying the records onto
//! a zeroed buffer reconstructs the full disk.
//!
//! Sector 0 (BPB + boot code) is normally absent from the journal — the
//! emulator's base image provides it — so [`normalize`] synthesizes the
//! canonical 720KB BPB when no record supplies one, and [`encode`] omits
//! exactly that synthesized sector so the fake BPB is never written back into
//! a journal the emulator will overlay onto its own base.
//!
//! Format per Meister's CC0-licensed analysis and reference converters:
//! <https://github.com/meister68k/MSXPLAYer_sav2dsk>.

use crate::error::{Error, Result};
use crate::image::geometry::{SECTOR_SIZE, SIZE_720K};

/// Bytes per journal record: a 4-byte sector number plus one sector.
const RECORD_SIZE: usize = 4 + SECTOR_SIZE;
/// Sectors on the fixed 720KB 2DD geometry `.sav` journals describe.
const NUM_SECTORS: usize = SIZE_720K / SECTOR_SIZE;

/// The sector 0 synthesized when the journal carries none: the canonical
/// 720KB MSX BPB behind a jump stub, matching the reference `sav2dsk` (no
/// boot code — the emulator's base image owns the real IPL).
fn synthetic_sector0() -> [u8; SECTOR_SIZE] {
    #[rustfmt::skip]
    const BPB: [u8; 30] = [
        0xEB, 0xFE, 0x90,                               // jump stub
        b'U', b'N', b'K', b'N', b'O', b'W', b'N', b' ', // OEM name
        0x00, 0x02, 0x02, 0x01, 0x00,                   // 512 B/sec, 2 sec/cluster, 1 reserved
        0x02, 0x70, 0x00, 0xA0, 0x05,                   // 2 FATs, 112 root entries, 1440 sectors
        0xF9, 0x03, 0x00, 0x09, 0x00,                   // media F9, 3 sec/FAT, 9 sec/track
        0x02, 0x00, 0x00, 0x00,                         // 2 heads, 0 hidden
    ];
    let mut sector = [0u8; SECTOR_SIZE];
    sector[..BPB.len()].copy_from_slice(&BPB);
    sector
}

/// Replay a `.sav` journal onto an empty 720KB disk, yielding normalized
/// sector data. Later records for the same sector win (journal order).
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(RECORD_SIZE) {
        return Err(Error::Malformed(format!(
            "sav journal length {} is not a multiple of {RECORD_SIZE}-byte records",
            bytes.len()
        )));
    }

    let mut data = vec![0u8; SIZE_720K];
    let mut has_sector0 = false;
    for record in bytes.chunks_exact(RECORD_SIZE) {
        let sector_no = u32::from_le_bytes(record[..4].try_into().unwrap()) as usize;
        if sector_no >= NUM_SECTORS {
            return Err(Error::Malformed(format!(
                "sav record for sector {sector_no} is outside the 720KB disk ({NUM_SECTORS} sectors)"
            )));
        }
        has_sector0 |= sector_no == 0;
        let start = sector_no * SECTOR_SIZE;
        data[start..start + SECTOR_SIZE].copy_from_slice(&record[4..]);
    }

    if !has_sector0 {
        data[..SECTOR_SIZE].copy_from_slice(&synthetic_sector0());
    }
    Ok(data)
}

/// Encode normalized sector data as a `.sav` journal.
///
/// The journal is written sparsely: all-zero sectors are omitted (against the
/// empty base they are a no-op), and sector 0 is omitted when it still equals
/// the [`normalize`]-synthesized BPB stub, so the placeholder never overwrites
/// the base image's real boot sector. A sector 0 the user actually modified
/// is kept.
pub fn encode(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() != SIZE_720K {
        return Err(Error::Unsupported(format!(
            ".sav journals describe a 720KB 2DD disk; got {} bytes (convert to 720KB first)",
            data.len()
        )));
    }

    let synthetic = synthetic_sector0();
    let mut out = Vec::new();
    for (sector_no, sector) in data.chunks_exact(SECTOR_SIZE).enumerate() {
        if sector.iter().all(|&b| b == 0) {
            continue;
        }
        if sector_no == 0 && sector == synthetic {
            continue;
        }
        out.extend_from_slice(&(sector_no as u32).to_le_bytes());
        out.extend_from_slice(sector);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A journal from `(sector, fill_byte)` pairs, each sector filled with one byte.
    fn journal(records: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(sector, fill) in records {
            out.extend_from_slice(&sector.to_le_bytes());
            out.extend_from_slice(&[fill; SECTOR_SIZE]);
        }
        out
    }

    #[test]
    fn replays_records_onto_an_empty_disk() {
        let data = normalize(&journal(&[(5, 0xAA), (1439, 0xBB)])).unwrap();
        assert_eq!(data.len(), SIZE_720K);
        assert_eq!(data[5 * SECTOR_SIZE], 0xAA);
        assert_eq!(data[1439 * SECTOR_SIZE], 0xBB);
        assert_eq!(data[6 * SECTOR_SIZE], 0x00, "untouched sectors stay empty");
    }

    #[test]
    fn later_records_for_the_same_sector_win() {
        let data = normalize(&journal(&[(7, 0x11), (7, 0x22)])).unwrap();
        assert_eq!(data[7 * SECTOR_SIZE], 0x22);
    }

    #[test]
    fn synthesizes_a_bpb_only_when_sector0_is_absent() {
        let without = normalize(&journal(&[(5, 0xAA)])).unwrap();
        assert_eq!(&without[..3], &[0xEB, 0xFE, 0x90]);
        assert_eq!(without[0x15], 0xF9, "720KB media descriptor");

        let with = normalize(&journal(&[(0, 0xCC), (5, 0xAA)])).unwrap();
        assert_eq!(with[0], 0xCC, "a real sector 0 record is kept verbatim");
    }

    #[test]
    fn rejects_malformed_journals() {
        assert!(normalize(&[]).is_err());
        assert!(normalize(&[0u8; RECORD_SIZE - 1]).is_err());
        assert!(
            normalize(&journal(&[(1440, 0xAA)])).is_err(),
            "sector out of range"
        );
    }

    #[test]
    fn encode_is_sparse_and_round_trips() {
        let original = journal(&[(5, 0xAA), (9, 0xBB)]);
        let data = normalize(&original).unwrap();
        let encoded = encode(&data).unwrap();
        // Only the two written sectors are journaled; the synthetic sector 0
        // and all empty sectors are omitted.
        assert_eq!(encoded.len(), 2 * RECORD_SIZE);
        assert_eq!(normalize(&encoded).unwrap(), data, "lossless round trip");
    }

    #[test]
    fn encode_keeps_a_modified_sector0() {
        let mut data = normalize(&journal(&[(5, 0xAA)])).unwrap();
        data[0x40] = 0xFE; // simulate a boot-sector edit past the BPB stub
        let encoded = encode(&data).unwrap();
        assert_eq!(encoded.len(), 2 * RECORD_SIZE);
        assert_eq!(normalize(&encoded).unwrap(), data);
    }

    #[test]
    fn encode_rejects_non_720k_data() {
        assert!(encode(&[0u8; 512]).is_err());
    }
}
