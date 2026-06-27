//! Physical geometry of an MSX floppy disk.
//!
//! The "normalized" representation used throughout the crate is a flat byte
//! buffer holding every sector in logical (LBA) order — exactly what a plain
//! `.dsk` dump contains. Geometry is the metadata needed to render sector maps
//! and to reason about tracks/sides; the byte layout itself is always linear.

/// Bytes per sector on every supported MSX disk.
pub const SECTOR_SIZE: usize = 512;

/// Raw size of a 5.25" single-sided 180KB disk (40 tracks).
pub const SIZE_180K: usize = 184_320;
/// Raw size of a single-sided 360KB disk.
pub const SIZE_360K: usize = 368_640;
/// Raw size of a double-sided 720KB disk.
pub const SIZE_720K: usize = 737_280;

/// The physical shape of a disk image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// Number of recording sides (heads): 1 or 2.
    pub sides: u8,
    /// Number of tracks (cylinders) per side.
    pub tracks: u16,
    /// Sectors per track.
    pub sectors_per_track: u8,
}

impl Geometry {
    /// Standard single-sided 360KB MSX disk (1 side, 80 tracks, 9 sectors).
    pub const SS_360K: Geometry = Geometry {
        sides: 1,
        tracks: 80,
        sectors_per_track: 9,
    };

    /// Standard double-sided 720KB MSX disk (2 sides, 80 tracks, 9 sectors).
    pub const DS_720K: Geometry = Geometry {
        sides: 2,
        tracks: 80,
        sectors_per_track: 9,
    };

    /// 5.25" single-sided 180KB MSX disk (1 side, 40 tracks, 9 sectors).
    pub const SS_180K: Geometry = Geometry {
        sides: 1,
        tracks: 40,
        sectors_per_track: 9,
    };

    /// 5.25" double-sided 360KB MSX disk (2 sides, 40 tracks, 9 sectors).
    pub const DS_360K_525: Geometry = Geometry {
        sides: 2,
        tracks: 40,
        sectors_per_track: 9,
    };

    /// Total number of sectors on the disk.
    pub const fn total_sectors(&self) -> usize {
        self.sides as usize * self.tracks as usize * self.sectors_per_track as usize
    }

    /// Total size of the disk in bytes.
    pub const fn total_bytes(&self) -> usize {
        self.total_sectors() * SECTOR_SIZE
    }

    /// Physical `(track, head, sector)` of a logical sector index.
    ///
    /// Uses the canonical MSX track-by-track interleave the normalizers produce:
    /// sectors within a track come first, then the other side of the same track,
    /// then the next track. This is the inverse of the linear LBA layout the
    /// sector map iterates over.
    pub fn chs_of_lba(&self, lba: usize) -> (u16, u8, u8) {
        let spt = (self.sectors_per_track as usize).max(1);
        let sides = (self.sides as usize).max(1);
        let sector = (lba % spt) as u8;
        let head = ((lba / spt) % sides) as u8;
        let track = (lba / (spt * sides)) as u16;
        (track, head, sector)
    }

    /// Best-effort geometry for a raw image of `len` bytes.
    ///
    /// The two standard MSX sizes map to their exact geometries. Any other size
    /// that is a whole number of sectors gets a generic geometry (9 sectors per
    /// track, 2 sides when large enough) so the sector map can still render;
    /// `None` is returned only when `len` is not a sector multiple.
    pub fn for_raw_len(len: usize) -> Option<Geometry> {
        match len {
            SIZE_360K => Some(Geometry::SS_360K),
            SIZE_720K => Some(Geometry::DS_720K),
            _ if len > 0 && len.is_multiple_of(SECTOR_SIZE) => {
                let total = len / SECTOR_SIZE;
                let spt = 9u8;
                let sides = if total >= 2 * 80 * 9 { 2u8 } else { 1u8 };
                let per_side = total.div_ceil(sides as usize);
                let tracks = per_side.div_ceil(spt as usize) as u16;
                Some(Geometry {
                    sides,
                    tracks,
                    sectors_per_track: spt,
                })
            }
            _ => None,
        }
    }
}

/// A standard MSX floppy format selectable when creating a new blank disk.
///
/// Each variant pins both the physical geometry and the media descriptor, so
/// the two 360KB formats (3.5" single-sided vs 5.25" double-sided) stay
/// distinct even though they are the same byte size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskFormat {
    /// 3.5" single-sided, double density — 360KB (1 side, 80 tracks, media F8).
    Ss360,
    /// 3.5" double-sided, double density — 720KB (2 sides, 80 tracks, media F9).
    Ds720,
    /// 5.25" single-sided, double density — 180KB (1 side, 40 tracks, media FC).
    Ss180,
    /// 5.25" double-sided, double density — 360KB (2 sides, 40 tracks, media FD).
    Ds360,
}

impl DiskFormat {
    /// Every format, in the order shown in the New Disk chooser.
    pub const ALL: [DiskFormat; 4] = [
        DiskFormat::Ss360,
        DiskFormat::Ds720,
        DiskFormat::Ss180,
        DiskFormat::Ds360,
    ];

    /// Physical geometry of the format.
    pub const fn geometry(self) -> Geometry {
        match self {
            DiskFormat::Ss360 => Geometry::SS_360K,
            DiskFormat::Ds720 => Geometry::DS_720K,
            DiskFormat::Ss180 => Geometry::SS_180K,
            DiskFormat::Ds360 => Geometry::DS_360K_525,
        }
    }

    /// Total image size in bytes.
    pub const fn total_bytes(self) -> usize {
        self.geometry().total_bytes()
    }

    /// Human-readable name for the New Disk chooser.
    pub const fn label(self) -> &'static str {
        match self {
            DiskFormat::Ss360 => "3.5\" Single Sided, 360 kB (1DD)",
            DiskFormat::Ds720 => "3.5\" Double Sided, 720 kB (2DD)",
            DiskFormat::Ss180 => "5.25\" Single Sided, 180 kB (SS,DD)",
            DiskFormat::Ds360 => "5.25\" Double Sided, 360 kB (DS,DD)",
        }
    }

    /// Suggested file name when saving a freshly created disk.
    pub const fn default_file_name(self) -> &'static str {
        match self {
            DiskFormat::Ss360 => "blank-360-ss.dsk",
            DiskFormat::Ds720 => "blank-720.dsk",
            DiskFormat::Ss180 => "blank-180.dsk",
            DiskFormat::Ds360 => "blank-360-ds.dsk",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_format_sizes_match_geometry() {
        assert_eq!(DiskFormat::Ss360.total_bytes(), SIZE_360K);
        assert_eq!(DiskFormat::Ds720.total_bytes(), SIZE_720K);
        assert_eq!(DiskFormat::Ss180.total_bytes(), SIZE_180K);
        // The 5.25" double-sided disk is the same size as the 3.5" single-sided.
        assert_eq!(DiskFormat::Ds360.total_bytes(), SIZE_360K);
    }

    #[test]
    fn chs_of_lba_double_sided_interleaves_sides_within_a_track() {
        let g = Geometry::DS_720K; // 2 sides, 80 tracks, 9 spt
        assert_eq!(g.chs_of_lba(0), (0, 0, 0));
        assert_eq!(g.chs_of_lba(8), (0, 0, 8)); // last sector of track 0, side 0
        assert_eq!(g.chs_of_lba(9), (0, 1, 0)); // first sector of track 0, side 1
        assert_eq!(g.chs_of_lba(17), (0, 1, 8)); // last sector of track 0, side 1
        assert_eq!(g.chs_of_lba(18), (1, 0, 0)); // first sector of track 1, side 0
    }

    #[test]
    fn chs_of_lba_single_sided_advances_track_every_spt() {
        let g = Geometry::SS_360K; // 1 side, 80 tracks, 9 spt
        assert_eq!(g.chs_of_lba(0), (0, 0, 0));
        assert_eq!(g.chs_of_lba(8), (0, 0, 8));
        assert_eq!(g.chs_of_lba(9), (1, 0, 0));
        assert_eq!(g.chs_of_lba(18), (2, 0, 0));
    }
}
