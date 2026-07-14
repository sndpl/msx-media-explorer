//! Pure glob filtering for the file tree.
//!
//! A tiny filename matcher for the explorer's filter box, plus the "keep-set"
//! that turns a pattern into the set of tree paths to show. No egui here, so
//! every case is unit-testable; the egui glue in `app/` only threads the
//! keep-set into the renderers and the keyboard-nav flattener.

use std::collections::BTreeSet;

use msx_disk::{DirEntry, MsxCharset};

/// Whether `name` matches the glob `pattern`, anchored (the whole name must
/// match) and case-insensitive (ASCII):
///
/// - `*` matches any run of characters, including none.
/// - `%` and `?` each match exactly one character (aliases; `%` is MSX-DOS, `?`
///   is standard glob).
/// - every other character matches itself.
///
/// Matching iterates over `char`s, so a single-character wildcard consumes one
/// Unicode scalar (e.g. one kana glyph), not one byte.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = name.chars().collect();
    let (mut pi, mut si) = (0usize, 0usize);
    // Last `*` seen and the input position it was allowed to start swallowing
    // from, so a failed match can backtrack and let the `*` consume one more.
    let mut star: Option<usize> = None;
    let mut star_si = 0usize;
    while si < s.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some(pi);
                star_si = si;
                pi += 1;
            }
            Some('%') | Some('?') => {
                pi += 1;
                si += 1;
            }
            Some(&c) if c.eq_ignore_ascii_case(&s[si]) => {
                pi += 1;
                si += 1;
            }
            _ => match star {
                Some(sp) => {
                    pi = sp + 1;
                    star_si += 1;
                    si = star_si;
                }
                None => return false,
            },
        }
    }
    // Any trailing pattern must be all `*` to match the now-exhausted name.
    p[pi..].iter().all(|&c| c == '*')
}

/// The set of tree paths to show for `pattern`: every file whose display name
/// matches, plus all of its ancestor directory paths (so the folder chain to a
/// match stays visible). Directories are never matched themselves — only kept as
/// ancestors. An empty result means nothing matched.
pub fn matching_paths(
    entries: &[DirEntry],
    pattern: &str,
    charset: MsxCharset,
) -> BTreeSet<String> {
    let mut keep = BTreeSet::new();
    for root in entries {
        for entry in root.walk() {
            if entry.is_dir || !glob_match(pattern, &entry.display_name(charset)) {
                continue;
            }
            // Insert the file path and every ancestor prefix, e.g.
            // "UTILS/SUB/X.BIN" -> {"UTILS", "UTILS/SUB", "UTILS/SUB/X.BIN"}.
            let mut prefix = String::new();
            for part in entry.path.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                keep.insert(prefix.clone());
            }
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;
    use msx_disk::fs::Attributes;

    #[test]
    fn star_matches_any_run() {
        assert!(glob_match("*.PIC", "FOO.PIC"));
        assert!(glob_match("*.PIC", ".PIC")); // `*` may match nothing
        assert!(!glob_match("*.PIC", "FOO.SC5"));
        assert!(glob_match("num*", "NUMBERS.TXT"));
        assert!(!glob_match("num*", "FOO.TXT"));
    }

    #[test]
    fn single_char_wildcards_are_aliases() {
        for w in ["img%.sc5", "img?.sc5"] {
            assert!(glob_match(w, "IMG1.SC5"), "{w} should match IMG1.SC5");
            assert!(glob_match(w, "IMG9.SC5"), "{w} should match IMG9.SC5");
            // `%`/`?` is exactly one char: neither zero nor two.
            assert!(!glob_match(w, "IMG.SC5"), "{w} must not match IMG.SC5");
            assert!(!glob_match(w, "IMG10.SC5"), "{w} must not match IMG10.SC5");
        }
    }

    #[test]
    fn matching_is_case_insensitive_and_dot_is_literal() {
        assert!(glob_match("*.pic", "PHOTO.PIC"));
        assert!(glob_match("Num*", "numbers.txt"));
        // `.` in the pattern is a literal dot, not a wildcard.
        assert!(!glob_match("a.c", "abc"));
        assert!(glob_match("a.c", "A.C"));
    }

    #[test]
    fn empty_and_exact_patterns() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "X"));
        assert!(glob_match("readme.txt", "README.TXT"));
        assert!(!glob_match("readme.txt", "README.TX"));
    }

    fn file(path: &str) -> DirEntry {
        DirEntry {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            is_dir: false,
            size: 0,
            attributes: Attributes::default(),
            modified: None,
            children: Vec::new(),
        }
    }

    fn dir(path: &str, children: Vec<DirEntry>) -> DirEntry {
        DirEntry {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            is_dir: true,
            size: 0,
            attributes: Attributes::default(),
            modified: None,
            children,
        }
    }

    /// GAMES/ { A.PIC, B.SC5 }, UTILS/ { SUB/ { C.PIC } }, HELLO.BAS
    fn sample() -> Vec<DirEntry> {
        vec![
            dir("GAMES", vec![file("GAMES/A.PIC"), file("GAMES/B.SC5")]),
            dir(
                "UTILS",
                vec![dir("UTILS/SUB", vec![file("UTILS/SUB/C.PIC")])],
            ),
            file("HELLO.BAS"),
        ]
    }

    #[test]
    fn matching_paths_keeps_matches_and_their_ancestors() {
        let keep = matching_paths(&sample(), "*.PIC", MsxCharset::International);
        // The two .PIC files plus every ancestor directory on their paths.
        let mut got: Vec<&str> = keep.iter().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(
            got,
            vec![
                "GAMES",
                "GAMES/A.PIC",
                "UTILS",
                "UTILS/SUB",
                "UTILS/SUB/C.PIC",
            ]
        );
        // Non-matching files and directories with no match inside are excluded.
        assert!(!keep.contains("GAMES/B.SC5"));
        assert!(!keep.contains("HELLO.BAS"));
    }

    #[test]
    fn matching_paths_is_empty_when_nothing_matches() {
        assert!(matching_paths(&sample(), "*.XYZ", MsxCharset::International).is_empty());
    }
}
