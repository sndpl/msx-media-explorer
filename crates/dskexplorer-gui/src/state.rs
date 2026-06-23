//! GUI-side state for a currently-open disk.

use std::path::{Path, PathBuf};

use msx_disk::fs::write;
use msx_disk::image::geometry::Geometry;
use msx_disk::{DirEntry, DiskFs, DiskImage, Error, ImageFormat};

/// Everything the UI needs about the open disk, computed once on load.
pub struct LoadedDisk {
    pub path: Option<PathBuf>,
    pub format: ImageFormat,
    pub geometry: Geometry,
    pub label: Option<String>,
    pub tree: Vec<DirEntry>,
    image: DiskImage,
    fs: DiskFs,
}

impl LoadedDisk {
    /// Open and mount a disk image from a filesystem path.
    pub fn open(path: &Path) -> msx_disk::Result<LoadedDisk> {
        let image = DiskImage::open(path)?;
        LoadedDisk::from_image(image, Some(path.to_path_buf()))
    }

    /// Mount an already-decoded image, remembering its source path.
    pub fn from_image(image: DiskImage, path: Option<PathBuf>) -> msx_disk::Result<LoadedDisk> {
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
            fs,
        })
    }

    /// Read a file's bytes by its slash-separated path.
    pub fn read_file(&self, path: &str) -> msx_disk::Result<Vec<u8>> {
        self.fs.read_file(path)
    }

    /// The disk's normalized sector data — a ready-to-write `.dsk` image.
    pub fn to_dsk_bytes(&self) -> Vec<u8> {
        self.image.data().to_vec()
    }

    /// Compress the disk's sectors into an `.xsa` image.
    pub fn to_xsa_bytes(&self) -> Vec<u8> {
        msx_disk::image::xsa::compress(self.image.data())
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

    /// Whether this disk can be modified in place (has a path and a writable
    /// container format).
    pub fn writable(&self) -> bool {
        self.path.is_some() && self.image.is_writable()
    }

    /// Add (or overwrite) files at the root of the disk.
    pub fn add_files(&mut self, files: &[(String, Vec<u8>)]) -> msx_disk::Result<()> {
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let updated = write::add_files(self.image.data(), &refs)?;
        self.write_back(updated)
    }

    /// Delete files by their slash-separated paths.
    pub fn delete(&mut self, paths: &[String]) -> msx_disk::Result<()> {
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let updated = write::delete(self.image.data(), &refs)?;
        self.write_back(updated)
    }

    /// Rename a file from `old_path` to `new_path` (both relative to the root).
    pub fn rename(&mut self, old_path: &str, new_path: &str) -> msx_disk::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/MSX-DOS2 TOOLS.dsk");
        path.exists().then_some(path)
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
        let tmp = std::env::temp_dir().join("dskexplorer_phase2_test.dsk");
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

        let _ = std::fs::remove_file(&tmp);
    }
}
