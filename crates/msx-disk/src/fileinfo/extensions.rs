//! One-line descriptions of known MSX file extensions.
//!
//! Curated from the MSX Resource Center wiki:
//! <https://www.msx.org/wiki/File_extensions_used_on_MSX>. Where the wiki lists
//! several meanings for an extension, the most common MSX one is used.

/// Extension (lowercase, no dot) to a short description.
const TABLE: &[(&str, &str)] = &[
    // Disk and tape images
    ("dsk", "Disk image for emulators"),
    ("di1", "Single-sided disk image"),
    ("di2", "Double-sided disk image"),
    ("ds1", "Single-sided disk image"),
    ("ds2", "Double-sided disk image"),
    ("img", "Raw disk image"),
    ("ddi", "DiskDupe disk image"),
    ("xsa", "Compressed disk image (XelaSoft Archive)"),
    ("dmk", "David Keil raw-track disk image"),
    ("cas", "BIOS-level cassette image for emulators"),
    ("tsx", "Cassette image (TZX-derived) for MSX"),
    ("rom", "Raw ROM image dump"),
    // BASIC and text
    ("bas", "Tokenized MSX-BASIC program"),
    ("asc", "Plain text, often a BASIC program or data"),
    ("bat", "MSX-DOS batch file (plain text)"),
    ("btm", "MSX-DOS 2 batch file"),
    ("txt", "Plain text file (ASCII, ANK or Shift-JIS)"),
    ("doc", "Plain text document"),
    ("hlp", "MSX-DOS 2 help file (plain text)"),
    ("ldr", "Tokenized BASIC loader program"),
    ("gen", "Z80 assembly source (GEN80)"),
    ("mac", "Z80 assembly source (Microsoft M80)"),
    ("asm", "Z80 assembly source"),
    ("inc", "Assembly include file"),
    ("prn", "Assembler listing (plain text)"),
    ("meg", "MegaAssembler Z80 source / Mega-ROM image"),
    // Machine code / executables
    (
        "bin",
        "BSAVE binary (7-byte BLOAD header: FE + start/end/exec)",
    ),
    ("com", "MSX-DOS/CP/M command (binary executable)"),
    ("cpm", "Renamed .COM executable"),
    ("exe", "SymbOS executable"),
    ("tsr", "Terminate-and-stay-resident program (MemMan)"),
    ("pac", "FM-PAC/PAC cartridge SRAM dump (save games)"),
    // Graphics
    ("sc0", "SCREEN 0 BLOAD image"),
    ("sc1", "SCREEN 1 BLOAD image"),
    ("sc2", "SCREEN 2 BLOAD image"),
    ("sc3", "SCREEN 3 BLOAD image"),
    ("sc4", "SCREEN 4 BLOAD image"),
    ("sc5", "SCREEN 5 BLOAD image"),
    ("sc6", "SCREEN 6 BLOAD image"),
    ("sc7", "SCREEN 7 BLOAD image"),
    ("sc8", "SCREEN 8 BLOAD image"),
    ("sca", "SCREEN 10 BLOAD image"),
    ("scb", "SCREEN 11 BLOAD image"),
    ("scc", "SCREEN 12 BLOAD image"),
    ("s12", "SCREEN 12 (SCC/YJK) image"),
    ("grp", "SCREEN 2 image (synonym for .SC2)"),
    ("pic", "SCREEN 8 image (or Yanagisawa PIC)"),
    ("ge5", "SCREEN 5 image (synonym for .SC5)"),
    ("ge7", "SCREEN 7 image (synonym for .SC7)"),
    ("ge8", "SCREEN 8 image (synonym for .SC8)"),
    ("cmp", "Compressed SCREEN 5 image with palette (DD-Graph)"),
    ("g9b", "GFX-9000 library graphic format"),
    ("mag", "Maki-chan v2 image"),
    ("mki", "Maki-chan v1 image"),
    ("max", "Maki-chan image (synonym for .MAG)"),
    ("pi", "Yanagisawa PI 16-colour image"),
    ("mif", "Compressed image"),
    ("pct", "Dynamic Publisher page"),
    ("gif", "Graphics Interchange Format image"),
    ("bmp", "Windows bitmap image"),
    ("jpg", "JPEG compressed image"),
    // Music and sound
    ("mbm", "MoonBlaster music file"),
    ("mbk", "MoonBlaster samplekit"),
    ("mbs", "MoonBlaster sample"),
    ("mbv", "MoonBlaster voice file"),
    ("mbw", "MoonBlaster wave song"),
    ("mwm", "MoonBlaster for MoonSound Wave song"),
    ("mfm", "MoonBlaster for MoonSound FM song"),
    ("mus", "FAC SoundTracker music file"),
    ("pro", "Tyfoon Pro-Tracker music file"),
    ("sbm", "SCC Blaffer NT music (SCC + PSG)"),
    ("sng", "SCC-Musixx music file"),
    ("pt3", "ProTracker 3 / Vortex Tracker II music"),
    ("mod", "Amiga module"),
    ("mid", "Standard MIDI file"),
    ("etc", "E-Tracker compiled music"),
    ("cop", "E-Tracker compiled music"),
    ("saa", "E-Tracker compiled music"),
    ("kss", "MSX music file with player code"),
    ("mgs", "MGSDRV music file (by AIN)"),
    ("mdx", "Sharp X68000-style music file"),
    ("opx", "OPLL driver music"),
    ("vgm", "Video Game Music log (many chips)"),
    ("s3m", "ScreamTracker 3 module (see MOD)"),
    ("xm", "FastTracker 2 module (see MOD)"),
    ("wav", "PCM sound sample"),
    ("mp3", "MPEG Audio Layer III file"),
    // Archives
    ("lha", "LHA archive"),
    ("lzh", "LHA archive (synonym)"),
    ("zip", "ZIP archive"),
    ("arc", "ARC archive"),
    ("arj", "ARJ archive"),
    ("pma", "PMARC archive"),
    ("gz", "GZIP archive"),
    ("ish", "ISH text-encoded binary"),
    ("ips", "IPS patch file"),
];

/// A short description of a file extension, or `None` if unknown.
/// `ext` is matched case-insensitively, without a leading dot.
pub fn describe_ext(ext: &str) -> Option<&'static str> {
    let ext = ext.to_ascii_lowercase();
    TABLE.iter().find(|(e, _)| *e == ext).map(|(_, desc)| *desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_extensions_resolve() {
        assert_eq!(describe_ext("dsk"), Some("Disk image for emulators"));
        assert_eq!(describe_ext("MBM"), Some("MoonBlaster music file"));
        assert!(describe_ext("bin").unwrap().contains("BSAVE"));
    }

    #[test]
    fn unknown_is_none() {
        assert!(describe_ext("zzz").is_none());
    }

    #[test]
    fn table_is_lowercase_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (ext, _) in TABLE {
            assert_eq!(*ext, ext.to_ascii_lowercase(), "{ext} must be lowercase");
            assert!(seen.insert(*ext), "duplicate extension {ext}");
        }
    }
}
