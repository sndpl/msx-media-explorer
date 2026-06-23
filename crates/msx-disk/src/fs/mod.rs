//! FAT12 filesystem layer.
//!
//! Mounts a normalized sector buffer (from [`crate::image::DiskImage`]) as a
//! FAT12 volume using the `fatfs` crate, and exposes a UI-free directory tree
//! plus file reads. MSX-DOS 2 subdirectories are standard FAT directories, so
//! they come through transparently.

mod boot;
pub mod dos;
pub mod entry;
pub mod map;
pub mod write;

pub use dos::{detect_dos_version, DosVersion};
pub use entry::{Attributes, DirEntry, Timestamp};

use std::io::{Cursor, Read};

use crate::error::{Error, Result};
use crate::image::DiskImage;

/// In-memory backing store for the mounted filesystem.
type Device = Cursor<Vec<u8>>;

/// A mounted MSX disk filesystem.
pub struct DiskFs {
    fs: fatfs::FileSystem<Device>,
}

impl DiskFs {
    /// Mount a normalized sector buffer as a FAT12 volume.
    ///
    /// The boot sector is repaired in this private copy (MSX-DOS compatibility);
    /// the caller's original bytes are never modified.
    pub fn mount(mut data: Vec<u8>) -> Result<DiskFs> {
        boot::repair_boot_sector(&mut data)?;
        let fs = fatfs::FileSystem::new(Cursor::new(data), fatfs::FsOptions::new())
            .map_err(|e| Error::Malformed(format!("not a FAT filesystem: {e}")))?;
        Ok(DiskFs { fs })
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
    pub fn tree(&self) -> Result<Vec<DirEntry>> {
        read_dir_recursive(&self.fs.root_dir(), "")
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
fn read_dir_recursive(dir: &fatfs::Dir<'_, Device>, prefix: &str) -> Result<Vec<DirEntry>> {
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

        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let is_dir = raw.is_dir();
        let children = if is_dir {
            read_dir_recursive(&raw.to_dir(), &path)?
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
