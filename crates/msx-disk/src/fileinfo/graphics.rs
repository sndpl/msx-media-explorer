//! Human-readable labels for MSX graphics files.
//!
//! The label is derived from the file extension, mirroring the dispatch in
//! [`crate::recoil::decode`]. It is a description only — actual decoding still
//! lives in the `recoil` module.

/// What a graphics file is, by format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsInfo {
    /// e.g. `SCREEN 7`, `DD Graph (compressed SCREEN 5)`, `MAG`.
    pub label: String,
}

/// Describe a graphics file from its name, or `None` for unknown extensions.
pub fn describe(name: &str) -> Option<GraphicsInfo> {
    let ext = name.rsplit(['/', '\\']).next()?.rsplit_once('.')?.1.to_ascii_lowercase();
    let label = match ext.as_str() {
        "sc2" | "grp" => "SCREEN 2",
        "sc3" => "SCREEN 3",
        "sc4" => "SCREEN 4",
        "sc5" | "ge5" => "SCREEN 5",
        "sc6" => "SCREEN 6",
        "sc7" | "ge7" => "SCREEN 7",
        "sc8" | "ge8" | "pic" => "SCREEN 8",
        "sr8" => "SCREEN 8 (Graph Saurus, RLE)",
        "sca" => "SCREEN 10",
        "scb" => "SCREEN 11",
        "sra" => "SCREEN 10/11 (Graph Saurus, RLE)",
        "scc" | "s12" => "SCREEN 12 (YJK)",
        "srs" => "SCREEN 12 (Graph Saurus, RLE)",
        "yjk" => "SCREEN 12 (YJK)",
        "shc" => "SCREEN 12 (Graph Saurus shape)",
        "sr5" => "SCREEN 5 (Graph Saurus, RLE)",
        "sr6" => "SCREEN 6 (Graph Saurus, RLE)",
        "sr7" => "SCREEN 7 (Graph Saurus, RLE)",
        "sri" => "SCREEN (Graph Saurus, RLE)",
        "gl5" | "sh5" => "SCREEN 5 (Graph Saurus shape)",
        "gl6" | "sh6" => "SCREEN 6 (Graph Saurus shape)",
        "gl7" | "sh7" => "SCREEN 7 (Graph Saurus shape)",
        "gl8" | "sh8" => "SCREEN 8 (Graph Saurus shape)",
        "gla" | "sha" => "SCREEN 10 (Graph Saurus shape)",
        "glb" | "shb" => "SCREEN 11 (Graph Saurus shape)",
        "glc" | "gls" => "Graph Saurus shape",
        "g9b" => "G9B (GFX9000 library image)",
        "stp" => "Dynamic Publisher stamp (SCREEN 6 shape)",
        "cmp" => "DD Graph (compressed SCREEN 5)",
        "fnt" | "pct" | "mis" => "Dynamic Publisher page/font",
        "mif" => "MIF (compressed image)",
        "mig" => "MIG (compressed image)",
        "mag" | "mki" | "max" => "MAG (Maki-chan image)",
        "pi" => "PI (Yanagisawa image)",
        _ => return None,
    };
    Some(GraphicsInfo {
        label: label.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_common_screens() {
        assert_eq!(describe("FOO.SC7").unwrap().label, "SCREEN 7");
        assert_eq!(describe("path/to/BAR.s12").unwrap().label, "SCREEN 12 (YJK)");
        assert_eq!(describe("X.CMP").unwrap().label, "DD Graph (compressed SCREEN 5)");
        assert_eq!(describe("Y.MAG").unwrap().label, "MAG (Maki-chan image)");
    }

    #[test]
    fn unknown_extension_is_none() {
        assert!(describe("readme.txt").is_none());
        assert!(describe("noext").is_none());
    }
}
