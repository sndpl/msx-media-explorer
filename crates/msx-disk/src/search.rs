//! Byte/text/hex pattern search over disk sectors and file contents.
//!
//! All functions return the start offsets of every match (overlapping matches
//! are reported). The GUI uses these to search a whole disk image's sector data
//! or an individual file's bytes.

/// Find every start offset where `needle` occurs in `haystack`.
pub fn find_bytes(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    (0..=haystack.len() - needle.len())
        .filter(|&i| &haystack[i..i + needle.len()] == needle)
        .collect()
}

/// Find every start offset where `text` occurs, optionally ignoring ASCII case.
pub fn find_text(haystack: &[u8], text: &str, case_insensitive: bool) -> Vec<usize> {
    let needle = text.as_bytes();
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    if !case_insensitive {
        return find_bytes(haystack, needle);
    }
    (0..=haystack.len() - needle.len())
        .filter(|&i| haystack[i..i + needle.len()].eq_ignore_ascii_case(needle))
        .collect()
}

/// Parse a hex search pattern such as `"DE AD BE EF"` or `"deadbeef"` into
/// bytes. Whitespace is ignored; returns `None` if the input is empty or not a
/// whole number of hex byte pairs.
pub fn parse_hex(input: &str) -> Option<Vec<u8>> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() || !cleaned.len().is_multiple_of(2) {
        return None;
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).ok())
        .collect()
}

/// Split a flat disk offset into a (sector index, offset-within-sector) pair.
pub fn locate(offset: usize) -> (usize, usize) {
    (
        offset / crate::image::geometry::SECTOR_SIZE,
        offset % crate::image::geometry::SECTOR_SIZE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_all_including_overlaps() {
        assert_eq!(find_bytes(b"aaaa", b"aa"), vec![0, 1, 2]);
        assert_eq!(find_bytes(b"abcabc", b"bc"), vec![1, 4]);
        assert_eq!(find_bytes(b"abc", b"x"), Vec::<usize>::new());
    }

    #[test]
    fn empty_or_oversized_needle_finds_nothing() {
        assert!(find_bytes(b"abc", b"").is_empty());
        assert!(find_bytes(b"ab", b"abc").is_empty());
    }

    #[test]
    fn text_search_case_sensitivity() {
        assert_eq!(find_text(b"Hello hello", "hello", false), vec![6]);
        assert_eq!(find_text(b"Hello hello", "hello", true), vec![0, 6]);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("DE AD BE EF"), Some(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(parse_hex("deadbeef"), Some(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(parse_hex(""), None);
        assert_eq!(parse_hex("ABC"), None); // odd length
        assert_eq!(parse_hex("ZZ"), None); // not hex
    }

    #[test]
    fn locate_splits_offset_into_sector() {
        assert_eq!(locate(0), (0, 0));
        assert_eq!(locate(513), (1, 1));
    }
}
