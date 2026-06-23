//! GUI-side state for a currently-open disk.

use std::path::{Path, PathBuf};

use msx_disk::image::geometry::Geometry;
use msx_disk::{DirEntry, DiskFs, DiskImage, ImageFormat};

/// Everything the UI needs about the open disk, computed once on load.
pub struct LoadedDisk {
    pub path: Option<PathBuf>,
    pub format: ImageFormat,
    pub geometry: Geometry,
    pub label: Option<String>,
    pub tree: Vec<DirEntry>,
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
            fs,
        })
    }

    /// Read a file's bytes by its slash-separated path.
    pub fn read_file(&self, path: &str) -> msx_disk::Result<Vec<u8>> {
        self.fs.read_file(path)
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
    use std::path::PathBuf;

    #[test]
    fn loads_fixture_disk_when_present() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/MSX-DOS2 TOOLS.dsk");
        if !path.exists() {
            eprintln!("skipping: fixture not present");
            return;
        }
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
}
