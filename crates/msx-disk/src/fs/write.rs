//! Write-back operations: add, delete, and rename files on a disk image.
//!
//! Each operation is a self-contained transaction over the *normalized* sector
//! buffer. The boot sector is repaired only for mounting; the original sector 0
//! is restored before returning, so MSX-DOS compatibility shims (and the boot
//! program code they overwrite) are never persisted to the user's disk.

use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::image::geometry::SECTOR_SIZE;

/// A seekable in-memory device backed by a shared buffer, so the modified bytes
/// can be reclaimed after `fatfs` releases the filesystem.
struct SharedDisk {
    buf: Rc<RefCell<Vec<u8>>>,
    pos: usize,
}

impl Read for SharedDisk {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let buf = self.buf.borrow();
        if self.pos >= buf.len() {
            return Ok(0);
        }
        let n = (buf.len() - self.pos).min(out.len());
        out[..n].copy_from_slice(&buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Write for SharedDisk {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut buf = self.buf.borrow_mut();
        let end = self.pos + data.len();
        if end > buf.len() {
            buf.resize(end, 0);
        }
        buf[self.pos..end].copy_from_slice(data);
        self.pos = end;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for SharedDisk {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let len = self.buf.borrow().len() as i64;
        let target = match pos {
            SeekFrom::Start(o) => o as i64,
            SeekFrom::End(o) => len + o,
            SeekFrom::Current(o) => self.pos as i64 + o,
        };
        if target < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "negative seek"));
        }
        self.pos = target as usize;
        Ok(self.pos as u64)
    }
}

/// Run a filesystem mutation against `normalized` and return the new normalized
/// data with the original sector 0 preserved.
fn transaction<F>(normalized: &[u8], op: F) -> Result<Vec<u8>>
where
    F: FnOnce(&fatfs::Dir<'_, SharedDisk>) -> io::Result<()>,
{
    if normalized.len() < SECTOR_SIZE {
        return Err(Error::Malformed("image smaller than one sector".into()));
    }
    let original_sector0 = normalized[..SECTOR_SIZE].to_vec();

    let mut data = normalized.to_vec();
    super::boot::repair_boot_sector(&mut data)?;

    let buf = Rc::new(RefCell::new(data));
    {
        let device = SharedDisk {
            buf: Rc::clone(&buf),
            pos: 0,
        };
        let fs = fatfs::FileSystem::new(device, fatfs::FsOptions::new())
            .map_err(|e| Error::Malformed(format!("mount for write failed: {e}")))?;
        op(&fs.root_dir()).map_err(map_fs_err)?;
        fs.unmount()
            .map_err(|e| Error::Malformed(format!("unmount failed: {e}")))?;
    }

    let mut out = buf.borrow().clone();
    out[..SECTOR_SIZE].copy_from_slice(&original_sector0);
    Ok(out)
}

fn map_fs_err(e: io::Error) -> Error {
    Error::Unsupported(format!("filesystem operation failed: {e}"))
}

/// Add (or overwrite) files at the root of the disk. Each tuple is `(name, bytes)`.
pub fn add_files(normalized: &[u8], files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    transaction(normalized, |root| {
        for (name, bytes) in files {
            let mut file = root.create_file(name)?;
            file.truncate()?;
            file.write_all(bytes)?;
            file.flush()?;
        }
        Ok(())
    })
}

/// Delete files (or empty directories) by their slash-separated paths.
pub fn delete(normalized: &[u8], paths: &[&str]) -> Result<Vec<u8>> {
    transaction(normalized, |root| {
        for path in paths {
            root.remove(path)?;
        }
        Ok(())
    })
}

/// Rename/move a file or directory from `old_path` to `new_path`.
pub fn rename(normalized: &[u8], old_path: &str, new_path: &str) -> Result<Vec<u8>> {
    transaction(normalized, |root| root.rename(old_path, root, new_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::DiskFs;
    use crate::image::geometry::SIZE_720K;
    use std::io::Cursor;

    /// A blank, freshly-formatted 720KB FAT12 disk.
    fn blank_disk() -> Vec<u8> {
        let mut cursor = Cursor::new(vec![0u8; SIZE_720K]);
        fatfs::format_volume(
            &mut cursor,
            fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat12),
        )
        .expect("format");
        cursor.into_inner()
    }

    #[test]
    fn add_then_read_roundtrip() {
        let disk = blank_disk();
        let out = add_files(&disk, &[("HELLO.TXT", b"hi there")]).unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("HELLO.TXT").unwrap(), b"hi there");
    }

    #[test]
    fn delete_removes_file() {
        let disk = blank_disk();
        let two = add_files(&disk, &[("A.TXT", b"a"), ("B.TXT", b"b")]).unwrap();
        let out = delete(&two, &["A.TXT"]).unwrap();
        let names: Vec<String> = DiskFs::mount(out)
            .unwrap()
            .tree()
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(!names.contains(&"A.TXT".to_string()));
        assert!(names.contains(&"B.TXT".to_string()));
    }

    #[test]
    fn rename_changes_name_keeps_contents() {
        let disk = blank_disk();
        let with = add_files(&disk, &[("OLD.TXT", b"data")]).unwrap();
        let out = rename(&with, "OLD.TXT", "NEW.TXT").unwrap();
        let fs = DiskFs::mount(out).unwrap();
        assert_eq!(fs.read_file("NEW.TXT").unwrap(), b"data");
        assert!(fs.read_file("OLD.TXT").is_err());
    }

    #[test]
    fn original_boot_code_is_preserved() {
        // A byte in the boot-code area (outside the BPB) must survive a write,
        // proving the boot-sector repair is never persisted.
        let mut disk = blank_disk();
        disk[0x80] = 0xAB;
        let out = add_files(&disk, &[("X.TXT", b"x")]).unwrap();
        assert_eq!(out[0x80], 0xAB);
        assert_eq!(&out[..SECTOR_SIZE], &disk[..SECTOR_SIZE]);
    }
}
