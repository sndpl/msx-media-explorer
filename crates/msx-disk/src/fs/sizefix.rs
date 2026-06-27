//! Detect and repair a mismatch between a disk's physical FAT layout and its
//! declared size / image file size.
//!
//! MSX 360KB and 720KB disks have different *physical* layouts: a 2-sector FAT
//! for 360KB versus a 3-sector FAT for 720KB, which place the root/data regions
//! at different offsets. So `sectors_per_fat` is the ground truth for a disk's
//! real size — a 2-sector-FAT disk can never address more than 360KB, so it can
//! never "really" be double-sided. We therefore reconcile the *image file size*
//! and the boot sector's *size-declaration fields* (`total_sectors`, media
//! descriptor, heads) to the layout, and never touch the layout fields
//! themselves (which would relocate data and corrupt the disk).

use crate::image::geometry::{SECTOR_SIZE, SIZE_360K, SIZE_720K};

/// Canonical size-declaration fields for a standard MSX disk. The layout fields
/// (reserved/FATs/`sectors_per_fat`/root) are deliberately absent: this fixes
/// only the size declaration, never the physical layout.
struct Canon {
    total_sectors: u16,
    media: u8,
    heads: u16,
    sectors_per_track: u16,
}

const CANON_360: Canon = Canon {
    total_sectors: 720,
    media: 0xF8,
    heads: 1,
    sectors_per_track: 9,
};

const CANON_720: Canon = Canon {
    total_sectors: 1440,
    media: 0xF9,
    heads: 2,
    sectors_per_track: 9,
};

/// The safe, recommended repair for a [`SizeMismatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeFix {
    /// Drop the unused (all-zero) trailing sectors so the image matches the
    /// filesystem's real size, then correct the boot size-declaration fields.
    TruncateImage,
    /// Zero-pad a too-short image up to the filesystem's real size, then correct
    /// the boot size-declaration fields.
    PadImage,
    /// Leave the image size unchanged; only correct the boot sector's
    /// size-declaration fields (total sectors / media / heads) to the real size.
    FixBootSector,
}

/// A detected disagreement between a disk's real (layout) size and its image
/// file size and/or declared size.
#[derive(Debug, Clone)]
pub struct SizeMismatch {
    /// The image file's current size in bytes.
    pub image_bytes: usize,
    /// The size the boot sector currently declares (`total_sectors * 512`).
    pub declared_bytes: usize,
    /// The disk's real size from its physical layout (`sectors_per_fat`).
    pub real_bytes: usize,
    /// The safe, recommended repair.
    pub fix: SizeFix,
}

fn rd_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

/// Detect a standard-size mismatch on a normalized image, or `None` when the
/// disk is already consistent, has a garbage BPB (synthesized from file size at
/// mount, so nothing to reconcile), is not a standard 360/720 image, or has no
/// safe repair.
pub fn detect(data: &[u8]) -> Option<SizeMismatch> {
    if data.len() < SECTOR_SIZE {
        return None;
    }
    // Only a self-consistent BPB is authoritative; a garbage one is synthesized
    // from the file size at mount, so there is nothing to reconcile.
    if !super::boot::existing_bpb_is_valid(data) {
        return None;
    }
    let image_bytes = data.len();
    // Only the two standard MSX image sizes are in scope.
    if image_bytes != SIZE_360K && image_bytes != SIZE_720K {
        return None;
    }
    // `sectors_per_fat` fixes the physical layout, hence the real size. Only the
    // two standard MSX layouts are handled.
    let (real_bytes, canon) = match rd_u16(data, 22) {
        2 => (SIZE_360K, &CANON_360),
        3 => (SIZE_720K, &CANON_720),
        _ => return None,
    };

    let declared_total = rd_u16(data, 19);
    let declared_bytes = declared_total as usize * SECTOR_SIZE;
    // The 5.25" double-sided 360KB format (media FD, 2 heads) shares the 360KB
    // 2-sector-FAT layout but is a valid format in its own right, so its media
    // and head count are not a mismatch.
    let alt_360_format = real_bytes == SIZE_360K && data[21] == 0xFD && rd_u16(data, 26) == 2;
    let fields_wrong = !alt_360_format
        && (declared_total != canon.total_sectors
            || data[21] != canon.media
            || rd_u16(data, 26) != canon.heads);
    let size_wrong = image_bytes != real_bytes;
    if !fields_wrong && !size_wrong {
        return None;
    }

    let fix = if !size_wrong {
        SizeFix::FixBootSector
    } else if image_bytes < real_bytes {
        SizeFix::PadImage
    } else if data[real_bytes..].iter().all(|&b| b == 0) {
        // The surplus tail is unused; safe to drop.
        SizeFix::TruncateImage
    } else if fields_wrong {
        // Tail has data (can't shrink), but the declaration is still wrong.
        SizeFix::FixBootSector
    } else {
        // Oversized container with data in the tail and a correct boot sector:
        // nothing safe to do automatically.
        return None;
    };

    Some(SizeMismatch {
        image_bytes,
        declared_bytes,
        real_bytes,
        fix,
    })
}

/// Produce the corrected image bytes for `mismatch`: resize per its [`SizeFix`]
/// and rewrite the boot sector's size-declaration fields to the real size.
pub fn repair(data: &[u8], mismatch: &SizeMismatch) -> Vec<u8> {
    let mut out = data.to_vec();
    match mismatch.fix {
        SizeFix::TruncateImage => out.truncate(mismatch.real_bytes),
        SizeFix::PadImage => out.resize(mismatch.real_bytes, 0),
        SizeFix::FixBootSector => {}
    }
    let canon = if mismatch.real_bytes == SIZE_720K {
        &CANON_720
    } else {
        &CANON_360
    };
    // Size-declaration fields only; the layout fields are never touched.
    out[19..21].copy_from_slice(&canon.total_sectors.to_le_bytes());
    out[21] = canon.media;
    out[24..26].copy_from_slice(&canon.sectors_per_track.to_le_bytes());
    out[26..28].copy_from_slice(&canon.heads.to_le_bytes());
    // Keep the media descriptor stored at the head of each FAT copy in sync.
    let reserved = rd_u16(&out, 14) as usize;
    let sectors_per_fat = rd_u16(&out, 22) as usize;
    let num_fats = out[16] as usize;
    for i in 0..num_fats {
        let off = (reserved + i * sectors_per_fat) * SECTOR_SIZE;
        if off < out.len() {
            out[off] = canon.media;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::write::create_blank;
    use crate::image::geometry::DiskFormat;

    fn set_u16(data: &mut [u8], off: usize, v: u16) {
        data[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn consistent_disks_report_no_mismatch() {
        // Every freshly-created format (including the 5.25" FD 360KB, which
        // shares the 360KB layout) must look consistent.
        for format in DiskFormat::ALL {
            assert!(
                detect(&create_blank(format).unwrap()).is_none(),
                "{format:?} should report no mismatch"
            );
        }
    }

    #[test]
    fn garbage_bpb_reports_no_mismatch() {
        // A zeroed sector 0 is not a valid BPB; it would be synthesized at mount.
        let mut data = create_blank(DiskFormat::Ds720).unwrap();
        data[..64].fill(0);
        assert!(detect(&data).is_none());
    }

    #[test]
    fn single_sided_in_720_container_truncates() {
        // A 360KB filesystem (2-sector FAT) padded into a 720KB container with a
        // zero tail: the real size is 360KB, the surplus is unused → truncate.
        let mut data = create_blank(DiskFormat::Ss360).unwrap();
        data.resize(SIZE_720K, 0);
        let m = detect(&data).expect("mismatch");
        assert_eq!(m.fix, SizeFix::TruncateImage);
        assert_eq!(m.real_bytes, SIZE_360K);
        assert_eq!(m.image_bytes, SIZE_720K);

        let fixed = repair(&data, &m);
        assert_eq!(fixed.len(), SIZE_360K);
        assert!(detect(&fixed).is_none(), "repaired disk is consistent");
    }

    #[test]
    fn non_empty_tail_is_not_truncated() {
        // Same container, but the surplus holds data: not safe to drop, and the
        // boot fields are already correct, so there is no safe repair.
        let mut data = create_blank(DiskFormat::Ss360).unwrap();
        data.resize(SIZE_720K, 0);
        data[SIZE_360K + 100] = 0xAB;
        assert!(detect(&data).is_none());
    }

    #[test]
    fn truncated_720_image_pads() {
        // A 720KB filesystem (3-sector FAT) whose image was truncated to 360KB:
        // the real size is 720KB → pad with zeros.
        let mut data = create_blank(DiskFormat::Ds720).unwrap();
        data.truncate(SIZE_360K);
        let m = detect(&data).expect("mismatch");
        assert_eq!(m.fix, SizeFix::PadImage);
        assert_eq!(m.real_bytes, SIZE_720K);

        let fixed = repair(&data, &m);
        assert_eq!(fixed.len(), SIZE_720K);
        assert!(detect(&fixed).is_none());
    }

    #[test]
    fn wrong_size_fields_on_720_layout_fix_boot_only() {
        // A genuine 720KB disk (3-sector FAT, 720KB image) whose boot sector
        // mislabels it single-sided (total=720, media=F8, heads=1). The image is
        // the right size; only the declaration is wrong → fix the boot sector.
        let mut data = create_blank(DiskFormat::Ds720).unwrap();
        set_u16(&mut data, 19, 720); // total_sectors says 360KB
        data[21] = 0xF8; // media descriptor: single-sided
        set_u16(&mut data, 26, 1); // heads
        let m = detect(&data).expect("mismatch");
        assert_eq!(m.fix, SizeFix::FixBootSector);
        assert_eq!(m.real_bytes, SIZE_720K);
        assert_eq!(m.declared_bytes, 720 * SECTOR_SIZE);

        let fixed = repair(&data, &m);
        assert_eq!(fixed.len(), SIZE_720K, "image size unchanged");
        assert_eq!(rd_u16(&fixed, 19), 1440);
        assert_eq!(fixed[21], 0xF9);
        assert_eq!(rd_u16(&fixed, 26), 2);
        assert!(detect(&fixed).is_none());
    }
}
