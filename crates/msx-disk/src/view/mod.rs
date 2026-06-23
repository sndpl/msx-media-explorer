//! File-content view models.
//!
//! Each submodule turns a file's raw bytes into a UI-free representation the
//! GUI can render directly: a hex dump, decoded text, (later) detokenized BASIC
//! and decoded MSX screen bitmaps.

pub mod hex;
pub mod text;
