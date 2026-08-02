//! GUI-side state for a currently-open disk.

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use std::collections::HashMap;

use msx_disk::cas::CasFile;
use msx_disk::fs::multidisk::{self, DiskSlice};
use msx_disk::fs::partition::{self, PartitionEntry};
use msx_disk::fs::write;
use msx_disk::fs::{FatType, Volume};
use msx_disk::image::geometry::Geometry;
use msx_disk::image::zip;
use msx_disk::tape::{self, Tape, TapeFormat};
use msx_disk::{DirEntry, DiskFs, DiskImage, Error, ImageFormat};

/// The filesystem backing for an open disk: a single floppy volume (mounted via
/// `fatfs`) or a hard-disk image holding several MSX FAT partitions.
enum Backing {
    /// A single FAT volume starting at sector 0 — the floppy/`.dsk` path. Fully
    /// editable; behavior is unchanged from the single-volume design. Boxed
    /// because a mounted `fatfs` volume dwarfs the partitioned variant.
    Floppy { fs: Box<DiskFs> },
    /// A partitioned hard-disk image: one read-only [`Volume`] per partition,
    /// surfaced as synthetic `P{n}` top-level directory nodes.
    Partitioned { volumes: Vec<Volume> },
}

/// What the synthetic top-level volume nodes of a multi-volume image stand for.
/// Both are mounted and browsed identically; they differ only in how the nodes
/// are named and keyed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// A partition of an openMSX `MSX_IDE` hard-disk image.
    Partition,
    /// A whole floppy inside a file holding several of them back to back.
    Disk,
    /// A disk image stored inside a `.zip` archive.
    Zip,
}

impl VolumeKind {
    /// The letter that starts a volume's path prefix (`P1/…` or `D1/…`).
    fn prefix(self) -> char {
        match self {
            VolumeKind::Partition => 'P',
            VolumeKind::Disk | VolumeKind::Zip => 'D',
        }
    }
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
    /// Byte size of each partition (from its partition-table entry), for the
    /// status bar. Empty for a single-volume floppy.
    partition_sizes: Vec<u64>,
    /// Lazily-computed whole-image checksums (the SHA-1 of a large HD image is
    /// too slow to compute eagerly on every open).
    image_checksums: OnceCell<msx_disk::Checksums>,
    /// Lazily-computed whole-disk statistics (single-volume floppies only).
    stats: OnceCell<Option<msx_disk::DiskStats>>,
    /// Lazily-computed per-partition statistics, one cell per volume.
    volume_stats: Vec<OnceCell<msx_disk::DiskStats>>,
    /// What the synthetic top-level nodes stand for. Only meaningful for a
    /// multi-volume image; a plain floppy leaves it at `Partition`.
    volume_kind: VolumeKind,
    /// First sector (LBA) of each volume, parallel to `partition_sizes` and to
    /// the top-level tree nodes. Empty for a single-volume floppy.
    volume_starts: Vec<usize>,
    /// Short label for each volume: `Partition 1`, `Disk 1`, or — for a zip —
    /// the member's own file name, which is worth showing verbatim.
    volume_labels: Vec<String>,
    /// Lazily-computed per-volume checksums. Only meaningful for a zip, whose
    /// whole-"image" is a synthetic concatenation; each member's own CRC32 and
    /// SHA-1 are what match a software database.
    volume_checksums: Vec<OnceCell<msx_disk::Checksums>>,
}

impl LoadedDisk {
    /// Open and mount a disk image from a filesystem path.
    ///
    /// A `.zip` takes its own route so the archive is inflated once and its
    /// member table reused, rather than decompressed again to find out what is
    /// inside it.
    pub fn open(path: &Path) -> msx_disk::Result<LoadedDisk> {
        let bytes = std::fs::read(path)?;
        if zip::is_zip(&bytes) {
            let archive = zip::open(&bytes)?;
            return LoadedDisk::from_archive(archive, Some(path.to_path_buf()));
        }
        let image = DiskImage::open(path)?;
        LoadedDisk::from_image(image, Some(path.to_path_buf()))
    }

    /// Mount the disk images unpacked from a `.zip`: one read-only volume per
    /// member, named after the member itself, exposed as synthetic `D{n}`
    /// top-level nodes exactly as a concatenated multi-disk file is.
    ///
    /// Read-only: writing back would mean recompressing and rewriting an
    /// archive that may hold files we do not own.
    pub fn from_archive(
        archive: zip::Archive,
        path: Option<PathBuf>,
    ) -> msx_disk::Result<LoadedDisk> {
        let mut volumes = Vec::new();
        let mut partition_sizes = Vec::new();
        let mut volume_starts = Vec::new();
        let mut volume_labels = Vec::new();
        let mut tree = Vec::new();
        for member in &archive.members {
            let Some(volume) =
                Volume::from_image_slice(&archive.data, member.lba_start, member.sector_count)
            else {
                continue;
            };
            let n = volumes.len() + 1;
            let prefix = format!("{}{n}", VolumeKind::Zip.prefix());
            let children = reprefix_tree(volume.tree(), &prefix);
            tree.push(DirEntry {
                name: archive_node_name(member, &volume),
                path: prefix,
                is_dir: true,
                size: 0,
                attributes: Default::default(),
                modified: None,
                children,
            });
            partition_sizes.push(member.byte_len() as u64);
            volume_starts.push(member.lba_start);
            volume_labels.push(member.name.clone());
            volumes.push(volume);
        }
        if volumes.is_empty() {
            return Err(Error::Unsupported(
                "no readable disk image in the archive".into(),
            ));
        }
        let geometry =
            Geometry::for_raw_len(archive.members[0].byte_len()).unwrap_or(Geometry::DS_720K);
        let volume_stats = volumes.iter().map(|_| OnceCell::new()).collect();
        let volume_checksums = volumes.iter().map(|_| OnceCell::new()).collect();
        let image = DiskImage::from_normalized(ImageFormat::Zip, archive.data)?;
        Ok(LoadedDisk {
            path,
            format: image.format(),
            // One member's shape, not the concatenation's: the file itself is
            // not a disk any drive could hold.
            geometry,
            // Labels live on the per-member nodes.
            label: None,
            tree,
            image,
            backing: Backing::Partitioned { volumes },
            partition_sizes,
            image_checksums: OnceCell::new(),
            stats: OnceCell::new(),
            volume_stats,
            volume_kind: VolumeKind::Zip,
            volume_starts,
            volume_labels,
            volume_checksums,
        })
    }

    /// Mount an already-decoded image, remembering its source path.
    ///
    /// Branches on the image's shape: a partitioned hard disk mounts one
    /// read-only [`Volume`] per partition under a `P{n}` node, a file holding
    /// several whole floppies back to back does the same per disk under `D{n}`,
    /// and anything else takes the single-volume floppy path (editable, via
    /// `fatfs`).
    pub fn from_image(image: DiskImage, path: Option<PathBuf>) -> msx_disk::Result<LoadedDisk> {
        if partition::is_partitioned(image.data()) {
            return LoadedDisk::from_partitioned_image(image, path);
        }
        if let Some(slices) = multidisk::split(image.data()) {
            return LoadedDisk::from_concatenated_image(image, path, &slices);
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
            backing: Backing::Floppy { fs: Box::new(fs) },
            partition_sizes: Vec::new(),
            image_checksums: OnceCell::new(),
            stats: OnceCell::new(),
            volume_stats: Vec::new(),
            volume_kind: VolumeKind::Partition,
            volume_starts: Vec::new(),
            volume_labels: Vec::new(),
            volume_checksums: Vec::new(),
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
        let mut partition_sizes = Vec::new();
        let mut volume_starts = Vec::new();
        let mut volume_labels = Vec::new();
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
            partition_sizes.push(entry.sector_count as u64 * 512);
            volume_starts.push(entry.lba_start as usize);
            volume_labels.push(format!("Partition {n}"));
            volumes.push(volume);
        }
        if volumes.is_empty() {
            return Err(Error::Unsupported(
                "partitioned image has no readable FAT partitions".into(),
            ));
        }
        let volume_stats = volumes.iter().map(|_| OnceCell::new()).collect();
        let volume_checksums = volumes.iter().map(|_| OnceCell::new()).collect();
        Ok(LoadedDisk {
            path,
            format: image.format(),
            geometry: image.geometry(),
            // Labels live on the partition nodes for HD images.
            label: None,
            tree,
            image,
            backing: Backing::Partitioned { volumes },
            partition_sizes,
            image_checksums: OnceCell::new(),
            stats: OnceCell::new(),
            volume_stats,
            volume_kind: VolumeKind::Partition,
            volume_starts,
            volume_labels,
            volume_checksums,
        })
    }

    /// Mount a file holding several whole floppies back to back: one read-only
    /// volume per disk, exposed as synthetic `D{n}` top-level nodes, exactly as
    /// a hard disk's partitions are. Writing back into one slice would need a
    /// slice-aware re-encode, so these are read-only for now.
    fn from_concatenated_image(
        image: DiskImage,
        path: Option<PathBuf>,
        slices: &[DiskSlice],
    ) -> msx_disk::Result<LoadedDisk> {
        let mut volumes = Vec::new();
        let mut partition_sizes = Vec::new();
        let mut volume_starts = Vec::new();
        let mut volume_labels = Vec::new();
        let mut tree = Vec::new();
        for slice in slices {
            let Some(volume) =
                Volume::from_image_slice(image.data(), slice.lba_start, slice.sector_count)
            else {
                continue;
            };
            let n = volumes.len() + 1;
            let prefix = format!("{}{n}", VolumeKind::Disk.prefix());
            let children = reprefix_tree(volume.tree(), &prefix);
            tree.push(DirEntry {
                name: disk_node_name(n, &volume),
                path: prefix,
                is_dir: true,
                size: 0,
                attributes: Default::default(),
                modified: None,
                children,
            });
            partition_sizes.push(slice.byte_len() as u64);
            volume_starts.push(slice.lba_start);
            volume_labels.push(format!("Disk {n}"));
            volumes.push(volume);
        }
        if volumes.is_empty() {
            return Err(Error::Unsupported(
                "concatenated image has no readable disks".into(),
            ));
        }
        let volume_stats = volumes.iter().map(|_| OnceCell::new()).collect();
        let volume_checksums = volumes.iter().map(|_| OnceCell::new()).collect();
        Ok(LoadedDisk {
            path,
            format: image.format(),
            // The physical shape of one disk, not of the whole file: the file
            // itself is not a disk any drive could hold.
            geometry: slices[0].geometry(),
            // Labels live on the per-disk nodes.
            label: None,
            tree,
            image,
            backing: Backing::Partitioned { volumes },
            partition_sizes,
            image_checksums: OnceCell::new(),
            stats: OnceCell::new(),
            volume_stats,
            volume_kind: VolumeKind::Disk,
            volume_starts,
            volume_labels,
            volume_checksums,
        })
    }

    /// How many root-directory entries could not describe real files and were
    /// therefore left out of the tree. Always 0 for a partitioned hard disk,
    /// whose volumes take a different mount path.
    pub fn invalid_entries(&self) -> usize {
        match &self.backing {
            Backing::Floppy { fs } => fs.invalid_entries().len(),
            Backing::Partitioned { .. } => 0,
        }
    }

    /// Whether the disk has no usable FAT filesystem: it has directory entries
    /// and every one of them is structurally impossible. Typical of a
    /// custom-format game disk whose loader reads raw sectors.
    pub fn has_no_filesystem(&self) -> bool {
        match &self.backing {
            Backing::Floppy { fs } => fs.has_no_filesystem(),
            Backing::Partitioned { .. } => false,
        }
    }

    /// Whether this image holds several volumes rather than one floppy: a
    /// partitioned hard disk, or several whole disks concatenated. Both are
    /// read-only and hide the whole-disk Map view.
    pub fn is_partitioned(&self) -> bool {
        matches!(self.backing, Backing::Partitioned { .. })
    }

    /// Whether the volumes are whole disks concatenated into one file, rather
    /// than partitions of a hard disk.
    pub fn is_multi_disk(&self) -> bool {
        self.is_partitioned() && self.volume_kind == VolumeKind::Disk
    }

    /// Byte length of the whole image file.
    pub fn image_bytes(&self) -> u64 {
        self.image.data().len() as u64
    }

    /// Short navigation labels for the volumes of a multi-volume image, paired
    /// with each one's first sector. Empty for a single-volume floppy.
    pub fn volume_jumps(&self) -> Vec<(String, usize)> {
        self.volume_labels
            .iter()
            .cloned()
            .zip(self.volume_starts.iter().copied())
            .collect()
    }

    /// Whether the volumes are disk images unpacked from a `.zip`.
    pub fn is_zip(&self) -> bool {
        self.volume_kind == VolumeKind::Zip
    }

    /// CRC32/SHA-1 of volume `n`'s own bytes, computed once.
    ///
    /// For a zip these are what a software database can be searched by; the
    /// whole-"image" checksum would be of a concatenation that exists in no
    /// file.
    pub fn volume_checksums(&self, n: usize) -> Option<&msx_disk::Checksums> {
        let cell = self.volume_checksums.get(n)?;
        let data = self.volume_data(n)?;
        Some(cell.get_or_init(|| msx_disk::Checksums::of(data)))
    }

    /// Which volume an absolute sector falls in, as its zero-based index and
    /// the sector's offset within that volume. `None` for a single-volume
    /// floppy or a sector before the first volume.
    pub fn locate_sector(&self, sector: usize) -> Option<(usize, usize)> {
        let (i, &start) = self
            .volume_starts
            .iter()
            .enumerate()
            .rev()
            .find(|(_, &start)| start <= sector)?;
        Some((i, sector - start))
    }

    /// The raw bytes of volume `n` (zero-based), for exporting one disk of a
    /// concatenated image as a file of its own.
    pub fn volume_data(&self, n: usize) -> Option<&[u8]> {
        let start = *self.volume_starts.get(n)? * 512;
        let len = *self.partition_sizes.get(n)? as usize;
        self.image.data().get(start..start + len)
    }

    /// Resolve a `P{n}/rel/path` (or `D{n}/…` for concatenated disks) into its
    /// volume and the volume-relative path. Returns `None` for the floppy
    /// backing or an out-of-range volume.
    fn resolve<'a>(&self, path: &'a str) -> Option<(&Volume, &'a str)> {
        let Backing::Partitioned { volumes } = &self.backing else {
            return None;
        };
        let (head, rest) = match path.split_once('/') {
            Some((h, r)) => (h, r),
            None => (path, ""),
        };
        let n: usize = head.strip_prefix(self.volume_kind.prefix())?.parse().ok()?;
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

    /// Byte size of each partition (partition-table order); empty for a floppy.
    pub fn partition_sizes(&self) -> &[u64] {
        &self.partition_sizes
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

/// Label for one disk's synthetic top-level node inside a concatenated image,
/// e.g. `Disk 1` or `Disk 1 — GAMEDISK`.
///
/// Deliberately shorter than a partition's node name: every disk in a
/// concatenated file is the same size, the status bar lists those sizes, and
/// the tree truncates a long name at the size/date columns.
fn disk_node_name(n: usize, volume: &Volume) -> String {
    match volume.volume_label() {
        Some(label) => format!("Disk {n} \u{2014} {label}"),
        None => format!("Disk {n}"),
    }
}

/// Label for one archive member's synthetic top-level node, e.g.
/// `Disc Station Disk_08b.dsk — 737.28 kB, GAMEDISK`.
fn archive_node_name(member: &zip::Member, volume: &Volume) -> String {
    let size = humanize_bytes(member.byte_len() as u64);
    match volume.volume_label() {
        Some(label) => format!("{} \u{2014} {size}, {label}", member.name),
        None => format!("{} \u{2014} {size}", member.name),
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

    /// Three formatted 720 kB FAT12 disks concatenated into one image, the
    /// shape a multi-disk release ships in.
    fn three_disk_image() -> LoadedDisk {
        use msx_disk::image::geometry::DiskFormat;
        let blank = write::create_blank(DiskFormat::Ds720).expect("blank disk");
        let bytes = blank.repeat(3);
        let image = DiskImage::open_bytes(ImageFormat::Dsk, bytes).expect("open");
        LoadedDisk::from_image(image, None).expect("mount")
    }

    /// Two blank disks packed into a stored-method zip, with a readme that must
    /// be ignored.
    fn two_disk_archive() -> zip::Archive {
        use msx_disk::fs::write;
        use msx_disk::image::geometry::DiskFormat;
        let disk = write::create_blank(DiskFormat::Ds720).expect("blank");
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body) in [
            ("A.dsk", disk.as_slice()),
            ("readme.txt", b"hi".as_slice()),
            ("B.dsk", disk.as_slice()),
        ] {
            let local_at = out.len() as u32;
            let len = body.len() as u32;
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&[0x0A, 0x00, 0x00, 0x00, 0x00, 0x00]);
            out.extend_from_slice(&[0u8; 8]); // time, date, crc
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0u8; 2]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(body);

            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&[0x14, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00]);
            central.extend_from_slice(&[0u8; 8]); // time, date, crc
            central.extend_from_slice(&len.to_le_bytes());
            central.extend_from_slice(&len.to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0u8; 12]);
            central.extend_from_slice(&local_at.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_at = out.len() as u32;
        let central_len = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend_from_slice(&central_len.to_le_bytes());
        out.extend_from_slice(&central_at.to_le_bytes());
        out.extend_from_slice(&[0u8; 2]);
        zip::open(&out).expect("archive")
    }

    /// A zip's disk images become one read-only volume each, named after the
    /// member so the tree shows the file the user actually has.
    #[test]
    fn zip_members_become_one_volume_each_named_after_the_member() {
        let disk = LoadedDisk::from_archive(two_disk_archive(), None).expect("mount");
        assert!(disk.is_zip());
        assert!(disk.is_partitioned(), "a zip is a multi-volume document");
        assert!(!disk.writable(), "zipped images are read-only");
        assert!(!disk.is_multi_disk(), "not a concatenated file");

        assert_eq!(disk.tree.len(), 2, "the readme must be skipped");
        assert!(disk.tree[0].name.starts_with("A.dsk"));
        assert!(disk.tree[1].name.starts_with("B.dsk"));
        assert_eq!(
            disk.volume_jumps(),
            vec![("A.dsk".to_string(), 0), ("B.dsk".to_string(), 1440)]
        );
        // One member's shape, not the concatenation's.
        assert_eq!(disk.geometry, Geometry::DS_720K);
        assert_eq!(disk.sector_count(), 2880);
    }

    /// Each member's own checksum is what matches a software database; the
    /// concatenated buffer's would match nothing.
    #[test]
    fn zip_members_carry_their_own_checksums() {
        let archive = two_disk_archive();
        let member = archive.data[..737_280].to_vec();
        let disk = LoadedDisk::from_archive(archive, None).expect("mount");

        let expected = msx_disk::Checksums::of(&member);
        let got = disk.volume_checksums(0).expect("member checksums");
        assert_eq!(got.crc32_hex(), expected.crc32_hex());
        assert_eq!(got.sha1_hex(), expected.sha1_hex());
        // And it is not the whole-document checksum.
        assert_ne!(got.sha1_hex(), disk.checksums().sha1_hex());
        assert!(disk.volume_checksums(2).is_none());
    }

    #[test]
    fn zip_members_can_each_be_pulled_out_whole() {
        use msx_disk::image::geometry::SIZE_720K;
        let disk = LoadedDisk::from_archive(two_disk_archive(), None).expect("mount");
        for n in 0..2 {
            let data = disk.volume_data(n).expect("member bytes");
            assert_eq!(data.len(), SIZE_720K);
            assert_eq!(u16::from_le_bytes([data[11], data[12]]), 512);
        }
        assert_eq!(disk.volume_data(2), None);
    }

    #[test]
    fn concatenated_disks_become_one_volume_each() {
        let disk = three_disk_image();
        assert!(disk.is_multi_disk());
        assert!(disk.is_partitioned(), "multi-disk images are read-only too");
        assert!(!disk.writable());
        assert_eq!(disk.tree.len(), 3);
        assert_eq!(disk.tree[0].name, "Disk 1");
        assert_eq!(disk.tree[2].name, "Disk 3");
        // The reported geometry is one disk's, not the whole file's.
        assert_eq!(disk.geometry, Geometry::DS_720K);
        assert_eq!(disk.sector_count(), 4320);
    }

    /// The tree's disk rows are keyed `D1`..`Dn` with no slash, which is what
    /// the right-click "Extract this disk" menu matches on to find the disk.
    #[test]
    fn disk_nodes_are_keyed_by_position_in_the_file() {
        let disk = three_disk_image();
        let paths: Vec<&str> = disk.tree.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["D1", "D2", "D3"]);
        assert!(paths.iter().all(|p| !p.contains('/')));
        // Matching a node's path gives the index its bytes live at.
        let index = disk.tree.iter().position(|e| e.path == "D3").expect("D3");
        assert_eq!(index, 2);
        assert_eq!(disk.volume_data(index), disk.volume_data(2));
    }

    #[test]
    fn volume_jumps_point_at_each_disk_start() {
        let disk = three_disk_image();
        let jumps = disk.volume_jumps();
        assert_eq!(
            jumps,
            vec![
                ("Disk 1".to_string(), 0),
                ("Disk 2".to_string(), 1440),
                ("Disk 3".to_string(), 2880),
            ]
        );
    }

    /// The Sectors view addresses the whole file, so a sector number has to be
    /// resolvable back to the disk it belongs to.
    #[test]
    fn locate_sector_names_the_disk_and_its_own_offset() {
        let disk = three_disk_image();
        assert_eq!(disk.locate_sector(0), Some((0, 0)));
        assert_eq!(disk.locate_sector(1439), Some((0, 1439)));
        assert_eq!(disk.locate_sector(1440), Some((1, 0)));
        assert_eq!(disk.locate_sector(2881), Some((2, 1)));
        assert_eq!(disk.locate_sector(4319), Some((2, 1439)));
    }

    #[test]
    fn a_single_floppy_has_no_volumes_to_jump_between() {
        let Some(path) = fixture() else {
            eprintln!("skipping: fixture not present");
            return;
        };
        let disk = LoadedDisk::open(&path).expect("open");
        assert!(!disk.is_multi_disk());
        assert!(disk.volume_jumps().is_empty());
        assert_eq!(disk.locate_sector(0), None);
        assert_eq!(disk.volume_data(0), None);
    }

    /// Each exported disk must be a standalone 720 kB image starting at its own
    /// boot sector, not a slice offset into the concatenated file.
    #[test]
    fn volume_data_yields_one_whole_disk_each() {
        use msx_disk::image::geometry::SIZE_720K;
        let disk = three_disk_image();
        for n in 0..3 {
            let data = disk.volume_data(n).expect("disk bytes");
            assert_eq!(data.len(), SIZE_720K);
            // A formatted FAT12 volume starts with a boot sector carrying the
            // 512-byte sector size in its BPB.
            assert_eq!(u16::from_le_bytes([data[11], data[12]]), 512);
        }
        assert_eq!(disk.volume_data(3), None);
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
        assert!(disk.partition_sizes().is_empty());
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
        // Per-partition byte sizes (from the partition table) are exposed for
        // the status bar: one per partition, all non-empty.
        assert_eq!(disk.partition_sizes().len(), 4);
        assert!(disk.partition_sizes().iter().all(|&b| b > 0));
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
