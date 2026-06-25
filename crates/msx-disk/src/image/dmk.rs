//! `.dmk` — David Keil raw-track disk images (as used by openMSX).
//!
//! Unlike a flat `.dsk`, DMK preserves the full FDC track layout — sync bytes,
//! ID/data address marks, CRCs and gaps — so it can hold copy-protected or
//! otherwise non-standard tracks. We parse each track's IDAM pointer table,
//! read every sector by its recorded C/H/R, and lay the 512-byte sectors into a
//! normalized `.dsk` buffer. [`analyze`] exposes the raw per-track detail for
//! the GUI's DMK analysis view.
//!
//! Reference: openMSX `Contrib/dmk` and David Keil's original format notes.

use crate::error::{Error, Result};
use crate::image::geometry::{SECTOR_SIZE, SIZE_360K, SIZE_720K};

const HEADER_LEN: usize = 16;
/// Per-track IDAM pointer table: 64 little-endian 2-byte entries.
const IDAM_TABLE_LEN: usize = 0x80;
const IDAM_COUNT: usize = 64;
const ID_ADDRESS_MARK: u8 = 0xFE;
/// Bit 15 of an IDAM pointer marks a double-density (MFM) sector.
const DD_FLAG: u16 = 0x8000;
/// Low 14 bits of an IDAM pointer are the offset from the track start.
const OFFSET_MASK: u16 = 0x3FFF;
/// How far past an ID field to look for the data address mark.
const DAM_SEARCH_WINDOW: usize = 64;

/// Recording density of a sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    /// MFM / double density (every MSX disk).
    Mfm,
    /// FM / single density.
    Fm,
}

/// One sector as recorded on a DMK track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmkSectorInfo {
    pub cyl: u8,
    pub head: u8,
    pub sector: u8,
    /// Sector payload size in bytes (`128 << N`).
    pub size: usize,
    pub density: Density,
    pub id_crc_ok: bool,
    pub data_crc_ok: bool,
    /// Whether a data address mark + payload was found for this sector.
    pub has_data: bool,
}

/// Every sector found on one physical track (one side of one cylinder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmkTrackInfo {
    pub track: u8,
    pub side: u8,
    pub sectors: Vec<DmkSectorInfo>,
}

impl DmkTrackInfo {
    /// A standard MSX track has nine 512-byte MFM sectors, all CRC-clean.
    pub fn is_standard(&self) -> bool {
        self.sectors.len() == 9
            && self.sectors.iter().all(|s| {
                s.size == SECTOR_SIZE
                    && s.density == Density::Mfm
                    && s.id_crc_ok
                    && s.data_crc_ok
                    && s.has_data
            })
    }
}

/// Raw per-track analysis of a DMK image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmkAnalysis {
    pub tracks: u8,
    pub sides: u8,
    pub write_protected: bool,
    pub track_len: usize,
    pub track_infos: Vec<DmkTrackInfo>,
}

impl DmkAnalysis {
    /// The number of sides that actually carry recorded sectors. This is what
    /// [`normalize`] keys the LBA layout off, and can be smaller than [`sides`]
    /// (the header's declared side count) when a side is unformatted.
    ///
    /// [`sides`]: Self::sides
    pub fn effective_sides(&self) -> u8 {
        let any_side1 = self
            .track_infos
            .iter()
            .any(|t| t.sectors.iter().any(|s| s.head == 1));
        if any_side1 {
            2
        } else {
            1
        }
    }

    /// Whether the header declares two sides but only side 0 is formatted — a
    /// single-sided disk stored in a two-sided container (common for some
    /// copy-protected games). [`normalize`] collapses these to a 360KB image.
    pub fn single_sided_in_two_sided_container(&self) -> bool {
        self.sides == 2 && self.effective_sides() == 1
    }
}

struct Header {
    write_protected: bool,
    tracks: u8,
    sides: u8,
    track_len: usize,
}

fn parse_header(bytes: &[u8]) -> Result<Header> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::Malformed("DMK image smaller than its header".into()));
    }
    let track_len = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
    if track_len <= IDAM_TABLE_LEN {
        return Err(Error::Malformed("DMK track length too small".into()));
    }
    let flags = bytes[4];
    Ok(Header {
        write_protected: bytes[0] == 0xFF,
        tracks: bytes[1],
        // Bit 4 of the flags byte marks a single-sided disk.
        sides: if flags & 0x10 != 0 { 1 } else { 2 },
        track_len,
    })
}

/// The byte range of physical track `(track, side)` within the file.
fn track_slice<'a>(bytes: &'a [u8], h: &Header, track: u8, side: u8) -> Option<&'a [u8]> {
    let index = track as usize * h.sides as usize + side as usize;
    let start = HEADER_LEN + index * h.track_len;
    bytes.get(start..start + h.track_len)
}

/// WD-style CRC-16-CCITT (poly 0x1021, preset 0xFFFF, no reflection).
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in bytes {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// A sector parsed from a track, with its payload location.
struct RawSector {
    info: DmkSectorInfo,
    data: Vec<u8>,
}

/// Parse all sectors recorded in one track's data.
fn track_sectors(track: &[u8]) -> Vec<RawSector> {
    let mut sectors = Vec::new();
    for i in 0..IDAM_COUNT {
        let ptr = u16::from_le_bytes([track[i * 2], track[i * 2 + 1]]);
        if ptr == 0 {
            break; // table is terminated by a zero entry
        }
        let density = if ptr & DD_FLAG != 0 {
            Density::Mfm
        } else {
            Density::Fm
        };
        let off = (ptr & OFFSET_MASK) as usize;
        // Need the mark byte plus C/H/R/N and the 2-byte ID CRC.
        if off + 7 > track.len() || track[off] != ID_ADDRESS_MARK {
            continue;
        }
        let cyl = track[off + 1];
        let head = track[off + 2];
        let sector = track[off + 3];
        let n = track[off + 4] & 0x03;
        let size = 128usize << n;
        let id_crc = u16::from_be_bytes([track[off + 5], track[off + 6]]);
        let id_crc_ok = crc16(&id_field(track, off)) == id_crc;

        // The data address mark follows after the ID gap; scan for it.
        let (data, data_crc_ok, has_data) = match find_dam(track, off + 7) {
            Some(dam) if dam + 1 + size + 2 <= track.len() => {
                let data = track[dam + 1..dam + 1 + size].to_vec();
                let stored = u16::from_be_bytes([track[dam + 1 + size], track[dam + 2 + size]]);
                let ok = crc16(&data_field(track, dam, size)) == stored;
                (data, ok, true)
            }
            _ => (Vec::new(), false, false),
        };

        sectors.push(RawSector {
            info: DmkSectorInfo {
                cyl,
                head,
                sector,
                size,
                density,
                id_crc_ok,
                data_crc_ok,
                has_data,
            },
            data,
        });
    }
    sectors
}

/// Bytes the ID CRC is computed over: the 3 MFM sync `0xA1`s (when present)
/// plus the address mark and the C/H/R/N field.
fn id_field(track: &[u8], off: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    if off >= 3 && track[off - 3..off] == [0xA1, 0xA1, 0xA1] {
        buf.extend_from_slice(&[0xA1, 0xA1, 0xA1]);
    }
    buf.extend_from_slice(&track[off..off + 5]); // FE C H R N
    buf
}

/// Bytes the data CRC is computed over: the 3 sync `0xA1`s (when present), the
/// data address mark, and the payload.
fn data_field(track: &[u8], dam: usize, size: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(size + 4);
    if dam >= 3 && track[dam - 3..dam] == [0xA1, 0xA1, 0xA1] {
        buf.extend_from_slice(&[0xA1, 0xA1, 0xA1]);
    }
    buf.push(track[dam]); // FB / F8 / ...
    buf.extend_from_slice(&track[dam + 1..dam + 1 + size]);
    buf
}

/// Find the data address mark (`0xFB`/`0xF8`/`0xF9`/`0xFA`) following an ID.
fn find_dam(track: &[u8], from: usize) -> Option<usize> {
    let end = (from + DAM_SEARCH_WINDOW).min(track.len());
    (from..end).find(|&i| matches!(track[i], 0xF8..=0xFB))
}

/// Decode a DMK image into a normalized linear sector buffer (a `.dsk`).
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>> {
    let header = parse_header(bytes)?;
    let mut sectors = Vec::new();
    for track in 0..header.tracks {
        for side in 0..header.sides {
            if let Some(slice) = track_slice(bytes, &header, track, side) {
                sectors.extend(track_sectors(slice));
            }
        }
    }
    let spt = sectors
        .iter()
        .map(|s| s.info.sector as usize)
        .max()
        .ok_or_else(|| Error::Malformed("DMK image contains no readable sectors".into()))?;
    if spt == 0 {
        return Err(Error::Malformed(
            "DMK sectors are numbered from zero".into(),
        ));
    }

    // The DMK header records the *physical* side count, but a single-sided disk
    // is often stored in a two-sided container: every sector is recorded on
    // head 0 and side 1 is left unformatted. Trusting the header would lay the
    // side-0 data into a two-sided LBA map, interleaving empty tracks between
    // the real ones and scrambling the filesystem. Derive the side count from
    // the sectors actually recorded instead.
    let sides = if sectors.iter().any(|s| s.info.head == 1) {
        2
    } else {
        1
    };

    let out_len = header.tracks as usize * sides * spt * SECTOR_SIZE;
    let mut out = vec![0u8; out_len];
    for s in &sectors {
        let (cyl, head, sector) = (s.info.cyl as usize, s.info.head as usize, s.info.sector);
        if s.info.size != SECTOR_SIZE
            || s.data.len() != SECTOR_SIZE
            || !(1..=spt).contains(&(sector as usize))
            || cyl >= header.tracks as usize
            || head >= sides
        {
            continue;
        }
        let lba = (cyl * sides + head) * spt + (sector as usize - 1);
        let start = lba * SECTOR_SIZE;
        if start + SECTOR_SIZE <= out.len() {
            out[start..start + SECTOR_SIZE].copy_from_slice(&s.data);
        }
    }

    // DMK images often carry a couple of extra (or short) tracks beyond the
    // 360KB/720KB the MSX filesystem actually uses. Snap to the standard size
    // so the boot-sector repair and FAT layer see a normal disk.
    let target = if out.len() >= SIZE_720K {
        SIZE_720K
    } else {
        SIZE_360K
    };
    out.resize(target, 0);
    Ok(out)
}

/// Produce the per-track analysis used by the DMK analysis view.
pub fn analyze(bytes: &[u8]) -> Result<DmkAnalysis> {
    let header = parse_header(bytes)?;
    let mut track_infos = Vec::new();
    for track in 0..header.tracks {
        for side in 0..header.sides {
            let sectors = track_slice(bytes, &header, track, side)
                .map(|slice| track_sectors(slice).into_iter().map(|s| s.info).collect())
                .unwrap_or_default();
            track_infos.push(DmkTrackInfo {
                track,
                side,
                sectors,
            });
        }
    }
    Ok(DmkAnalysis {
        tracks: header.tracks,
        sides: header.sides,
        write_protected: header.write_protected,
        track_len: header.track_len,
        track_infos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one MFM track containing `count` standard 512-byte sectors whose
    /// payload byte encodes the LBA, mirroring a real formatted MSX track.
    fn build_track(track_len: usize, cyl: u8, head: u8, sides: u8, count: u8) -> Vec<u8> {
        let mut data = vec![0x4Eu8; track_len];
        // Reserve the IDAM table; fill it as we place sectors.
        for b in data.iter_mut().take(IDAM_TABLE_LEN) {
            *b = 0;
        }
        let mut pos = IDAM_TABLE_LEN + 32; // leading gap
        for s in 0..count {
            let sector = s + 1;
            // ID field: A1 A1 A1 FE C H R N CRC CRC
            data[pos..pos + 3].copy_from_slice(&[0xA1, 0xA1, 0xA1]);
            let idam = pos + 3;
            data[idam] = ID_ADDRESS_MARK;
            data[idam + 1] = cyl;
            data[idam + 2] = head;
            data[idam + 3] = sector;
            data[idam + 4] = 2; // N=2 -> 512 bytes
            let id_crc = crc16(&[0xA1, 0xA1, 0xA1, 0xFE, cyl, head, sector, 2]);
            data[idam + 5..idam + 7].copy_from_slice(&id_crc.to_be_bytes());

            // Record the IDAM pointer (offset to the FE, DD flag set).
            let ptr = (idam as u16) | DD_FLAG;
            data[s as usize * 2..s as usize * 2 + 2].copy_from_slice(&ptr.to_le_bytes());

            // Gap + data field: A1 A1 A1 FB <512 bytes> CRC CRC
            let dam = idam + 7 + 30;
            data[dam - 3..dam].copy_from_slice(&[0xA1, 0xA1, 0xA1]);
            data[dam] = 0xFB;
            let lba = (cyl as usize * sides as usize + head as usize) * count as usize
                + (sector as usize - 1);
            let payload = vec![(lba & 0xFF) as u8; SECTOR_SIZE];
            data[dam + 1..dam + 1 + SECTOR_SIZE].copy_from_slice(&payload);
            let mut crc_in = vec![0xA1, 0xA1, 0xA1, 0xFB];
            crc_in.extend_from_slice(&payload);
            let data_crc = crc16(&crc_in);
            data[dam + 1 + SECTOR_SIZE..dam + 3 + SECTOR_SIZE]
                .copy_from_slice(&data_crc.to_be_bytes());

            pos = dam + 3 + SECTOR_SIZE + 40; // trailing gap before next sector
        }
        data
    }

    fn build_dmk(tracks: u8, sides: u8, spt: u8) -> Vec<u8> {
        let track_len = 0x1900; // openMSX default
        let mut file = vec![0u8; HEADER_LEN];
        file[1] = tracks;
        file[2..4].copy_from_slice(&(track_len as u16).to_le_bytes());
        file[4] = if sides == 1 { 0x10 } else { 0x00 };
        for track in 0..tracks {
            for head in 0..sides {
                file.extend_from_slice(&build_track(track_len, track, head, sides, spt));
            }
        }
        file
    }

    /// A single-sided disk stored in a two-sided container: the header declares
    /// two sides, every sector is on head 0, and side 1 is unformatted. This is
    /// how some copy-protected games (and openMSX dumps of them) are stored.
    fn build_dmk_single_side_in_two_sided_container(tracks: u8) -> Vec<u8> {
        let track_len = 0x1900;
        let mut file = vec![0u8; HEADER_LEN];
        file[1] = tracks;
        file[2..4].copy_from_slice(&(track_len as u16).to_le_bytes());
        file[4] = 0x00; // header claims two sides
        for track in 0..tracks {
            // Side 0 holds standard nine-sector data; its payloads encode the
            // single-sided LBA (sides = 1).
            file.extend_from_slice(&build_track(track_len, track, 0, 1, 9));
            // Side 1 is unformatted: an all-zero track parses to no sectors.
            file.extend_from_slice(&vec![0u8; track_len]);
        }
        file
    }

    #[test]
    fn crc16_matches_known_vector() {
        // CRC-16/CCITT-FALSE of "123456789" is 0x29B1.
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn normalizes_to_standard_720k_image() {
        let dmk = build_dmk(80, 2, 9);
        let dsk = normalize(&dmk).expect("normalize");
        assert_eq!(dsk.len(), 737_280);
        // Each sector's payload byte equals its LBA & 0xFF.
        for lba in 0..1440usize {
            assert_eq!(dsk[lba * SECTOR_SIZE], (lba & 0xFF) as u8, "lba {lba}");
        }
    }

    #[test]
    fn normalizes_single_sided_360k() {
        let dmk = build_dmk(80, 1, 9);
        let dsk = normalize(&dmk).expect("normalize");
        assert_eq!(dsk.len(), 368_640);
    }

    #[test]
    fn analyze_reports_clean_standard_tracks() {
        let dmk = build_dmk(80, 2, 9);
        let analysis = analyze(&dmk).expect("analyze");
        assert_eq!(analysis.tracks, 80);
        assert_eq!(analysis.sides, 2);
        assert_eq!(analysis.track_infos.len(), 160);
        let t0 = &analysis.track_infos[0];
        assert_eq!(t0.sectors.len(), 9);
        assert!(t0.is_standard());
        assert!(t0.sectors.iter().all(|s| s.id_crc_ok && s.data_crc_ok));
        assert!(t0.sectors.iter().all(|s| s.density == Density::Mfm));
    }

    #[test]
    fn detects_bad_data_crc() {
        let mut dmk = build_dmk(2, 1, 9);
        // Corrupt the first sector's payload without fixing its CRC.
        let first_track = HEADER_LEN;
        let idam_ptr = u16::from_le_bytes([dmk[first_track], dmk[first_track + 1]]);
        let off = (idam_ptr & OFFSET_MASK) as usize;
        let dam = first_track + find_dam(&dmk[first_track..], off + 7).unwrap();
        dmk[dam + 1] ^= 0xFF;
        let analysis = analyze(&dmk).expect("analyze");
        let s0 = &analysis.track_infos[0].sectors[0];
        assert!(s0.id_crc_ok, "id untouched");
        assert!(!s0.data_crc_ok, "payload corrupted");
        assert!(!analysis.track_infos[0].is_standard());
    }

    #[test]
    fn single_sided_data_in_two_sided_container_normalizes_to_360k() {
        // The header claims two sides but only head 0 is formatted, so the disk
        // must collapse to a contiguous single-sided 360KB image rather than a
        // 720KB layout with empty side-1 tracks interleaved between the data.
        let dmk = build_dmk_single_side_in_two_sided_container(80);
        let dsk = normalize(&dmk).expect("normalize");
        assert_eq!(dsk.len(), SIZE_360K);
        // Sectors stay contiguous: each payload byte equals its single-sided LBA.
        for lba in 0..720usize {
            assert_eq!(dsk[lba * SECTOR_SIZE], (lba & 0xFF) as u8, "lba {lba}");
        }
    }

    #[test]
    fn analysis_flags_single_sided_data_in_two_sided_container() {
        let dmk = build_dmk_single_side_in_two_sided_container(80);
        let analysis = analyze(&dmk).expect("analyze");
        assert_eq!(analysis.sides, 2, "header declares two sides");
        assert_eq!(analysis.effective_sides(), 1, "only side 0 is formatted");
        assert!(analysis.single_sided_in_two_sided_container());
    }

    #[test]
    fn analysis_does_not_flag_genuine_two_sided_disk() {
        let analysis = analyze(&build_dmk(80, 2, 9)).expect("analyze");
        assert_eq!(analysis.effective_sides(), 2);
        assert!(!analysis.single_sided_in_two_sided_container());
    }

    #[test]
    fn analysis_does_not_flag_honest_single_sided_disk() {
        // Header honestly declares one side, so there is nothing to flag.
        let analysis = analyze(&build_dmk(80, 1, 9)).expect("analyze");
        assert_eq!(analysis.sides, 1);
        assert_eq!(analysis.effective_sides(), 1);
        assert!(!analysis.single_sided_in_two_sided_container());
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(normalize(&[0u8; 8]).is_err());
    }
}
