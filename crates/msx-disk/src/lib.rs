//! `msx-disk` — a headless library for reading, decoding, and converting
//! MSX floppy disk image formats.
//!
//! The crate is deliberately UI-free: every public function returns plain data
//! structures (sector buffers, directory trees, decoded view models) so the
//! logic can be tested without a GUI and reused from a CLI.

pub mod archive;
pub mod cas;
pub mod charset;
pub mod error;
pub mod fileinfo;
pub mod fs;
pub mod hostname;
pub mod image;
pub mod recoil;
pub mod search;
pub mod stats;
pub mod tape;
pub mod tsx;
pub mod verify;
pub mod view;

pub use archive::{ArchiveEntry, Method};
pub use charset::MsxCharset;
pub use error::{Error, Result};
pub use fs::{DirEntry, DiskFs};
pub use image::{DiskImage, ImageFormat};
pub use stats::{DiskStats, FatIntegrity};
pub use verify::Checksums;
