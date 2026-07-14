use super::*;

/// Build an app with a mounted synthetic floppy: `dirs` created first (parents
/// before children), then `files` added.
#[cfg(test)]
fn app_with_disk(dirs: &[&str], files: &[(&str, &[u8])]) -> MediaExplorerApp {
    use msx_disk::fs::write;
    use msx_disk::image::geometry::DiskFormat;
    let mut bytes = write::create_blank(DiskFormat::Ds720).unwrap();
    for d in dirs {
        bytes = write::create_dir(&bytes, d).unwrap();
    }
    bytes = write::add_files(&bytes, files).unwrap();
    let image = msx_disk::image::DiskImage::open_bytes(msx_disk::ImageFormat::Dsk, bytes).unwrap();
    let disk = LoadedDisk::from_image(image, None).unwrap();
    MediaExplorerApp {
        disk: Some(disk),
        charset: MsxCharset::International,
        ..Default::default()
    }
}

/// Relative host paths of a plan, as `/`-joined strings, sorted for comparison.
#[cfg(test)]
fn rels(planned: &[super::transfer::Planned]) -> Vec<String> {
    let mut v: Vec<String> = planned
        .iter()
        .map(|p| p.rel.to_string_lossy().replace('\\', "/"))
        .collect();
    v.sort();
    v
}

#[test]
fn plan_extraction_walks_a_directory_preserving_structure() {
    let app = app_with_disk(
        &["UTILS", "UTILS/SUB"],
        &[
            ("HELLO.TXT", b"hi"),
            ("UTILS/GAME.COM", b"game"),
            ("UTILS/SUB/DEEP.BIN", b"deep"),
        ],
    );
    let plan = app.plan_extraction(&["UTILS".to_string()]);
    assert_eq!(rels(&plan), vec!["UTILS/GAME.COM", "UTILS/SUB/DEEP.BIN"]);
    // Each planned item reads from its real disk path.
    assert!(plan.iter().all(|p| p.disk_path.starts_with("UTILS/")));
}

#[test]
fn plan_extraction_file_uses_base_name_not_full_path() {
    let app = app_with_disk(&["UTILS"], &[("UTILS/GAME.COM", b"game")]);
    let plan = app.plan_extraction(&["UTILS/GAME.COM".to_string()]);
    assert_eq!(rels(&plan), vec!["GAME.COM"]);
    assert_eq!(plan[0].disk_path, "UTILS/GAME.COM");
}

#[test]
fn write_extractions_creates_subdirectories_and_applies_timestamps() {
    let app = app_with_disk(&[], &[("HELLO.TXT", b"payload")]);
    let modified = msx_disk::fs::Timestamp {
        year: 1990,
        month: 5,
        day: 12,
        hour: 14,
        minute: 30,
    };
    let plan = vec![
        super::transfer::Planned {
            rel: std::path::PathBuf::from("UTILS/SUB/DEEP.TXT"),
            disk_path: "HELLO.TXT".to_string(),
            modified: Some(modified),
        },
        super::transfer::Planned {
            rel: std::path::PathBuf::from("TOP.TXT"),
            disk_path: "HELLO.TXT".to_string(),
            modified: None,
        },
    ];
    let dest = std::env::temp_dir().join(format!("megui-tree-{}", std::process::id()));
    std::fs::create_dir_all(&dest).unwrap();

    let (ok, failed) = app.write_extractions(&dest, &plan);
    assert_eq!((ok, failed), (2, 0));

    let deep = dest.join("UTILS/SUB/DEEP.TXT");
    assert_eq!(std::fs::read(&deep).unwrap(), b"payload");
    assert_eq!(
        std::fs::metadata(&deep).unwrap().modified().unwrap(),
        modified.to_system_time().unwrap()
    );
    assert!(dest.join("TOP.TXT").exists());
    std::fs::remove_dir_all(&dest).unwrap();
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn stage_files_for_drag_stages_a_directory_tree() {
    let app = app_with_disk(
        &["UTILS", "UTILS/SUB"],
        &[("UTILS/GAME.COM", b"game"), ("UTILS/SUB/DEEP.BIN", b"deep")],
    );
    let roots = app.stage_files_for_drag(&["UTILS".to_string()]).unwrap();
    // The OS drag is handed the folder itself, not each file.
    assert_eq!(roots.len(), 1);
    let root = &roots[0];
    assert_eq!(root.file_name().unwrap(), "UTILS");
    assert!(root.is_dir());
    assert_eq!(std::fs::read(root.join("GAME.COM")).unwrap(), b"game");
    assert_eq!(std::fs::read(root.join("SUB/DEEP.BIN")).unwrap(), b"deep");
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn file_filter_shows_only_matches_and_their_ancestors() {
    let mut app = app_with_disk(
        &["GAMES", "UTILS", "UTILS/SUB"],
        &[
            ("GAMES/A.PIC", b"a"),
            ("GAMES/B.SC5", b"b"),
            ("UTILS/SUB/C.PIC", b"c"),
            ("HELLO.BAS", b"h"),
        ],
    );

    // `*.PIC` keeps the two .PIC files and every folder on the way to them (plus
    // the synthetic "/" root), and hides everything else. This is the exact path
    // the renderer and keyboard nav consume.
    app.filter = "*.PIC".to_string();
    let mut rows: Vec<String> = app
        .visible_tree_rows()
        .into_iter()
        .map(|r| r.path)
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            "",
            "GAMES",
            "GAMES/A.PIC",
            "UTILS",
            "UTILS/SUB",
            "UTILS/SUB/C.PIC"
        ]
    );

    // A pattern that matches nothing hides the whole tree.
    app.filter = "*.XYZ".to_string();
    assert!(app.visible_tree_rows().is_empty());

    // Clearing the filter restores the full tree.
    app.filter.clear();
    let all: Vec<String> = app
        .visible_tree_rows()
        .into_iter()
        .map(|r| r.path)
        .collect();
    assert!(all.contains(&"GAMES/B.SC5".to_string()));
    assert!(all.contains(&"HELLO.BAS".to_string()));
}

#[test]
fn file_filter_single_char_wildcard_aliases() {
    let mut app = app_with_disk(
        &[],
        &[
            ("IMG1.SC5", b"1"),
            ("IMG9.SC5", b"9"),
            ("IMG10.SC5", b"0"),
            ("IMG.SC5", b"x"),
        ],
    );
    for pattern in ["img%.sc5", "img?.sc5"] {
        app.filter = pattern.to_string();
        let mut rows: Vec<String> = app
            .visible_tree_rows()
            .into_iter()
            .map(|r| r.path)
            .collect();
        rows.sort();
        // `%`/`?` is exactly one char: IMG1/IMG9 match, IMG10 and IMG do not.
        assert_eq!(rows, vec!["", "IMG1.SC5", "IMG9.SC5"], "pattern {pattern}");
    }
}

#[test]
fn real_dsk_directory_extracts_with_structure_and_timestamps() {
    // Skip-if-absent, like the msx-disk real-fixture tests: the .dsk is not
    // committed but gives real-world coverage on machines that have it.
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/MSX-DOS2 TOOLS.dsk");
    if !path.exists() {
        eprintln!("skipping: fixture 'MSX-DOS2 TOOLS.dsk' not present");
        return;
    }
    let image = msx_disk::image::DiskImage::open(&path).unwrap();
    let app = MediaExplorerApp {
        disk: Some(LoadedDisk::from_image(image, None).unwrap()),
        charset: MsxCharset::International,
        ..Default::default()
    };

    let plan = app.plan_extraction(&["TOOLS".to_string()]);
    assert!(!plan.is_empty(), "TOOLS/ should contain files");
    assert!(
        plan.iter().all(|p| p.rel.starts_with("TOOLS")),
        "every file is rooted at the TOOLS folder"
    );
    // The disk's TOOLS files carry real 1989-1992 dates, not "now".
    assert!(
        plan.iter()
            .filter_map(|p| p.modified)
            .any(|t| t.year < 2000),
        "expected vintage FAT timestamps"
    );

    let dest = std::env::temp_dir().join(format!("megui-realdir-{}", std::process::id()));
    std::fs::create_dir_all(&dest).unwrap();
    let (ok, failed) = app.write_extractions(&dest, &plan);
    assert_eq!((ok, failed), (plan.len(), 0));

    let sample = &plan[0];
    let out = dest.join(&sample.rel);
    assert!(out.exists() && out.starts_with(dest.join("TOOLS")));
    if let Some(ts) = sample.modified {
        assert_eq!(
            std::fs::metadata(&out).unwrap().modified().unwrap(),
            ts.to_system_time().unwrap()
        );
    }
    std::fs::remove_dir_all(&dest).ok();
}

#[test]
fn select_all_files_selects_visible_files_not_directories() {
    let mut app = app_with_disk(
        &["UTILS"],
        &[
            ("HELLO.TXT", b"hi"),
            ("UTILS/GAME.COM", b"game"),
            ("UTILS/README.TXT", b"doc"),
        ],
    );
    app.select_all_files();
    let selected: Vec<&str> = app.selection.iter().map(String::as_str).collect();
    assert_eq!(
        selected,
        vec!["HELLO.TXT", "UTILS/GAME.COM", "UTILS/README.TXT"]
    );

    // With a filter active, only the visible (matching) files are selected.
    app.selection.clear();
    app.filter = "*.TXT".to_string();
    app.select_all_files();
    let selected: Vec<&str> = app.selection.iter().map(String::as_str).collect();
    assert_eq!(selected, vec!["HELLO.TXT", "UTILS/README.TXT"]);
}

#[test]
fn control_char_filenames_sanitize_to_pictures() {
    // Skip-if-absent: jaarg-hw.di1 has crafted directory entries whose names are
    // C0 control bytes (BEL/CR/LF/FF/SUB).
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/jaarg-hw.di1");
    if !path.exists() {
        eprintln!("skipping: fixture 'jaarg-hw.di1' not present");
        return;
    }
    let image = msx_disk::image::DiskImage::open(&path).unwrap();
    let disk = LoadedDisk::from_image(image, None).unwrap();
    let names: Vec<String> = disk
        .tree
        .iter()
        .flat_map(|e| e.walk())
        .filter(|e| !e.is_dir)
        .map(|e| {
            msx_disk::charset::display_control_safe(&e.display_name(MsxCharset::International))
        })
        .collect();

    // The crafted entry's BEL byte becomes its control picture (␇), and no raw
    // control byte survives into any displayed name.
    assert!(
        names.iter().any(|n| n.contains('\u{2407}')),
        "expected a BEL control picture among: {names:?}"
    );
    assert!(
        names
            .iter()
            .all(|n| !n.chars().any(|c| (c as u32) < 0x20 || c == '\u{7F}')),
        "no raw control characters should remain in a displayed name"
    );
}

#[test]
fn extracted_file_keeps_its_fat_modification_time() {
    let dir = std::env::temp_dir().join(format!("megui-mtime-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("VINTAGE.PIC");

    let modified = msx_disk::fs::Timestamp {
        year: 1990,
        month: 5,
        day: 12,
        hour: 14,
        minute: 30,
    };
    super::transfer::write_extracted(&target, b"pixels", Some(modified)).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"pixels");
    let got = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(got, modified.to_system_time().unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn about_icon_asset_decodes() {
    // The About window decodes this embedded PNG on first open; guard
    // against a broken or wrong asset being bundled.
    let img = image::load_from_memory(ICON_PNG).expect("embedded icon is a valid PNG");
    let (w, h) = image::GenericImageView::dimensions(&img);
    assert_eq!(w, h, "app icon should be square");
    assert!(w >= 128, "app icon should be reasonably hi-res, got {w}px");
}

#[test]
fn hex_edit_format_parse_roundtrip() {
    let bytes: Vec<u8> = (0u8..50).collect();
    let text = format_hex_for_edit(&bytes);
    assert!(text.contains('\n'), "should wrap at 16 bytes");
    assert_eq!(msx_disk::search::parse_hex(&text), Some(bytes));
}

#[test]
fn largest_file_label_decodes_pua_names_under_charset() {
    // A PUA-encoded path (high bytes carried in U+F080..=U+F0FF) must render
    // as decoded glyphs under the active charset, not raw PUA — which shows
    // as tofu boxes in the Stats "Largest files" list.
    let raw = "\u{F0B1}\u{F0B2}\u{F0B3}-01.PIC";
    let label = largest_file_label(27_801, raw, MsxCharset::Japanese);
    assert!(
        !label
            .chars()
            .any(|c| ('\u{F080}'..='\u{F0FF}').contains(&c)),
        "label still shows raw PUA bytes (tofu): {label:?}"
    );
    // It contains the same decoded name the file tree shows.
    let decoded = charset::decode_fs_name(MsxCharset::Japanese, raw);
    assert!(
        label.ends_with(&decoded),
        "label {label:?} should end with {decoded:?}"
    );
}

#[test]
fn normalize_orders_endpoints() {
    assert_eq!(normalize(5, 2), (2, 5));
    assert_eq!(normalize(2, 5), (2, 5));
    assert_eq!(normalize(7, 7), (7, 7));
}

#[test]
fn parse_offset_accepts_hex_forms() {
    assert_eq!(parse_offset("1A0"), Some(0x1A0));
    assert_eq!(parse_offset("0x1a0"), Some(0x1A0));
    assert_eq!(parse_offset("  0X10 "), Some(0x10));
    assert_eq!(parse_offset(""), None);
    assert_eq!(parse_offset("xyz"), None);
}

#[test]
fn hex_gesture_updates_selection_state() {
    let mut hs = HexUiState::default();
    // A plain click sets the cursor, clears any selection, and records the
    // column it landed in.
    apply_hex_gesture(
        &mut hs,
        HexGesture::Click {
            byte: 4,
            shift: false,
            region: HexRegion::Hex,
        },
    );
    assert_eq!(hs.cursor, Some(4));
    assert_eq!(hs.selection, None);
    assert_eq!(hs.region, HexRegion::Hex);
    // Shift-click extends from the cursor and updates the column.
    apply_hex_gesture(
        &mut hs,
        HexGesture::Click {
            byte: 9,
            shift: true,
            region: HexRegion::Ascii,
        },
    );
    assert_eq!(hs.selection, Some((4, 9)));
    assert_eq!(hs.region, HexRegion::Ascii);
    // A drag selects from anchor to the dragged byte (normalized) and keeps
    // the column the drag began in.
    apply_hex_gesture(
        &mut hs,
        HexGesture::DragStart {
            byte: 20,
            region: HexRegion::Ascii,
        },
    );
    apply_hex_gesture(&mut hs, HexGesture::DragTo(12));
    assert_eq!(hs.selection, Some((12, 20)));
    assert_eq!(hs.region, HexRegion::Ascii);
}

#[test]
fn sanitize_makes_msx_83_names() {
    assert_eq!(sanitize_msx_name("my long file.text"), "MYLONGFI.TEX");
    assert_eq!(sanitize_msx_name("a.b"), "A.B");
    assert_eq!(sanitize_msx_name(""), "FILE");
    assert_eq!(sanitize_msx_name("game.com"), "GAME.COM");
}

#[test]
fn normalize_input_enforces_83_shape() {
    let intl = MsxCharset::International;
    // Uppercases, drops disallowed characters, caps stem and extension.
    assert_eq!(
        normalize_msx_input("my long file.text", intl),
        "MYLONGFI.TEX"
    );
    assert_eq!(normalize_msx_input("game.com", intl), "GAME.COM");
    // Only the first dot separates; later dots are dropped.
    assert_eq!(normalize_msx_input("a.b.c", intl), "A.BC");
    // Spaces and other illegal characters are filtered out.
    assert_eq!(normalize_msx_input("a b/c?", intl), "ABC");
}

#[test]
fn normalize_input_allows_in_progress_typing() {
    let intl = MsxCharset::International;
    // Empty stays empty (the field can be cleared) instead of "FILE".
    assert_eq!(normalize_msx_input("", intl), "");
    // A trailing dot is kept so the extension can still be typed.
    assert_eq!(normalize_msx_input("GAME.", intl), "GAME.");
}

/// Three half-width katakana (bytes 0xB1..=0xB3) as decoded display glyphs.
fn sample_kana() -> String {
    (0xB1u8..=0xB3)
        .map(|b| charset::decode_byte(MsxCharset::Japanese, b))
        .collect()
}

#[test]
fn rename_normalize_keeps_japanese_glyphs() {
    let kana = sample_kana();
    let out = normalize_msx_input(&format!("{kana}.bas"), MsxCharset::Japanese);
    assert_eq!(out, format!("{kana}.BAS"));
}

#[test]
fn rename_normalize_drops_glyphs_outside_charset() {
    // Kana isn't representable in International, so it is stripped; ASCII stays.
    let out = normalize_msx_input(
        &format!("{}AB.bas", sample_kana()),
        MsxCharset::International,
    );
    assert_eq!(out, "AB.BAS");
}

#[test]
fn rename_round_trips_japanese_name_to_fatfs_key() {
    // The on-disk (PUA) key decodes for display, sanitizes, and re-encodes
    // to the exact same key — proving a rename preserves Japanese names.
    let key = "\u{F0B1}\u{F0B2}\u{F0B3}.BAS";
    let display = charset::decode_fs_name(MsxCharset::Japanese, key);
    let sane = sanitize_8_3(&display, |c| is_rename_char(c, MsxCharset::Japanese));
    let encoded = charset::encode_fs_name(MsxCharset::Japanese, &sane).unwrap();
    assert_eq!(encoded, key);
}

fn dir_entry(path: &str, is_dir: bool, size: u64, children: Vec<DirEntry>) -> DirEntry {
    use msx_disk::fs::Attributes;
    DirEntry {
        name: path.rsplit('/').next().unwrap().to_string(),
        path: path.to_string(),
        is_dir,
        size,
        attributes: Attributes::default(),
        modified: None,
        children,
    }
}

#[test]
fn add_target_dir_resolves_the_destination_folder() {
    // Root row → root; a directory → itself; a file → its parent folder.
    assert_eq!(add_target_dir(ROOT_PATH, true), "");
    assert_eq!(add_target_dir("TOOLS", true), "TOOLS");
    assert_eq!(add_target_dir("TOOLS/SUB", true), "TOOLS/SUB");
    assert_eq!(add_target_dir("TOOLS/ASM.COM", false), "TOOLS");
    // A root-level file has no parent folder, so it targets the root.
    assert_eq!(add_target_dir("HELLO.BAS", false), "");
}

#[test]
fn child_path_joins_unless_at_root() {
    assert_eq!(child_path("", "FILE.TXT"), "FILE.TXT");
    assert_eq!(child_path("TOOLS", "ASM.COM"), "TOOLS/ASM.COM");
    assert_eq!(child_path("A/B", "C.COM"), "A/B/C.COM");
}

#[test]
fn removal_paths_lists_descendants_before_their_folder() {
    // TOOLS/ { A.TXT, SUB/ { B.TXT } }
    let dir = dir_entry(
        "TOOLS",
        true,
        0,
        vec![
            dir_entry("TOOLS/A.TXT", false, 1, vec![]),
            dir_entry(
                "TOOLS/SUB",
                true,
                0,
                vec![dir_entry("TOOLS/SUB/B.TXT", false, 1, vec![])],
            ),
        ],
    );
    let order = removal_paths(&dir);
    // Every entry appears after all of its descendants, and the folder last.
    let pos = |p: &str| order.iter().position(|x| x == p).unwrap();
    assert!(pos("TOOLS/SUB/B.TXT") < pos("TOOLS/SUB"));
    assert!(pos("TOOLS/A.TXT") < pos("TOOLS"));
    assert!(pos("TOOLS/SUB") < pos("TOOLS"));
    assert_eq!(order.last().unwrap(), "TOOLS");
    assert_eq!(order.len(), 4);
}

#[test]
fn drop_target_at_picks_the_row_under_the_pointer() {
    let targets = vec![
        (
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 10.0)),
            "".to_string(),
        ),
        (
            egui::Rect::from_min_max(egui::pos2(0.0, 10.0), egui::pos2(100.0, 20.0)),
            "TOOLS".to_string(),
        ),
    ];
    assert_eq!(
        drop_target_at(&targets, egui::pos2(50.0, 15.0)).as_deref(),
        Some("TOOLS")
    );
    assert_eq!(
        drop_target_at(&targets, egui::pos2(50.0, 5.0)).as_deref(),
        Some("")
    );
    // A drop below every row hits nothing.
    assert_eq!(drop_target_at(&targets, egui::pos2(50.0, 99.0)), None);
}

#[test]
fn directory_stats_counts_immediate_and_recursive() {
    // KIDS/ { KID.COM(100), AKID.COM(200), SUB/ { C.COM(50) } }
    let dir = dir_entry(
        "KIDS",
        true,
        0,
        vec![
            dir_entry("KIDS/KID.COM", false, 100, vec![]),
            dir_entry("KIDS/AKID.COM", false, 200, vec![]),
            dir_entry(
                "KIDS/SUB",
                true,
                0,
                vec![dir_entry("KIDS/SUB/C.COM", false, 50, vec![])],
            ),
        ],
    );
    let s = directory_stats(&dir);
    assert_eq!(s.files, 2, "immediate files");
    assert_eq!(s.dirs, 1, "immediate subdirectories");
    assert_eq!(s.total_files, 3, "files including nested");
    assert_eq!(s.total_dirs, 1, "subdirectories including nested");
    assert_eq!(s.total_bytes, 350, "recursive sum of file sizes");
}

#[test]
fn directory_stats_of_empty_directory_is_all_zero() {
    let dir = dir_entry("EMPTY", true, 0, vec![]);
    let s = directory_stats(&dir);
    assert_eq!(
        (s.files, s.dirs, s.total_files, s.total_dirs, s.total_bytes),
        (0, 0, 0, 0, 0)
    );
}

#[test]
fn focusing_a_directory_clears_the_file_selection() {
    // The "both stay blue" bug: a previously selected file kept its
    // highlight when the cursor moved onto a directory. Focusing a
    // directory must drop the file selection so only the folder is shown.
    let mut app = MediaExplorerApp {
        selected: Some("KIDS/KID.COM".to_string()),
        content: Some(FileContent {
            path: "KIDS/KID.COM".to_string(),
            bytes: vec![1, 2, 3],
            checksums: msx_disk::Checksums::of(&[1, 2, 3]),
            info: OnceCell::new(),
            rendered: RefCell::new(None),
        }),
        ..Default::default()
    };
    app.selection.insert("KIDS/KID.COM".to_string());

    app.focus_directory("KIDS".to_string());

    assert_eq!(app.cursor.as_deref(), Some("KIDS"));
    assert!(app.selection.is_empty(), "file selection cleared");
    assert!(app.selected.is_none());
    assert!(
        app.content.is_none(),
        "file viewer cleared for the dir info pane"
    );
}

#[test]
fn msx_name_stem_detects_applicable_names() {
    assert_eq!(msx_name_stem("GAME.COM"), "GAME");
    assert_eq!(msx_name_stem("GAME"), "GAME");
    // No stem means nothing to apply.
    assert!(msx_name_stem("").is_empty());
    assert!(msx_name_stem(".COM").is_empty());
}

/// A synthetic, minimal SCREEN 2 BSAVE buffer (single top-left pixel), used
/// to exercise format selection without a real disk fixture.
fn synthetic_sc2() -> Vec<u8> {
    let mut buf = vec![0u8; 14343];
    buf[0] = 0xfe;
    buf[3] = 0xff;
    buf[4] = 0x37;
    buf[7] = 0x80;
    buf[0x2007] = 0xf1;
    buf
}

#[test]
fn screen_failure_message_distinguishes_forced_attempt() {
    // A failed manual attempt names the format, so picking a type that does
    // not decode produces a visibly different message (proving an attempt
    // was made) rather than the constant "not recognized" placeholder.
    let forced = screen_decode_failed_message(Some(recoil::ImageFormat::Screen8));
    assert!(
        forced.contains("SCREEN 8"),
        "forced-failure message should name the format: {forced}"
    );
    assert!(
        forced.to_lowercase().contains("could not"),
        "should read as a failed attempt: {forced}"
    );

    // The auto (no forced format) message is distinct and points at the
    // picker.
    let auto = screen_decode_failed_message(None);
    assert!(!auto.contains("SCREEN 8"));
    assert!(
        auto.to_lowercase().contains("pick a format"),
        "auto message should point at the picker: {auto}"
    );
}

#[test]
fn decode_screen_uses_extension_when_no_format_forced() {
    let buf = synthetic_sc2();
    assert!(decode_screen(None, "pic.sc2", &buf, &recoil::NoCompanions).is_some());
    // An unknown extension can't be classified, so auto-decode fails.
    assert!(decode_screen(None, "pic.dat", &buf, &recoil::NoCompanions).is_none());
}

#[test]
fn decode_screen_forced_format_overrides_extension() {
    let buf = synthetic_sc2();
    // Forcing SCREEN 2 decodes a file whose extension is unrecognized.
    assert!(decode_screen(
        Some(recoil::ImageFormat::Screen2),
        "pic.dat",
        &buf,
        &recoil::NoCompanions
    )
    .is_some());
    // Forcing a mismatched format on a recognized .sc2 fails (these are not
    // SCREEN 8 bytes), proving the forced format wins over the extension.
    assert!(decode_screen(
        Some(recoil::ImageFormat::Screen8),
        "pic.sc2",
        &buf,
        &recoil::NoCompanions
    )
    .is_none());
}

#[test]
fn display_ext_replaces_nonprintable_but_keeps_char_count() {
    // Normal ASCII extensions pass through unchanged.
    assert_eq!(display_ext("pct"), "pct");
    assert_eq!(display_ext(""), "");

    // A "strange" filename whose extension is C0 control bytes (SUB + FF +
    // FF, as seen on real disks): each control char becomes the single-width
    // placeholder, and crucially the char count is preserved so the
    // monospace `{:<7}` column padding stays aligned. Every output char is a
    // single ASCII cell, so nothing renders at an unexpected width.
    let ctrl = "\u{001a}\u{000c}\u{000c}";
    let shown = display_ext(ctrl);
    assert_eq!(shown, "???");
    assert_eq!(shown.chars().count(), ctrl.chars().count());
    assert!(shown.chars().all(|c| c.is_ascii_graphic()));

    // Mixed printable + control, and undecoded PUA high bytes, are sanitized
    // too while leaving the printable parts intact.
    assert_eq!(display_ext("a\u{001a}b"), "a?b");
    assert_eq!(display_ext("\u{F081}"), "?");
}

#[test]
fn screen_display_size_caps_small_images_at_2x() {
    // A 256x192 SCREEN 2 picture in a roomy panel: shown at native 2x, not
    // enlarged to fill the space.
    let size = screen_display_size(egui::vec2(256.0, 192.0), egui::vec2(2000.0, 2000.0));
    assert_eq!(size, egui::vec2(512.0, 384.0));
}

#[test]
fn screen_display_size_shrinks_tall_pages_to_fit() {
    // A 512x1408 Dynamic Publisher page (2x native = 1024x2816) shrunk to a
    // 900px-tall panel: it fits within the panel and keeps its aspect ratio.
    let available = egui::vec2(1000.0, 900.0);
    let size = screen_display_size(egui::vec2(512.0, 1408.0), available);
    assert!(size.x <= available.x + 0.01 && size.y <= available.y + 0.01);
    // Height is the limiting dimension here, so it pins to the panel height.
    assert!((size.y - 900.0).abs() < 0.01, "{size:?}");
    let native = egui::vec2(512.0, 1408.0) * 2.0;
    assert!(
        (size.x / size.y - native.x / native.y).abs() < 1e-4,
        "aspect ratio preserved: {size:?}"
    );
}

#[test]
fn screen_display_size_handles_degenerate_inputs() {
    // Zero available space collapses to zero rather than producing NaN.
    let size = screen_display_size(egui::vec2(256.0, 192.0), egui::vec2(0.0, 0.0));
    assert_eq!(size, egui::vec2(0.0, 0.0));
    // A zero-size texture is returned unscaled (and never divides by zero).
    let size = screen_display_size(egui::vec2(0.0, 0.0), egui::vec2(100.0, 100.0));
    assert_eq!(size, egui::vec2(0.0, 0.0));
}

#[test]
fn default_view_mode_by_extension() {
    assert_eq!(default_view_mode("PIC.SC8"), ViewMode::Screen);
    assert_eq!(default_view_mode("PROG.BAS"), ViewMode::Basic);
    assert_eq!(default_view_mode("DATA.BIN"), ViewMode::Disasm);
    assert_eq!(default_view_mode("GAME.COM"), ViewMode::Disasm);
    assert_eq!(default_view_mode("TOOL.cpm"), ViewMode::Disasm);
    assert_eq!(default_view_mode("README.TXT"), ViewMode::Text);
    assert_eq!(default_view_mode("AUTOEXEC.BAT"), ViewMode::Text);
    assert_eq!(default_view_mode("notes.txt"), ViewMode::Text);
    assert_eq!(default_view_mode("SONG.MBM"), ViewMode::Info);
    assert_eq!(default_view_mode("TUNE.mod"), ViewMode::Info);
    assert_eq!(default_view_mode("track.pt3"), ViewMode::Info);
    assert_eq!(default_view_mode("GAME.LZH"), ViewMode::Archive);
    assert_eq!(default_view_mode("util.lha"), ViewMode::Archive);
    assert_eq!(default_view_mode("DEMO.LZS"), ViewMode::Archive);
    assert_eq!(default_view_mode("SNOOPY.pma"), ViewMode::Archive);
}

#[test]
fn sanitize_member_path_preserves_subdirs_and_blocks_traversal() {
    assert_eq!(sanitize_member_path("FILE.BIN"), PathBuf::from("FILE.BIN"));
    assert_eq!(
        sanitize_member_path("SUB/DIR/FILE.BIN"),
        PathBuf::from("SUB/DIR/FILE.BIN")
    );
    // Backslash separators (MS-DOS) are normalized.
    assert_eq!(
        sanitize_member_path("SUB\\FILE.BIN"),
        PathBuf::from("SUB/FILE.BIN")
    );
    // `..` and leading separators cannot escape the chosen folder.
    assert_eq!(
        sanitize_member_path("../../etc/passwd"),
        PathBuf::from("etc/passwd")
    );
    assert_eq!(sanitize_member_path("/abs/path"), PathBuf::from("abs/path"));
    assert_eq!(sanitize_member_path("../.."), PathBuf::from("extracted"));
}

fn file_entry(
    name: &str,
    size: u64,
    attributes: msx_disk::fs::Attributes,
    modified: Option<msx_disk::fs::Timestamp>,
) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        path: name.to_string(),
        is_dir: false,
        size,
        attributes,
        modified,
        children: Vec::new(),
    }
}

#[test]
fn attributes_render_as_rhsa_flags() {
    use msx_disk::fs::Attributes;
    assert_eq!(format_attributes(Attributes::default()), "----");
    assert_eq!(
        format_attributes(Attributes {
            read_only: true,
            archive: true,
            ..Attributes::default()
        }),
        "R--A"
    );
    assert_eq!(
        format_attributes(Attributes {
            read_only: true,
            hidden: true,
            system: true,
            archive: true,
        }),
        "RHSA"
    );
}

#[test]
fn timestamp_formats_iso_minute_or_empty() {
    use msx_disk::fs::Timestamp;
    assert_eq!(
        format_timestamp(Some(Timestamp {
            year: 1991,
            month: 3,
            day: 25,
            hour: 14,
            minute: 30,
        })),
        "1991-03-25 14:30"
    );
    assert_eq!(format_timestamp(None), "");
}

#[test]
fn file_row_always_includes_date_and_attributes() {
    use msx_disk::fs::{Attributes, Timestamp};
    // DOS1 disks carry timestamps too, so a file with one always shows it.
    let e = file_entry(
        "GAME.COM",
        1234,
        Attributes {
            archive: true,
            ..Attributes::default()
        },
        Some(Timestamp {
            year: 1991,
            month: 3,
            day: 25,
            hour: 14,
            minute: 30,
        }),
    );
    let cols = file_row_columns(&e);
    assert!(cols.starts_with(&format!("{:>8}", 1234u64)), "cols: {cols}");
    assert!(cols.contains("1991-03-25 14:30"), "cols: {cols}");
    assert!(cols.trim_end().ends_with("---A"), "cols: {cols}");
}

#[test]
fn info_name_decodes_under_charset_like_the_file_list() {
    // A PUA-encoded high byte (0xB1 -> U+F0B1) must decode to the same glyph
    // the file list shows, not render as the raw PUA name.
    let name = "\u{F0B1}.BIN";
    let e = file_entry(name, 10, msx_disk::fs::Attributes::default(), None);
    let shown = info_display_name(Some(&e), "DIR/\u{F0B1}.BIN", MsxCharset::Japanese);
    // Matches the file list's decoded name...
    assert_eq!(shown, e.display_name(MsxCharset::Japanese));
    // ...and is no longer the raw, undecoded PUA string.
    assert_ne!(shown, name);
}

#[test]
fn info_name_falls_back_to_path_base_when_no_entry() {
    // With no DirEntry, the path's base name is still decoded under charset.
    let shown = info_display_name(None, "DIR/\u{F0B1}.BIN", MsxCharset::Japanese);
    assert_eq!(
        shown,
        charset::decode_fs_name(MsxCharset::Japanese, "\u{F0B1}.BIN")
    );
}

#[test]
fn file_row_width_matches_panel_sizing_constant() {
    use msx_disk::fs::{Attributes, Timestamp};
    // A maximal file row must be exactly FILE_ROW_CHARS wide, so the panel's
    // default width (derived from that constant) fits a full row on one line.
    let e = file_entry(
        "ABCDEFGH.IJK",
        99_999_999,
        Attributes {
            read_only: true,
            hidden: true,
            system: true,
            archive: true,
        },
        Some(Timestamp {
            year: 9999,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
        }),
    );
    // Name field + data columns together must equal the sizing constant.
    let cols = file_row_columns(&e);
    assert_eq!(
        NAME_FIELD_CHARS + cols.chars().count(),
        FILE_ROW_CHARS,
        "cols: {cols:?}"
    );
}

#[test]
fn empty_fat_message_names_fat_and_points_to_raw_views() {
    let msg = empty_fat_message();
    assert!(msg.to_lowercase().contains("fat"), "msg: {msg}");
    assert!(msg.contains("Sectors"), "msg: {msg}");
}

#[test]
fn file_row_keeps_columns_when_timestamp_absent() {
    // No timestamp -> blank date column, but the attribute column remains.
    let e = file_entry("GAME.COM", 1234, msx_disk::fs::Attributes::default(), None);
    let cols = file_row_columns(&e);
    assert!(cols.starts_with(&format!("{:>8}", 1234u64)), "cols: {cols}");
    assert!(cols.trim_end().ends_with("----"), "cols: {cols}");
}

#[test]
fn disk_search_jump_maps_offset_to_sector_and_row() {
    // Offset 1234 -> sector 2 (1024..1536), row (1234 % 512) / 16 = 210/16 = 13.
    let mut app = MediaExplorerApp {
        disk_search_matches: vec![1234],
        ..Default::default()
    };
    app.jump_to_disk_match();
    assert_eq!(app.current_sector, 2);
    assert_eq!(app.sector_scroll_row, Some(13));
    assert_eq!(app.sector_highlight, Some((2, 13)));
}

#[test]
fn disk_search_step_wraps_in_both_directions() {
    let mut app = MediaExplorerApp {
        disk_search_matches: vec![0, 512, 1024],
        ..Default::default()
    };
    app.step_disk_search(true);
    assert_eq!(app.disk_search_pos, 1);
    app.step_disk_search(false);
    assert_eq!(app.disk_search_pos, 0);
    // Wrap backwards from first to last.
    app.step_disk_search(false);
    assert_eq!(app.disk_search_pos, 2);
    // Wrap forwards from last to first.
    app.step_disk_search(true);
    assert_eq!(app.disk_search_pos, 0);
}

#[test]
fn disk_search_step_is_noop_without_matches() {
    let mut app = MediaExplorerApp::default();
    app.step_disk_search(true);
    assert_eq!(app.disk_search_pos, 0);
}

#[test]
fn base_name_takes_last_path_component() {
    assert_eq!(base_name("A/B/C.BIN"), "C.BIN");
    assert_eq!(base_name("ROOT.COM"), "ROOT.COM");
    assert_eq!(base_name("DIR/SUB/"), "");
}

#[test]
fn describe_geometry_names_standard_formats() {
    let desc = describe_geometry(Geometry::DS_720K);
    assert!(
        desc.starts_with("3.5\" Double Sided, Double Density (2DD)"),
        "{desc}"
    );
    assert!(desc.contains("2 sides"), "{desc}");
    assert!(desc.contains("80 tracks"), "{desc}");
    assert!(desc.contains("= 737280 bytes (720 kB)"), "{desc}");

    let desc360 = describe_geometry(Geometry::SS_360K);
    assert!(
        desc360.starts_with("3.5\" Single Sided, Double Density (1DD)"),
        "{desc360}"
    );
    assert!(desc360.contains("1 side "), "singular: {desc360}");
    assert!(desc360.contains("(360 kB)"), "{desc360}");
}

#[test]
fn describe_geometry_falls_back_to_arithmetic_for_unknown() {
    let odd = Geometry {
        sides: 2,
        tracks: 35,
        sectors_per_track: 8,
    };
    let desc = describe_geometry(odd);
    // No named form factor, just the arithmetic.
    assert!(!desc.contains('"'), "{desc}");
    assert!(
        desc.starts_with("2 sides × 35 tracks × 8 sectors/track"),
        "{desc}"
    );
}

#[test]
fn describe_hard_disk_lists_partition_sizes() {
    // 4 x 32 MiB partitions, as on a typical MSX-IDE hard disk; sizes use the
    // app-wide decimal humanize_bytes, so 33,554,432 bytes reads as 33.55 MB
    // (matching the per-partition labels in the tree).
    let desc = describe_hard_disk(&[33_554_432; 4]);
    assert_eq!(
        desc,
        "Hard disk image: 4 partitions (33.55 MB, 33.55 MB, 33.55 MB, 33.55 MB)"
    );
    // Singular, and no fabricated floppy geometry ("sides"/"tracks").
    let one = describe_hard_disk(&[16_777_216]);
    assert_eq!(one, "Hard disk image: 1 partition (16.78 MB)");
    assert!(!one.contains("sides") && !one.contains("tracks"));
    // Degenerate: no readable partitions.
    assert_eq!(describe_hard_disk(&[]), "Hard disk image");
}

#[test]
fn is_disk_image_matches_known_extensions_case_insensitively() {
    assert!(is_disk_image(Path::new("GAME.DSK")));
    assert!(is_disk_image(Path::new("game.xsa")));
    assert!(is_disk_image(Path::new("raw.DMK")));
    assert!(is_disk_image(Path::new("/tmp/disk.Di2")));
    assert!(!is_disk_image(Path::new("README.TXT")));
    assert!(!is_disk_image(Path::new("noext")));
    // Tapes are not disk images, but are still openable documents.
    assert!(!is_disk_image(Path::new("tape.cas")));
}

#[test]
fn tape_and_openable_extensions() {
    assert!(is_tape(Path::new("game.cas")));
    assert!(is_tape(Path::new("GAME.TSX")));
    assert!(!is_tape(Path::new("disk.dsk")));
    // Openable covers both disks and tapes.
    assert!(is_openable(Path::new("disk.dmk")));
    assert!(is_openable(Path::new("tape.tsx")));
    assert!(!is_openable(Path::new("notes.txt")));
}

#[test]
fn selection_paths_prefers_marked_set_in_sorted_order() {
    let app = MediaExplorerApp {
        selection: BTreeSet::from(["B.TXT".to_string(), "A.TXT".to_string()]),
        selected: Some("C.TXT".to_string()),
        ..Default::default()
    };
    assert_eq!(app.selection_paths(), vec!["A.TXT", "B.TXT"]);
}

#[test]
fn selection_paths_falls_back_to_viewed_file() {
    let app = MediaExplorerApp {
        selected: Some("ONLY.TXT".to_string()),
        ..Default::default()
    };
    assert_eq!(app.selection_paths(), vec!["ONLY.TXT"]);

    let empty = MediaExplorerApp::default();
    assert!(empty.selection_paths().is_empty());
}

#[test]
fn paths_for_row_targets_only_an_unselected_row() {
    let app = MediaExplorerApp {
        selection: BTreeSet::from(["A.TXT".to_string(), "B.TXT".to_string()]),
        ..Default::default()
    };
    // Right-clicking a row outside the selection acts on that row alone.
    assert_eq!(app.paths_for_row("C.TXT"), vec!["C.TXT"]);
}

#[test]
fn paths_for_row_expands_to_whole_multiselection() {
    let app = MediaExplorerApp {
        selection: BTreeSet::from(["B.TXT".to_string(), "A.TXT".to_string()]),
        ..Default::default()
    };
    // Right-clicking a row inside a multi-selection acts on the whole set.
    assert_eq!(app.paths_for_row("A.TXT"), vec!["A.TXT", "B.TXT"]);
}

#[test]
fn paths_for_row_single_selected_row_acts_on_itself() {
    let app = MediaExplorerApp {
        selection: BTreeSet::from(["A.TXT".to_string()]),
        ..Default::default()
    };
    assert_eq!(app.paths_for_row("A.TXT"), vec!["A.TXT"]);
}
