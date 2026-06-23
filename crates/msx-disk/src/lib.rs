//! `msx-disk` — a headless library for reading, decoding, and converting
//! MSX floppy disk image formats.
//!
//! The crate is deliberately UI-free: every public function returns plain data
//! structures (sector buffers, directory trees, decoded view models) so the
//! logic can be tested without a GUI and reused from a CLI.

pub mod cas;
pub mod error;
pub mod fs;
pub mod image;
pub mod search;
pub mod view;

pub use error::{Error, Result};
pub use fs::{DirEntry, DiskFs};
pub use image::{DiskImage, ImageFormat};
