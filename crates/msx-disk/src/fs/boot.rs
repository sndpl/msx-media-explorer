//! MSX boot-sector compatibility shims.
//!
//! Real MSX disks routinely fail a strict PC FAT parser for two reasons:
//!
//! 1. They lack the `0x55 0xAA` signature at offset 510 (MSX-DOS never required
//!    it).
//! 2. MSX-DOS 1 disks ignore the BPB entirely and derive geometry from the
//!    media-descriptor byte, so the BPB area often holds garbage (e.g. an OEM
//!    string that overran its field).
//!
//! [`repair_boot_sector`] makes an in-memory copy acceptable to `fatfs`: it
//! keeps a genuinely valid BPB untouched (only adding the signature) and
//! otherwise synthesizes the canonical BPB for the disk's size. The synthesized
//! values match the fixed layout MSX-DOS itself uses, so they line up with the
//! real FAT/root/data regions on the disk.

use crate::error::{Error, Result};
use crate::image::geometry::{SECTOR_SIZE, SIZE_360K, SIZE_720K};

/// Canonical BIOS Parameter Block field values for a standard MSX disk.
struct Bpb {
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entries: u16,
    total_sectors: u16,
    media: u8,
    sectors_per_fat: u16,
    sectors_per_track: u16,
    heads: u16,
}

/// Standard double-sided 720KB MSX layout (media descriptor 0xF9).
const BPB_720K: Bpb = Bpb {
    sectors_per_cluster: 2,
    reserved_sectors: 1,
    num_fats: 2,
    root_entries: 112,
    total_sectors: 1440,
    media: 0xF9,
    sectors_per_fat: 3,
    sectors_per_track: 9,
    heads: 2,
};

/// Standard single-sided 360KB MSX layout (media descriptor 0xF8).
const BPB_360K: Bpb = Bpb {
    sectors_per_cluster: 2,
    reserved_sectors: 1,
    num_fats: 2,
    root_entries: 112,
    total_sectors: 720,
    media: 0xF8,
    sectors_per_fat: 2,
    sectors_per_track: 9,
    heads: 1,
};

/// Make `data`'s boot sector acceptable to a strict FAT parser, in place.
pub(crate) fn repair_boot_sector(data: &mut [u8]) -> Result<()> {
    if data.len() < SECTOR_SIZE {
        return Err(Error::Malformed("image smaller than one sector".into()));
    }
    if !existing_bpb_is_valid(data) {
        let canonical = match data.len() {
            SIZE_720K => &BPB_720K,
            SIZE_360K => &BPB_360K,
            _ => {
                return Err(Error::Unsupported(
                    "disk has no valid BPB and is not a standard 360KB/720KB size".into(),
                ))
            }
        };
        write_bpb(data, canonical);
    }

    // MSX disks use the short DOS-1 BPB: the Z80 boot program starts around
    // offset 30, so the bytes a PC parser reads as hidden_sectors,
    // total_sectors_32, and the extended-signature/volume-label fields are
    // actually boot code. Zero them so they do not look like a (bogus) 32-bit
    // sector count or a garbage volume label. total_sectors_16 carries the real
    // count for every MSX floppy.
    data[28..36].fill(0); // hidden_sectors + total_sectors_32
    data[36] = 0; // drive number
    data[37] = 0; // reserved
    data[38] = 0; // extended boot signature (!= 0x29 -> label fields ignored)

    // The PC boot signature is mandatory for fatfs; MSX disks frequently omit it.
    data[510] = 0x55;
    data[511] = 0xAA;
    Ok(())
}

fn rd_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

/// Whether the on-disk BPB is already self-consistent enough for fatfs.
fn existing_bpb_is_valid(data: &[u8]) -> bool {
    let bytes_per_sector = rd_u16(data, 11);
    let sectors_per_cluster = data[13];
    let reserved = rd_u16(data, 14);
    let num_fats = data[16];
    let root_entries = rd_u16(data, 17);
    let sectors_per_fat = rd_u16(data, 22);

    let total_sectors_16 = rd_u16(data, 19);

    bytes_per_sector == SECTOR_SIZE as u16
        && sectors_per_cluster.is_power_of_two()
        && (1..=2).contains(&num_fats)
        && reserved >= 1
        && root_entries > 0
        && (root_entries as usize * 32).is_multiple_of(bytes_per_sector as usize)
        && (1..=15).contains(&sectors_per_fat)
        && total_sectors_16 != 0
}

fn write_bpb(data: &mut [u8], b: &Bpb) {
    // Provide a sane jump opcode if the existing one is bogus.
    if data[0] != 0xEB && data[0] != 0xE9 {
        data[0] = 0xEB;
        data[1] = 0x3C;
        data[2] = 0x90;
    }
    data[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
    data[13] = b.sectors_per_cluster;
    data[14..16].copy_from_slice(&b.reserved_sectors.to_le_bytes());
    data[16] = b.num_fats;
    data[17..19].copy_from_slice(&b.root_entries.to_le_bytes());
    data[19..21].copy_from_slice(&b.total_sectors.to_le_bytes());
    data[21] = b.media;
    data[22..24].copy_from_slice(&b.sectors_per_fat.to_le_bytes());
    data[24..26].copy_from_slice(&b.sectors_per_track.to_le_bytes());
    data[26..28].copy_from_slice(&b.heads.to_le_bytes());
    data[28..32].copy_from_slice(&0u32.to_le_bytes()); // hidden sectors
    data[32..36].copy_from_slice(&0u32.to_le_bytes()); // total sectors (32-bit)
    data[36] = 0x00; // drive number
    data[37] = 0x00; // reserved
    data[38] = 0x00; // extended boot signature (!= 0x29 -> label fields ignored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::geometry::SIZE_720K;

    #[test]
    fn synthesizes_bpb_for_garbage_msxdos1_boot() {
        let mut data = vec![0u8; SIZE_720K];
        // Simulate an OEM string that overran the BPB area + missing signature.
        data[..16].copy_from_slice(b"\xEB\xFE\x90GAOS-MSX.1989");
        repair_boot_sector(&mut data).unwrap();

        assert_eq!(rd_u16(&data, 11), 512); // bytes per sector
        assert_eq!(data[13], 2); // sectors per cluster
        assert_eq!(rd_u16(&data, 17), 112); // root entries
        assert_eq!(rd_u16(&data, 19), 1440); // total sectors
        assert_eq!(data[21], 0xF9); // media descriptor
        assert_eq!(&data[510..512], &[0x55, 0xAA]);
    }

    #[test]
    fn preserves_valid_bpb_and_only_adds_signature() {
        let mut data = vec![0u8; SIZE_720K];
        write_bpb(&mut data, &BPB_720K);
        // A nonstandard-but-valid sectors_per_cluster should survive untouched.
        data[13] = 4;
        data[510] = 0;
        data[511] = 0;

        repair_boot_sector(&mut data).unwrap();
        assert_eq!(data[13], 4, "valid BPB must not be rewritten");
        assert_eq!(&data[510..512], &[0x55, 0xAA]);
    }

    #[test]
    fn rejects_nonstandard_size_without_bpb() {
        let mut data = vec![0u8; 100 * SECTOR_SIZE];
        assert!(repair_boot_sector(&mut data).is_err());
    }
}
