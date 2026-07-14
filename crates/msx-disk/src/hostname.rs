//! Turn MSX filenames into safe, unique host-filesystem names for extraction.
//!
//! MSX directory entries can hold names the host filesystem rejects or mangles:
//! C0 control bytes (crafted "fake" entries are built from them), NUL bytes
//! (a null extension), and characters Windows reserves. Writing such a name
//! verbatim either creates an unusable file or aborts the whole extraction
//! (Rust rejects a path containing a NUL). These helpers make a name safe and,
//! at write time, unique.

use std::path::{Path, PathBuf};

use crate::charset;

/// Characters reserved on common host filesystems (Windows is the strict one);
/// `/` and `\` are path separators, so they must never appear in a component.
const RESERVED: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Make `name` safe to use as one host path component: control bytes become
/// their Unicode Control Picture (matching the in-app display, and dropping NUL
/// bytes that would otherwise abort extraction), and characters reserved on
/// common host filesystems become `_`. Ordinary MSX names are unchanged.
pub fn safe_component(name: &str) -> String {
    charset::display_control_safe(name)
        .chars()
        .map(|c| if RESERVED.contains(&c) { '_' } else { c })
        .collect()
}

/// `target` if nothing exists there, otherwise the same path with " (2)",
/// " (3)", ... inserted before the extension until a free name is found — so two
/// byte-identical directory entries (or an already-present file) do not silently
/// overwrite each other.
pub fn free_target(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let parent = target.parent();
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = target.extension().map(|e| e.to_string_lossy().into_owned());
    let mut n = 2u32;
    loop {
        let name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = match parent {
            Some(p) if !p.as_os_str().is_empty() => p.join(&name),
            _ => PathBuf::from(&name),
        };
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_component_maps_controls_and_reserved() {
        // Ordinary names are untouched.
        assert_eq!(safe_component("GAME.COM"), "GAME.COM");
        // Reserved characters -> underscore.
        assert_eq!(safe_component("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(safe_component("a/b\\c"), "a_b_c");
        // Control bytes (incl. NUL) -> control pictures, never a raw NUL.
        assert_eq!(safe_component("\u{00}"), "\u{2400}");
        assert!(!safe_component("P.Vaesen\u{00}\u{00}").contains('\u{00}'));
        // The crafted jaarg-hw.di1 name: dots kept, controls become pictures.
        assert_eq!(
            safe_component("\u{07}\r\r\r\r\n\u{0C}\u{0C}.\u{1A}\u{0C}\u{0C}"),
            "\u{2407}\u{240D}\u{240D}\u{240D}\u{240D}\u{240A}\u{240C}\u{240C}.\u{241A}\u{240C}\u{240C}"
        );
    }

    #[test]
    fn free_target_suffixes_on_collision() {
        let dir = std::env::temp_dir().join(format!("msxhost-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Free name is returned unchanged.
        let a = dir.join("PIC.SC5");
        assert_eq!(free_target(&a), a);

        // Once it exists, the next call suffixes before the extension.
        std::fs::write(&a, b"1").unwrap();
        let b = free_target(&a);
        assert_eq!(b, dir.join("PIC (2).SC5"));

        // And it keeps counting past taken suffixes.
        std::fs::write(&b, b"2").unwrap();
        assert_eq!(free_target(&a), dir.join("PIC (3).SC5"));

        // A name without an extension suffixes at the end.
        let c = dir.join("README");
        std::fs::write(&c, b"x").unwrap();
        assert_eq!(free_target(&c), dir.join("README (2)"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
