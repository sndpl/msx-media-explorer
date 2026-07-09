//! GUI-side state for a currently-open disk.

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use std::collections::HashMap;

use msx_disk::cas::CasFile;
use msx_disk::fs::partition::{self, PartitionEntry};
use msx_disk::fs::write;
use msx_disk::fs::{FatType, Volume};
use msx_disk::image::geometry::Geometry;
use msx_disk::tape::{self, Tape, TapeFormat};
use msx_disk::{DirEntry, DiskFs, DiskImage, Error, ImageFormat};

/// The filesystem backing for an open disk: a single floppy volume (mounted via
/// `fatfs`) or a hard-disk image holding several MSX FAT partitions.
enum Backing {
    /// A single FAT volume starting at sector 0 — the floppy/`.dsk` path. Fully
    /// editable; behavior is unchanged from the single-volume design.
    Floppy { fs: DiskFs },
    /// A partitioned hard-disk image: one read-only [`Volume`] per partition,
    /// surfaced as synthetic `P{n}` top-level directory nodes.
    Partitioned { volumes: Vec<Volume> },
}

/// How many largest files the Stats view lists.
const LARGEST_FILES: usize = 12;

/// Everything the UI needs about the open disk, computed once on load.
pub struct LoadedDisk {
    pub path: Option<PathBuf>,
    pub format: ImageFormat,
    pub geometry: Geometry,
    pub label: Option<String>,
    pub tree: Vec<DirEntry>,
    image: DiskImage,
    backing: Backing,
    /// Lazily-computed whole-image checksums (the SHA-1 of a large HD image is
    /// too slow to compute eagerly on every open).
    image_checksums: OnceCell<msx_disk::Checksums>,
    /// Lazily-computed whole-disk statistics (single-volume floppies only).
    stats: OnceCell<Option<msx_disk::DiskStats>>,
    /// Lazily-computed per-partition statistics, one cell per volume.
    volume_stats: Vec<OnceCell<msx_disk::DiskStats>>,
}

impl LoadedDisk {
    /// Open and mount a disk image from a filesystem path.
    pub fn open(path: &Path) -> msx_disk::Result<LoadedDisk> {
        let image = DiskImage::open(path)?;
        LoadedDisk::from_image(image, Some(path.to_path_buf()))
    }

    /// Mount an already-decoded image, remembering its source path.
    ///
    /// Branches on whether the image is a partitioned hard disk: a single FAT
    /// volume takes the floppy path (editable, via `fatfs`); a partitioned image
    /// mounts each partition as a read-only [`Volume`] shown under a `P{n}` node.
    pub fn from_image(image: DiskImage, path: Option<PathBuf>) -> msx_disk::Result<LoadedDisk> {
        if partition::is_partitioned(image.data()) {
            return LoadedDisk::from_partitioned_image(image, path);
        }
        let fs = DiskFs::from_image(&image)?;
        let label = fs.volume_label();
        let tree = fs.tree()?;
        Ok(LoadedDisk {
            path,
            format: image.format(),
            geometry: image.geometry(),
            label,
            tree,
            image,
            backing: Backing::Floppy { fs },
            image_checksums: OnceCell::new(),
            stats: OnceCell::new(),
            volume_stats: Vec::new(),
        })
    }

    /// Mount a partitioned hard-disk image: one read-only volume per live
    /// partition, exposed as synthetic `P{n}` top-level directory nodes whose
    /// children are that volume's tree re-prefixed with `P{n}/`.
    fn from_partitioned_image(
        image: DiskImage,
        path: Option<PathBuf>,
    ) -> msx_disk::Result<LoadedDisk> {
        let entries = partition::parse_partition_table(image.data()).unwrap_or_default();
        let mut volumes = Vec::new();
        let mut tree = Vec::new();
        for entry in &entries {
            let Some(volume) = Volume::from_partition(&image, entry) else {
                continue;
            };
            let n = volumes.len() + 1;
            let prefix = format!("P{n}");
            let children = reprefix_tree(volume.tree(), &prefix);
            tree.push(DirEntry {
                name: partition_node_name(n, &volume, entry),
                path: prefix,
                is_dir: true,
                size: 0,
                attributes: Default::default(),
                modified: None,
                children,
            });
            volumes.push(volume);
        }
        if volumes.is_empty() {
            return Err(Error::Unsupported(
                "partitioned image has no readable FAT partitions".into(),
            ));
        }
        let volume_stats = volumes.iter().map(|_| OnceCell::new()).collect();
        Ok(LoadedDisk {
            path,
            format: image.format(),
            geometry: image.geometry(),
            // Labels live on the partition nodes for HD images.
            label: None,
            tree,
            image,
            backing: Backing::Partitioned { volumes },
            image_checksums: OnceCell::new(),
            stats: OnceCell::new(),
            volume_stats,
        })
    }

    /// Whether this is a partitioned hard-disk image (vs. a single floppy
    /// volume). Partitioned images are read-only and hide the Map view.
    pub fn is_partitioned(&self) -> bool {
        matches!(self.backing, Backing::Partitioned { .. })
    }

    /// Resolve a `P{n}/rel/path` into its volume and the volume-relative path.
    /// Returns `None` for the floppy backing or an out-of-range partition.
    fn resolve<'a>(&self, path: &'a str) -> Option<(&Volume, &'a str)> {
        let Backing::Partitioned { volumes } = &self.backing else {
            return None;
        };
        let (head, rest) = match path.split_once('/') {
            Some((h, r)) => (h, r),
            None => (path, ""),
        };
        let n: usize = head.strip_prefix('P')?.parse().ok()?;
        let volume = volumes.get(n.checked_sub(1)?)?;
        Some((volume, rest))
    }

    /// Read a file's bytes by its slash-separated path.
    pub fn read_file(&self, path: &str) -> msx_disk::Result<Vec<u8>> {
        match &self.backing {
            Backing::Floppy { fs } => fs.read_file(path),
            Backing::Partitioned { .. } => {
                let (volume, rel) = self
                    .resolve(path)
                    .ok_or_else(|| Error::Unsupported(format!("no such path '{path}'")))?;
                volume
                    .read_file(rel)
                    .ok_or_else(|| Error::Unsupported(format!("cannot read '{path}'")))
            }
        }
    }

    /// The disk's normalized sector data — a ready-to-write `.dsk` image.
    pub fn to_dsk_bytes(&self) -> Vec<u8> {
        self.image.data().to_vec()
    }

    /// Compress the disk's sectors into an `.xsa` image.
    pub fn to_xsa_bytes(&self) -> Vec<u8> {
        msx_disk::image::xsa::compress(self.image.data())
    }

    /// Whether exporting to `.dsk` is meaningful: a single-volume image whose
    /// source is not already `.dsk`. Partitioned HD images (not a single floppy)
    /// are excluded.
    pub fn can_convert_to_dsk(&self) -> bool {
        !self.is_partitioned() && !matches!(self.format, ImageFormat::Dsk)
    }

    /// Whether exporting to `.xsa` is meaningful: a single-volume image whose
    /// source is not already `.xsa`.
    pub fn can_convert_to_xsa(&self) -> bool {
        !self.is_partitioned() && !matches!(self.format, ImageFormat::Xsa)
    }

    /// Journal the disk's sectors into an MSXPLAYer `.sav` image.
    pub fn to_sav_bytes(&self) -> msx_disk::Result<Vec<u8>> {
        msx_disk::image::sav::encode(self.image.data())
    }

    /// Whether exporting to `.sav` is meaningful: a single-volume 720KB image
    /// (the only geometry the MSXPLAYer journal describes) whose source is not
    /// already `.sav`.
    pub fn can_convert_to_sav(&self) -> bool {
        !self.is_partitioned()
            && !matches!(self.format, ImageFormat::Sav)
            && self.image.data().len() == msx_disk::image::geometry::SIZE_720K
    }

    /// The raw normalized sector data (for disk-wide search).
    pub fn data(&self) -> &[u8] {
        self.image.data()
    }

    /// Number of 512-byte sectors on the disk.
    pub fn sector_count(&self) -> usize {
        self.image.sector_count()
    }

    /// A copy of sector `idx`'s 512 bytes, if in range.
    pub fn sector_bytes(&self, idx: usize) -> Option<Vec<u8>> {
        self.image.sector(idx).map(<[u8]>::to_vec)
    }

    /// A borrow of sector `idx`'s 512 bytes, if in range. Preferred over
    /// [`sector_bytes`](Self::sector_bytes) on the per-frame render path, where
    /// copying the sector every frame is wasteful.
    pub fn sector_slice(&self, idx: usize) -> Option<&[u8]> {
        self.image.sector(idx)
    }

    /// Overwrite a raw sector and persist the change to the source image.
    pub fn write_sector(&mut self, idx: usize, bytes: &[u8]) -> msx_disk::Result<()> {
        if self.is_partitioned() {
            return Err(Error::Unsupported(
                "hard-disk partitions are read-only".into(),
            ));
        }
        if bytes.len() != 512 {
            return Err(Error::Unsupported("a sector is 512 bytes".into()));
        }
        let mut data = self.image.data().to_vec();
        let start = idx * 512;
        if start + 512 > data.len() {
            return Err(Error::Unsupported("sector out of range".into()));
        }
        data[start..start + 512].copy_from_slice(bytes);
        self.write_back(data)
    }

    /// Classify every sector by usage. `None` for partitioned hard disks, whose
    /// Map view is disabled in this version.
    pub fn disk_map(&self) -> Option<msx_disk::fs::map::DiskMap> {
        if self.is_partitioned() {
            return None;
        }
        msx_disk::fs::map::disk_usage(self.image.data())
    }

    /// BPB-derived filesystem geometry (clusters, sectors, etc.) for the status
    /// bar, or `None` if the disk has no recognizable FAT BPB (and always for
    /// partitioned hard disks, which have no single whole-disk BPB).
    pub fn fs_geometry(&self) -> Option<msx_disk::fs::map::FsGeometry> {
        if self.is_partitioned() {
            return None;
        }
        msx_disk::fs::map::fs_geometry(self.image.data())
    }

    /// CRC32 + SHA-1 of the whole disk image, computed once and cached.
    pub fn checksums(&self) -> &msx_disk::Checksums {
        self.image_checksums
            .get_or_init(|| msx_disk::verify::image_checksums(self.image.data()))
    }

    /// Whole-disk statistics for a single-volume floppy, computed once and
    /// cached. `None` for partitioned hard disks (use [`LoadedDisk::volume_stats`]).
    pub fn stats(&self) -> Option<&msx_disk::DiskStats> {
        if self.is_partitioned() {
            return None;
        }
        self.stats
            .get_or_init(|| msx_disk::stats::disk_stats(self.image.data(), LARGEST_FILES))
            .as_ref()
    }

    /// The number of partitions on a hard-disk image (0 for a floppy).
    pub fn partition_count(&self) -> usize {
        match &self.backing {
            Backing::Partitioned { volumes } => volumes.len(),
            Backing::Floppy { .. } => 0,
        }
    }

    /// Statistics for partition `i` of a hard-disk image, computed once and
    /// cached. `None` for a floppy or an out-of-range index.
    pub fn volume_stats(&self, i: usize) -> Option<&msx_disk::DiskStats> {
        let Backing::Partitioned { volumes } = &self.backing else {
            return None;
        };
        let vol = volumes.get(i)?;
        Some(
            self.volume_stats
                .get(i)?
                .get_or_init(|| vol.stats(LARGEST_FILES)),
        )
    }

    /// The sectors occupied by a file (its cluster chain), in whole-image sector
    /// indices. For partitions the chain is resolved within the volume and
    /// offset by the partition's LBA.
    pub fn file_sectors(&self, path: &str) -> Vec<usize> {
        match &self.backing {
            Backing::Floppy { .. } => msx_disk::fs::map::file_sectors(self.image.data(), path),
            Backing::Partitioned { .. } => match self.resolve(path) {
                Some((volume, rel)) => volume.file_sectors_absolute(rel),
                None => Vec::new(),
            },
        }
    }

    /// Find a companion file: the sibling of `base_path` (same directory and
    /// stem) whose extension is `ext`, matched case-insensitively.
    pub fn companion(&self, base_path: &str, ext: &str) -> Option<Vec<u8>> {
        let dir = base_path.rsplit_once('/').map(|(d, _)| d);
        let stem = base_path
            .rsplit('/')
            .next()
            .and_then(|n| n.rsplit_once('.').map(|(s, _)| s).or(Some(n)))?;
        let target = format!("{stem}.{ext}");
        for entry in self.tree.iter().flat_map(DirEntry::walk) {
            if entry.is_dir {
                continue;
            }
            let entry_dir = entry.path.rsplit_once('/').map(|(d, _)| d);
            if entry_dir == dir && entry.name.eq_ignore_ascii_case(&target) {
                return self.read_file(&entry.path).ok();
            }
        }
        None
    }

    /// Whether this disk can be modified in place (a single-volume floppy with a
    /// path and a writable container). Partitioned hard disks are read-only.
    pub fn writable(&self) -> bool {
        !self.is_partitioned() && self.path.is_some() && self.image.is_writable()
    }

    /// Add (or overwrite) files at the root of the disk.
    pub fn add_files(&mut self, files: &[(String, Vec<u8>)]) -> msx_disk::Result<()> {
        if self.is_partitioned() {
            return Err(Error::Unsupported(
                "hard-disk partitions are read-only".into(),
            ));
        }
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let updated = write::add_files(self.image.data(), &refs)?;
        self.write_back(updated)
    }

    /// Detect a boot-sector/image-size mismatch that can be safely repaired.
    /// Limited to raw, writable single-volume images (`.dsk`/`.msx`), where
    /// resizing the file and relabelling the boot sector are both safe.
    pub fn size_mismatch(&self) -> Option<msx_disk::fs::sizefix::SizeMismatch> {
        if self.is_partitioned() || self.path.is_none() || !self.image.is_writable() {
            return None;
        }
        if !matches!(self.format, ImageFormat::Dsk | ImageFormat::Msx) {
            return None;
        }
        msx_disk::fs::sizefix::detect(self.image.data())
    }

    /// Apply a geometry repair and write the result back to the source file.
    pub fn apply_size_fix(
        &mut self,
        mismatch: &msx_disk::fs::sizefix::SizeMismatch,
    ) -> msx_disk::Result<()> {
        let fixed = msx_disk::fs::sizefix::repair(self.image.data(), mismatch);
        self.write_back(fixed)
    }

    /// Create a directory at a slash-separated path (parent must already exist).
    pub fn create_dir(&mut self, path: &str) -> msx_disk::Result<()> {
        if self.is_partitioned() {
            return Err(Error::Unsupported(
                "hard-disk partitions are read-only".into(),
            ));
        }
        let updated = write::create_dir(self.image.data(), path)?;
        self.write_back(updated)
    }

    /// Delete files by their slash-separated paths.
    pub fn delete(&mut self, paths: &[String]) -> msx_disk::Result<()> {
        if self.is_partitioned() {
            return Err(Error::Unsupported(
                "hard-disk partitions are read-only".into(),
            ));
        }
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let updated = write::delete(self.image.data(), &refs)?;
        self.write_back(updated)
    }

    /// Rename a file from `old_path` to `new_path` (both relative to the root).
    pub fn rename(&mut self, old_path: &str, new_path: &str) -> msx_disk::Result<()> {
        if self.is_partitioned() {
            return Err(Error::Unsupported(
                "hard-disk partitions are read-only".into(),
            ));
        }
        let updated = write::rename(self.image.data(), old_path, new_path)?;
        self.write_back(updated)
    }

    /// Re-encode modified sector data into the original container, write it to
    /// the source file, and reload so all derived state is refreshed.
    fn write_back(&mut self, updated: Vec<u8>) -> msx_disk::Result<()> {
        let bytes = self.image.reencode(&updated)?;
        let path = self
            .path
            .clone()
            .ok_or_else(|| Error::Unsupported("disk has no file to save to".into()))?;
        std::fs::write(&path, bytes)?;
        *self = LoadedDisk::open(&path)?;
        Ok(())
    }

    /// Short human-readable name for the open image.
    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(in-memory image)".to_string())
    }
}

/// Re-prefix every path in `entries` (recursively) with `prefix/`, so a
/// volume's `A/B.TXT` becomes `P1/A/B.TXT` once mounted under its partition node.
fn reprefix_tree(entries: Vec<DirEntry>, prefix: &str) -> Vec<DirEntry> {
    entries
        .into_iter()
        .map(|e| DirEntry {
            path: format!("{prefix}/{}", e.path),
            children: reprefix_tree(e.children, prefix),
            ..e
        })
        .collect()
}

/// Label for a partition's synthetic top-level node, e.g.
/// `Partition 1 — FAT12, 32 MB, VOL_ID`.
fn partition_node_name(n: usize, volume: &Volume, entry: &PartitionEntry) -> String {
    let fat = match volume.fat_type() {
        FatType::Fat12 => "FAT12",
        FatType::Fat16 => "FAT16",
    };
    let size = humanize_bytes(entry.sector_count as u64 * 512);
    match volume.volume_label() {
        Some(label) => format!("Partition {n} \u{2014} {fat}, {size}, {label}"),
        None => format!("Partition {n} \u{2014} {fat}, {size}"),
    }
}

/// Format a byte count as a human-readable size using SI (decimal, 1000-based)
/// prefixes with two decimals, e.g. `134.22 MB`. Ported from Kohana's
/// `Num::bytes()`, picking the largest unit that keeps the value at least 1.
pub(crate) fn humanize_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    const MOD: f64 = 1000.0;
    let power = if bytes > 0 {
        ((bytes as f64).log(MOD).floor() as usize).min(UNITS.len() - 1)
    } else {
        0
    };
    format!(
        "{:.2} {}",
        bytes as f64 / MOD.powi(power as i32),
        UNITS[power]
    )
}

/// A currently-open tape image (`.cas` / `.tsx`): its blocks for the overview
/// and the logical files derived from them, with unique selection keys.
pub struct LoadedTape {
    pub path: PathBuf,
    pub format: TapeFormat,
    pub tape: Tape,
    files: Vec<CasFile>,
    /// One stable, unique key per file (the name, de-duplicated on collision).
    keys: Vec<String>,
}

impl LoadedTape {
    /// Open and parse a tape image from a filesystem path.
    pub fn open(path: &Path) -> msx_disk::Result<LoadedTape> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let format = TapeFormat::from_extension(ext)
            .ok_or_else(|| Error::Unsupported("not a tape image".into()))?;
        let bytes = std::fs::read(path)?;
        let tape = tape::open(&bytes, format);
        let files = tape.files();

        let mut keys = Vec::with_capacity(files.len());
        let mut seen: HashMap<String, usize> = HashMap::new();
        for file in &files {
            let base = if file.name.is_empty() {
                "FILE".to_string()
            } else {
                file.name.clone()
            };
            let count = seen.entry(base.clone()).or_insert(0);
            keys.push(if *count == 0 {
                base.clone()
            } else {
                format!("{base}.{count}")
            });
            *count += 1;
        }

        Ok(LoadedTape {
            path: path.to_path_buf(),
            format,
            tape,
            files,
            keys,
        })
    }

    /// `(key, file)` pairs in tape order, for the file list.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &CasFile)> {
        self.keys.iter().map(String::as_str).zip(&self.files)
    }

    /// The payload bytes of the file identified by `key`.
    pub fn read_file(&self, key: &str) -> Option<Vec<u8>> {
        let idx = self.keys.iter().position(|k| k == key)?;
        Some(self.files[idx].data.clone())
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn total_bytes(&self) -> usize {
        self.tape.total_file_bytes()
    }

    /// Short human-readable name for the open tape.
    pub fn title(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(tape)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/MSX-DOS2 TOOLS.dsk");
        path.exists().then_some(path)
    }

    fn hd_fixture() -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/hd.dsk");
        path.exists().then_some(path)
    }

    #[test]
    fn size_fix_truncates_single_sided_disk_in_720_container() {
        use msx_disk::image::geometry::{DiskFormat, SIZE_360K, SIZE_720K};
        // A real 360KB disk padded into a 720KB container, written to a temp file.
        let mut bytes = msx_disk::fs::write::create_blank(DiskFormat::Ss360).expect("blank 360");
        bytes.resize(SIZE_720K, 0);
        let tmp = std::env::temp_dir().join("mediaexplorer_sizefix_test.dsk");
        std::fs::write(&tmp, &bytes).expect("write temp");

        let mut disk = LoadedDisk::open(&tmp).expect("open");
        let m = disk.size_mismatch().expect("mismatch detected");
        assert_eq!(m.real_bytes, SIZE_360K);
        disk.apply_size_fix(&m).expect("apply");

        // The file on disk is now exactly 360KB and free of further mismatch.
        let reopened = LoadedDisk::open(&tmp).expect("reopen");
        assert_eq!(reopened.data().len(), SIZE_360K);
        assert!(reopened.size_mismatch().is_none());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn save_as_predicates_track_source_format() {
        use msx_disk::image::geometry::DiskFormat;
        // A plain .dsk is already in .dsk form, so "Save as .dsk" is pointless,
        // but it can still be compressed to .xsa.
        let bytes = msx_disk::fs::write::create_blank(DiskFormat::Ss360).expect("blank");
        let dsk_path = std::env::temp_dir().join("mediaexplorer_saveas_test.dsk");
        std::fs::write(&dsk_path, &bytes).expect("write dsk");
        let dsk = LoadedDisk::open(&dsk_path).expect("open dsk");
        assert!(!dsk.can_convert_to_dsk(), "already a .dsk");
        assert!(dsk.can_convert_to_xsa(), ".dsk can be compressed to .xsa");
        assert!(
            !dsk.can_convert_to_sav(),
            "a 360KB disk cannot be a .sav journal (720KB only)"
        );

        // Compress it to .xsa and reopen: the reverse now holds.
        let xsa_path = std::env::temp_dir().join("mediaexplorer_saveas_test.xsa");
        std::fs::write(&xsa_path, dsk.to_xsa_bytes()).expect("write xsa");
        let xsa = LoadedDisk::open(&xsa_path).expect("open xsa");
        assert!(xsa.can_convert_to_dsk(), ".xsa can be saved as .dsk");
        assert!(!xsa.can_convert_to_xsa(), "already a .xsa");

        // A 720KB disk journals to .sav and back; a .sav is never re-offered.
        let bytes720 = msx_disk::fs::write::create_blank(DiskFormat::Ds720).expect("blank 720");
        let dsk720_path = std::env::temp_dir().join("mediaexplorer_saveas_test720.dsk");
        std::fs::write(&dsk720_path, &bytes720).expect("write 720 dsk");
        let dsk720 = LoadedDisk::open(&dsk720_path).expect("open 720 dsk");
        assert!(
            dsk720.can_convert_to_sav(),
            "720KB .dsk can journal to .sav"
        );
        let sav_path = std::env::temp_dir().join("mediaexplorer_saveas_test.sav");
        std::fs::write(&sav_path, dsk720.to_sav_bytes().expect("encode sav")).expect("write sav");
        let sav = LoadedDisk::open(&sav_path).expect("open sav");
        assert!(sav.can_convert_to_dsk(), ".sav can be saved as .dsk");
        assert!(!sav.can_convert_to_sav(), "already a .sav");

        let _ = std::fs::remove_file(&dsk_path);
        let _ = std::fs::remove_file(&xsa_path);
        let _ = std::fs::remove_file(&dsk720_path);
        let _ = std::fs::remove_file(&sav_path);
    }

    #[test]
    fn partitioned_hd_cannot_be_converted() {
        let Some(path) = hd_fixture() else {
            eprintln!("skipping: hd.dsk fixture not present");
            return;
        };
        let disk = LoadedDisk::open(&path).expect("open hd.dsk");
        assert!(disk.is_partitioned());
        // A multi-partition HD image is not a single floppy; both exports off.
        assert!(!disk.can_convert_to_dsk());
        assert!(!disk.can_convert_to_xsa());
    }

    #[test]
    fn humanize_bytes_picks_units() {
        assert_eq!(humanize_bytes(0), "0.00 B");
        assert_eq!(humanize_bytes(512), "512.00 B");
        assert_eq!(humanize_bytes(2048), "2.05 kB");
        assert_eq!(humanize_bytes(32 * 1024 * 1024), "33.55 MB");
        // The 128 MiB hard-disk size that previously read "131076 KB".
        assert_eq!(humanize_bytes(131_076 * 1024), "134.22 MB");
        assert_eq!(humanize_bytes(2 * 1000 * 1000 * 1000), "2.00 GB");
    }

    #[test]
    fn reprefix_tree_rewrites_nested_paths() {
        let tree = vec![DirEntry {
            name: "SUB".into(),
            path: "SUB".into(),
            is_dir: true,
            size: 0,
            attributes: Default::default(),
            modified: None,
            children: vec![DirEntry {
                name: "A.TXT".into(),
                path: "SUB/A.TXT".into(),
                is_dir: false,
                size: 3,
                attributes: Default::default(),
                modified: None,
                children: Vec::new(),
            }],
        }];
        let prefixed = reprefix_tree(tree, "P1");
        assert_eq!(prefixed[0].path, "P1/SUB");
        assert_eq!(prefixed[0].children[0].path, "P1/SUB/A.TXT");
    }

    #[test]
    fn hd_image_opens_as_four_readonly_partition_nodes() {
        let Some(path) = hd_fixture() else {
            eprintln!("skipping: hd.dsk fixture not present");
            return;
        };
        let disk = LoadedDisk::open(&path).expect("open hd.dsk");
        assert!(disk.is_partitioned());
        // Four synthetic P{n} top-level nodes.
        assert_eq!(disk.tree.len(), 4);
        for (i, node) in disk.tree.iter().enumerate() {
            assert_eq!(node.path, format!("P{}", i + 1));
            assert!(node.is_dir);
            assert!(node.name.starts_with(&format!("Partition {}", i + 1)));
            assert!(node.name.contains("FAT12"));
        }
        // Read-only: no editing, no whole-disk map, no single FS geometry.
        assert!(!disk.writable());
        assert!(disk.fs_geometry().is_none());
        assert!(disk.disk_map().is_none());
        assert!(disk.label.is_none());

        // A file under P1 reads back with a length matching its dir-entry size.
        let file = disk
            .tree
            .iter()
            .flat_map(|e| e.walk())
            .find(|e| !e.is_dir && e.path.starts_with("P1/"))
            .expect("a file under P1");
        let bytes = disk.read_file(&file.path).expect("read P1 file");
        assert_eq!(bytes.len() as u64, file.size);
    }

    #[test]
    fn hd_write_operations_are_rejected() {
        let Some(path) = hd_fixture() else {
            eprintln!("skipping: hd.dsk fixture not present");
            return;
        };
        let mut disk = LoadedDisk::open(&path).expect("open hd.dsk");
        assert!(disk
            .add_files(&[("X.TXT".to_string(), b"x".to_vec())])
            .is_err());
        assert!(disk.delete(&["P1/ANY".to_string()]).is_err());
        assert!(disk.rename("P1/A", "P1/B").is_err());
        assert!(disk.write_sector(0, &[0u8; 512]).is_err());
    }

    #[test]
    fn blank_disk_exposes_stats_and_checksums() {
        let image = DiskImage::open_bytes(ImageFormat::Dsk, vec![0u8; 720 * 1024]).expect("image");
        let disk = LoadedDisk::from_image(image, None).expect("mount");
        // The cached whole-image checksums equal a direct computation.
        assert_eq!(
            *disk.checksums(),
            msx_disk::verify::image_checksums(disk.data())
        );
        // A blank floppy reports zero files and a clean filesystem.
        let stats = disk.stats().expect("stats");
        assert_eq!(stats.file_count, 0);
        assert!(stats.integrity.is_clean());
        // Floppies have no partitions.
        assert_eq!(disk.partition_count(), 0);
        assert!(disk.volume_stats(0).is_none());
    }

    #[test]
    fn hd_image_exposes_per_partition_stats() {
        let Some(path) = hd_fixture() else {
            eprintln!("skipping: hd.dsk fixture not present");
            return;
        };
        let disk = LoadedDisk::open(&path).expect("open hd.dsk");
        // Whole-disk stats are unavailable for partitioned images.
        assert!(disk.stats().is_none());
        assert_eq!(disk.partition_count(), 4);
        // Each partition has its own statistics.
        for i in 0..disk.partition_count() {
            assert!(disk.volume_stats(i).is_some(), "partition {i} stats");
        }
        assert!(disk.volume_stats(99).is_none());
    }

    #[test]
    fn blank_disk_mounts_with_empty_tree() {
        // A zeroed image mounts as a blank FAT volume (boot sector repaired),
        // so its FAT directory holds no entries — the sector-based / empty-FAT
        // case the Files panel must surface with a message.
        let image = DiskImage::open_bytes(ImageFormat::Dsk, vec![0u8; 720 * 1024]).expect("image");
        let disk = LoadedDisk::from_image(image, None).expect("mount blank disk");
        assert!(!disk.is_partitioned());
        assert!(disk.tree.is_empty());
    }

    #[test]
    fn loads_fixture_disk_when_present() {
        let Some(path) = fixture() else {
            eprintln!("skipping: fixture not present");
            return;
        };
        let disk = LoadedDisk::open(&path).expect("load");
        assert_eq!(disk.title(), "MSX-DOS2 TOOLS.dsk");
        assert!(!disk.tree.is_empty());
        let first_file = disk
            .tree
            .iter()
            .flat_map(|e| e.walk())
            .find(|e| !e.is_dir)
            .expect("a file");
        assert_eq!(
            disk.read_file(&first_file.path).unwrap().len() as u64,
            first_file.size
        );
    }

    #[test]
    fn full_save_path_roundtrip_on_temp_copy() {
        let Some(src) = fixture() else {
            eprintln!("skipping: fixture not present");
            return;
        };
        // Work on a temp copy so the real fixture is never modified.
        let tmp = std::env::temp_dir().join("mediaexplorer_phase2_test.dsk");
        std::fs::copy(&src, &tmp).expect("copy fixture");

        let mut disk = LoadedDisk::open(&tmp).expect("open");
        assert!(disk.writable());
        disk.add_files(&[("PHASE2.TXT".to_string(), b"hi".to_vec())])
            .expect("add");

        // Reopen from disk to prove it persisted.
        let reopened = LoadedDisk::open(&tmp).expect("reopen");
        assert_eq!(reopened.read_file("PHASE2.TXT").unwrap(), b"hi");

        let mut disk = reopened;
        disk.delete(&["PHASE2.TXT".to_string()]).expect("delete");
        let reopened = LoadedDisk::open(&tmp).expect("reopen2");
        assert!(reopened.read_file("PHASE2.TXT").is_err());

        // Create a subdirectory and add a file into it, then reopen to confirm.
        let mut disk = reopened;
        disk.create_dir("NEWDIR").expect("create dir");
        disk.add_files(&[("NEWDIR/INSIDE.TXT".to_string(), b"in".to_vec())])
            .expect("add into subdir");
        let reopened = LoadedDisk::open(&tmp).expect("reopen3");
        assert_eq!(reopened.read_file("NEWDIR/INSIDE.TXT").unwrap(), b"in");

        let _ = std::fs::remove_file(&tmp);
    }
}
