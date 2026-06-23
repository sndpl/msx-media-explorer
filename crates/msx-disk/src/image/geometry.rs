//! Physical geometry of an MSX floppy disk.
//!
//! The "normalized" representation used throughout the crate is a flat byte
//! buffer holding every sector in logical (LBA) order — exactly what a plain
//! `.dsk` dump contains. Geometry is the metadata needed to render sector maps
//! and to reason about tracks/sides; the byte layout itself is always linear.

/// Bytes per sector on every supported MSX disk.
pub const SECTOR_SIZE: usize = 512;

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

    /// Total number of sectors on the disk.
    pub const fn total_sectors(&self) -> usize {
        self.sides as usize * self.tracks as usize * self.sectors_per_track as usize
    }

    /// Total size of the disk in bytes.
    pub const fn total_bytes(&self) -> usize {
        self.total_sectors() * SECTOR_SIZE
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
