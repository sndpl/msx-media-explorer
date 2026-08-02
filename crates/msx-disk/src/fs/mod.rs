//! FAT12 filesystem layer.
//!
//! Mounts a normalized sector buffer (from [`crate::image::DiskImage`]) as a
//! FAT12 volume using the `fatfs` crate, and exposes a UI-free directory tree
//! plus file reads. MSX-DOS 2 subdirectories are standard FAT directories, so
//! they come through transparently.

mod boot;
mod bootblocks;
pub mod dirscan;
pub mod dos;
pub mod entry;
pub mod map;
pub mod multidisk;
pub mod partition;
pub mod sizefix;
pub mod volume;
pub mod write;

pub use dos::{detect_dos_version, DosVersion};
pub use entry::{Attributes, DirEntry, Timestamp};
pub use map::FatType;
pub use volume::Volume;

use std::io::{Cursor, Read};

use crate::error::{Error, Result};
use crate::image::DiskImage;

/// In-memory backing store for the mounted filesystem.
type Device = Cursor<Vec<u8>>;

/// A mounted MSX disk filesystem.
pub struct DiskFs {
    fs: fatfs::FileSystem<Device>,
    /// Validation of the root directory, done on the raw bytes before mounting
    /// (`fatfs` exposes neither the first cluster nor the reserved attribute
    /// bits, so the checks cannot be made through it).
    scan: dirscan::RootScan,
}

impl DiskFs {
    /// Mount a normalized sector buffer as a FAT12 volume.
    ///
    /// The boot sector is repaired in this private copy (MSX-DOS compatibility);
    /// the caller's original bytes are never modified.
    pub fn mount(mut data: Vec<u8>) -> Result<DiskFs> {
        boot::repair_boot_sector(&mut data)?;
        // Validate against the repaired BPB, which is the geometry `fatfs` will
        // read the directory under.
        let scan = dirscan::scan_root(&data).unwrap_or_default();
        // Use the lossless PUA converter so filename bytes >= 0x80 are preserved
        // (the default converter would replace them with U+FFFD) and the open
        // key still round-trips. See [`crate::charset::PuaOemCpConverter`].
        let opts = fatfs::FsOptions::new().oem_cp_converter(&crate::charset::PUA_OEM_CP_CONVERTER);
        let fs = fatfs::FileSystem::new(Cursor::new(data), opts)
            .map_err(|e| Error::Malformed(format!("not a FAT filesystem: {e}")))?;
        Ok(DiskFs { fs, scan })
    }

    /// Root-directory entries that cannot describe real files and are therefore
    /// left out of [`tree`](Self::tree).
    pub fn invalid_entries(&self) -> &[dirscan::InvalidEntry] {
        &self.scan.invalid
    }

    /// Whether this disk has no usable FAT filesystem: it has directory entries
    /// and every one of them is structurally impossible. Typical of a
    /// custom-format game disk, whose loader reads raw sectors and whose
    /// "directory" area holds code.
    pub fn has_no_filesystem(&self) -> bool {
        self.scan.has_no_filesystem()
    }

    /// Mount the filesystem contained in a disk image.
    pub fn from_image(image: &DiskImage) -> Result<DiskFs> {
        DiskFs::mount(image.data().to_vec())
    }

    /// The disk's volume label, if any.
    ///
    /// Read from the root-directory VOLUME_ID entry (where MSX-DOS stores it),
    /// not the BPB label field which holds boot-code garbage on MSX disks.
    pub fn volume_label(&self) -> Option<String> {
        if self.has_no_filesystem() {
            // The "label" would be whatever the disk's loader code decodes to.
            return None;
        }
        let label = self.fs.read_volume_label_from_root_dir().ok().flatten()?;
        let cleaned: String = label
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        (!cleaned.is_empty()).then_some(cleaned)
    }

    /// Build the full recursive directory tree from the root.
    ///
    /// Root entries rejected by [`dirscan`] are left out: they are provably not
    /// files, and listing them presents disk-loader code as content. See
    /// [`invalid_entries`](Self::invalid_entries) to report how many there were.
    pub fn tree(&self) -> Result<Vec<DirEntry>> {
        read_dir_recursive(&self.fs.root_dir(), "", &self.scan)
    }

    /// Read the entire contents of a file by its slash-separated path.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let mut file = self
            .fs
            .root_dir()
            .open_file(path)
            .map_err(|e| Error::Unsupported(format!("cannot open '{path}': {e}")))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

/// Recursively collect entries under `dir`, prefixing paths with `prefix`.
///
/// `scan` validates the *root* directory only; subdirectories are reached
/// through a root entry that has already been vetted, and a disk whose root is
/// not a directory has no reachable subdirectories to begin with.
fn read_dir_recursive(
    dir: &fatfs::Dir<'_, Device>,
    prefix: &str,
    scan: &dirscan::RootScan,
) -> Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    for result in dir.iter() {
        let raw = result.map_err(|e| Error::Malformed(format!("directory read failed: {e}")))?;
        let name = raw.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let attrs = raw.attributes();
        if attrs.contains(fatfs::FileAttributes::VOLUME_ID) {
            continue; // the volume label is exposed separately
        }
        if prefix.is_empty() && scan.rejects(raw.short_file_name_as_bytes()) {
            continue; // not a file: an impossible size, cluster or attribute
        }

        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let is_dir = raw.is_dir();
        let children = if is_dir {
            read_dir_recursive(&raw.to_dir(), &path, scan)?
        } else {
            Vec::new()
        };

        entries.push(DirEntry {
            name,
            path,
            is_dir,
            size: raw.len(),
            attributes: Attributes {
                read_only: attrs.contains(fatfs::FileAttributes::READ_ONLY),
                hidden: attrs.contains(fatfs::FileAttributes::HIDDEN),
                system: attrs.contains(fatfs::FileAttributes::SYSTEM),
                archive: attrs.contains(fatfs::FileAttributes::ARCHIVE),
            },
            modified: timestamp_from(raw.modified()),
            children,
        });
    }
    Ok(entries)
}

/// Convert a fatfs `DateTime` into our minute-resolution [`Timestamp`].
///
/// fatfs reports the DOS epoch (1980) for entries without a real date; treat
/// those as "no timestamp" to avoid showing a meaningless 1980 everywhere.
fn timestamp_from(dt: fatfs::DateTime) -> Option<Timestamp> {
    if dt.date.year <= 1980 {
        return None;
    }
    Some(Timestamp {
        year: dt.date.year,
        month: dt.date.month as u8,
        day: dt.date.day as u8,
        hour: dt.time.hour as u8,
        minute: dt.time.min as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::geometry::SIZE_720K;

    /// Format a blank 720KB FAT12 image and populate it with a couple of files
    /// and a subdirectory, returning the raw sector bytes.
    fn make_disk() -> Vec<u8> {
        let mut cursor = Cursor::new(vec![0u8; SIZE_720K]);
        fatfs::format_volume(
            &mut cursor,
            fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat12),
        )
        .expect("format");

        {
            let fs = fatfs::FileSystem::new(&mut cursor, fatfs::FsOptions::new()).expect("mount");
            let root = fs.root_dir();
            {
                use std::io::Write;
                let mut f = root.create_file("HELLO.TXT").expect("create");
                f.write_all(b"hi there").expect("write");
            }
            let sub = root.create_dir("UTILS").expect("mkdir");
            {
                use std::io::Write;
                let mut f = sub.create_file("GAME.COM").expect("create sub file");
                f.write_all(&[0xC9; 16]).expect("write sub");
            }
        }
        cursor.into_inner()
    }

    #[test]
    fn lists_files_and_subdirectories() {
        let fs = DiskFs::mount(make_disk()).expect("mount");
        let tree = fs.tree().expect("tree");

        let names: Vec<&str> = tree.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"HELLO.TXT"));
        assert!(names.contains(&"UTILS"));

        let utils = tree.iter().find(|e| e.name == "UTILS").unwrap();
        assert!(utils.is_dir);
        assert_eq!(utils.children.len(), 1);
        assert_eq!(utils.children[0].name, "GAME.COM");
        assert_eq!(utils.children[0].path, "UTILS/GAME.COM");
    }

    #[test]
    fn reads_file_contents_including_subdir() {
        let fs = DiskFs::mount(make_disk()).expect("mount");
        assert_eq!(fs.read_file("HELLO.TXT").unwrap(), b"hi there");
        assert_eq!(fs.read_file("UTILS/GAME.COM").unwrap(), vec![0xC9; 16]);
    }

    #[test]
    fn walk_visits_every_entry() {
        let fs = DiskFs::mount(make_disk()).expect("mount");
        let tree = fs.tree().expect("tree");
        let total: usize = tree.iter().map(|e| e.walk().count()).sum();
        // HELLO.TXT + UTILS + UTILS/GAME.COM = 3
        assert_eq!(total, 3);
    }

    /// Format a 720KB image and create one file whose 8.3 name contains bytes
    /// `>= 0x80` (katakana ｱｲｳ = bytes 0xB1 0xB2 0xB3), written through the
    /// lossless PUA converter so the high bytes survive on disk.
    fn make_kana_disk() -> Vec<u8> {
        let mut cursor = Cursor::new(vec![0u8; SIZE_720K]);
        fatfs::format_volume(
            &mut cursor,
            fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat12),
        )
        .expect("format");
        {
            let opts =
                fatfs::FsOptions::new().oem_cp_converter(&crate::charset::PUA_OEM_CP_CONVERTER);
            let fs = fatfs::FileSystem::new(&mut cursor, opts).expect("mount");
            let root = fs.root_dir();
            use std::io::Write;
            let mut f = root
                .create_file("\u{F0B1}\u{F0B2}\u{F0B3}.BAS")
                .expect("create");
            f.write_all(b"10 END").expect("write");
        }
        cursor.into_inner()
    }

    #[test]
    fn high_byte_filename_survives_and_decodes_to_kana() {
        use crate::charset::MsxCharset;
        let fs = DiskFs::mount(make_kana_disk()).expect("mount");
        let tree = fs.tree().expect("tree");
        let entry = tree.iter().find(|e| !e.is_dir).expect("the kana file");

        // The raw high bytes survive (as PUA) instead of being lost to U+FFFD.
        assert_eq!(entry.name, "\u{F0B1}\u{F0B2}\u{F0B3}.BAS");
        assert!(!entry.name.contains('\u{FFFD}'));
        // Decoded for display under Japanese: real half-width katakana.
        assert_eq!(
            entry.display_name(MsxCharset::Japanese),
            "\u{FF71}\u{FF72}\u{FF73}.BAS"
        );
        // The access path still opens the file despite the high bytes.
        assert_eq!(fs.read_file(&entry.path).expect("read"), b"10 END");
    }

    #[test]
    fn rejects_undersized_image() {
        assert!(DiskFs::mount(vec![0u8; 256]).is_err());
    }

    #[test]
    fn blank_standard_size_mounts_as_empty() {
        // After boot-sector repair a zeroed 720KB image is a valid empty disk.
        let fs = DiskFs::mount(vec![0u8; SIZE_720K]).expect("mount blank");
        assert!(fs.tree().expect("tree").is_empty());
    }
}
