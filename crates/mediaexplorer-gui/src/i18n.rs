//! UI language selection.
//!
//! The translated strings live in `locales/*.yml` (embedded at compile time by
//! the `rust_i18n::i18n!` macro in `main.rs`); this module is just the small
//! enum of supported languages plus the mapping to/from locale codes. Adding a
//! language is: add a `locales/<code>.yml`, add a variant here, and translate.
//!
//! This is the UI language, entirely separate from [`msx_disk::MsxCharset`],
//! which decodes MSX *file content* and is chosen in the Text Encoding menu.

use serde::{Deserialize, Serialize};

/// Pluralized translation. rust-i18n 3.x treats `count` as a plain interpolation
/// variable and does not select a plural form, so we do: `<base>.one` for a
/// count of exactly 1, else `<base>.other` (the English/Dutch rule). `count` is
/// interpolated automatically; pass any other placeholders as `name => value`.
///
/// ```ignore
/// tn!("status.matches", n);                       // "3 matches"
/// tn!("status.deleted", paths.len(), name => f);  // "Deleted FOO.TXT"
/// ```
macro_rules! tn {
    ($base:literal, $count:expr $(, $name:ident => $value:expr)* $(,)?) => {{
        let n = $count;
        if n == 1 {
            rust_i18n::t!(concat!($base, ".one"), count => n $(, $name => $value)*)
        } else {
            rust_i18n::t!(concat!($base, ".other"), count => n $(, $name => $value)*)
        }
    }};
}
pub(crate) use tn;

/// A supported user-interface language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    English,
    Dutch,
}

impl Lang {
    /// Every language, in menu order (English, the source language, first).
    pub const ALL: &'static [Lang] = &[Lang::English, Lang::Dutch];

    /// The `rust-i18n` locale code, matching the `locales/<code>.yml` file name.
    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Dutch => "nl",
        }
    }

    /// The language's own name, for the selector (shown the same in every UI
    /// language, as is conventional).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::Dutch => "Nederlands",
        }
    }

    /// Map a BCP-47 locale tag (e.g. `"nl-NL"`, `"en_US"`) to a supported
    /// language by its primary subtag, or `None` if unsupported. Used once at
    /// first run to follow the OS locale.
    pub fn from_locale_tag(tag: &str) -> Option<Lang> {
        let primary = tag
            .split(['-', '_'])
            .next()
            .unwrap_or(tag)
            .to_ascii_lowercase();
        Lang::ALL.iter().copied().find(|l| l.code() == primary)
    }
}

/// Serializes tests that mutate the process-global `rust_i18n` locale so they
/// never overlap a test reading translated output on another thread.
#[cfg(test)]
pub(crate) static LOCALE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_names_are_unique_and_match_all() {
        for &l in Lang::ALL {
            assert!(!l.code().is_empty());
            assert!(!l.native_name().is_empty());
            // Round-trip: a language's own code resolves back to it.
            assert_eq!(Lang::from_locale_tag(l.code()), Some(l));
        }
        assert_eq!(Lang::ALL.len(), 2);
    }

    #[test]
    fn from_locale_tag_matches_primary_subtag() {
        assert_eq!(Lang::from_locale_tag("nl-NL"), Some(Lang::Dutch));
        assert_eq!(Lang::from_locale_tag("nl_BE"), Some(Lang::Dutch));
        assert_eq!(Lang::from_locale_tag("NL"), Some(Lang::Dutch));
        assert_eq!(Lang::from_locale_tag("en-US"), Some(Lang::English));
        // Unsupported languages fall through (caller defaults to English).
        assert_eq!(Lang::from_locale_tag("fr-FR"), None);
        assert_eq!(Lang::from_locale_tag(""), None);
    }

    /// Every catalog must define exactly the same keys, so a translation is
    /// never silently missing (or stale). This is the safety net that makes
    /// adding a language "drop a file and fill it in".
    #[test]
    fn every_catalog_has_the_same_keys() {
        use std::collections::BTreeSet;

        fn leaf_keys(value: &serde_yaml::Value, prefix: &str, out: &mut BTreeSet<String>) {
            match value {
                serde_yaml::Value::Mapping(map) => {
                    for (k, v) in map {
                        let key = k.as_str().unwrap_or_default();
                        let path = if prefix.is_empty() {
                            key.to_string()
                        } else {
                            format!("{prefix}.{key}")
                        };
                        leaf_keys(v, &path, out);
                    }
                }
                _ => {
                    out.insert(prefix.to_string());
                }
            }
        }

        let en: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../locales/en.yml")).expect("en.yml parses");
        let mut en_keys = BTreeSet::new();
        leaf_keys(&en, "", &mut en_keys);

        // One entry per non-English catalog; grows as languages are added.
        #[allow(clippy::single_element_loop)]
        for (code, src) in [("nl", include_str!("../locales/nl.yml"))] {
            let cat: serde_yaml::Value =
                serde_yaml::from_str(src).unwrap_or_else(|e| panic!("{code}.yml parses: {e}"));
            let mut keys = BTreeSet::new();
            leaf_keys(&cat, "", &mut keys);
            let missing: Vec<_> = en_keys.difference(&keys).collect();
            let extra: Vec<_> = keys.difference(&en_keys).collect();
            assert!(
                missing.is_empty(),
                "{code}.yml is missing keys: {missing:?}"
            );
            assert!(
                extra.is_empty(),
                "{code}.yml has keys not in en.yml: {extra:?}"
            );
        }
    }

    /// Interpolation and our `tn!` pluralization work in every locale.
    #[test]
    fn interpolation_and_plurals_render() {
        use rust_i18n::t;
        // Serialize the locale mutation: `set_locale` is global.
        let _guard = LOCALE_LOCK.lock().unwrap();
        rust_i18n::set_locale("en");
        assert_eq!(
            t!("status.cannot_read", path => "A.TXT"),
            "Cannot read A.TXT"
        );
        assert_eq!(tn!("status.matches", 1), "1 match");
        assert_eq!(tn!("status.matches", 5), "5 matches");
        assert_eq!(tn!("status.deleted", 1, name => "A.TXT"), "Deleted A.TXT");
        assert_eq!(tn!("status.deleted", 3, name => "A.TXT"), "Deleted 3 files");
        rust_i18n::set_locale("nl");
        assert_eq!(tn!("status.matches", 1), "1 resultaat");
        assert_eq!(tn!("status.matches", 5), "5 resultaten");
        rust_i18n::set_locale("en");
    }
}
