//! Content-derived facts about a file: a description, a BSAVE header, and any
//! graphics- or music-format specifics.
//!
//! This module is UI-free. It works from a file's name (for the extension) and
//! its raw bytes; filesystem metadata (attributes, timestamp) is layered on by
//! the caller. The per-format byte offsets are documented in each submodule.

pub mod bload;
pub mod extensions;
pub mod graphics;
pub mod music;

pub use bload::BloadHeader;
pub use graphics::GraphicsInfo;
pub use music::MusicInfo;

/// Everything we can say about a file from its name and bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInfo {
    /// A short description of the file type, from the extension table.
    pub description: Option<&'static str>,
    /// The BSAVE/BLOAD header, when the file starts with one.
    pub bload: Option<BloadHeader>,
    /// Graphics-format details, when the extension is a known image format.
    pub graphics: Option<GraphicsInfo>,
    /// Music-format details, when the extension is a known music format.
    pub music: Option<MusicInfo>,
}

/// The lowercase extension of a slash/backslash path, without the dot.
pub fn extension(name: &str) -> Option<String> {
    name.rsplit(['/', '\\'])
        .next()
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
}

/// Describe a file from its name and raw bytes.
pub fn describe(name: &str, bytes: &[u8]) -> FileInfo {
    let ext = extension(name).unwrap_or_default();
    FileInfo {
        description: extensions::describe_ext(&ext),
        bload: bload::parse(bytes),
        graphics: graphics::describe(name),
        music: music::describe(&ext, bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_lowercased_and_path_aware() {
        assert_eq!(extension("DIR/FILE.SC7").as_deref(), Some("sc7"));
        assert_eq!(extension("noext"), None);
    }

    #[test]
    fn describe_a_bsave_screen() {
        // 0xFE BLOAD header for a SCREEN 7 image.
        let bytes = [0xFE, 0x00, 0x00, 0xFF, 0xD3, 0x00, 0x00, 0x00];
        let info = describe("PIC.SC7", &bytes);
        assert!(info.bload.is_some());
        assert_eq!(info.graphics.unwrap().label, "SCREEN 7");
        assert!(info.description.unwrap().contains("SCREEN 7"));
        assert!(info.music.is_none());
    }

    #[test]
    fn describe_a_music_file() {
        let mut buf = vec![0u8; 1084];
        buf[..4].copy_from_slice(b"NAME");
        buf[1080..1084].copy_from_slice(b"M.K.");
        let info = describe("TUNE.MOD", &buf);
        let music = info.music.expect("music");
        assert_eq!(music.format, "Amiga module");
        assert_eq!(music.channels, Some(4));
        assert_eq!(info.description, Some("Amiga module"));
    }
}
