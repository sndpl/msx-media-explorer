//! End-to-end test of the compiled binary: a full disk-editing round trip in a
//! temp directory, exercising arg parsing, exit codes, and the I/O shell that
//! the unit tests do not touch.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_mediaexplorer-cli")
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("binary runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(
        out.status.success(),
        "expected success for {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

fn fails(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for {args:?}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8(out.stderr).expect("utf8 stderr")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("mecli-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn full_round_trip_new_add_ls_extract_mv_rm() {
    let tmp = TempDir::new("roundtrip");
    let dir = &tmp.0;
    std::fs::write(dir.join("hello.bas"), b"10 PRINT\"HI\"\r\n20 END\r\n").unwrap();

    ok(dir, &["new", "game.dsk", "--format", "720"]);
    assert_eq!(
        std::fs::metadata(dir.join("game.dsk")).unwrap().len(),
        737_280
    );

    // A fresh disk defaults to a DOS 2 boot sector.
    let boot = ok(dir, &["bootsector", "game.dsk"]);
    assert!(boot.contains("MSX-DOS 2"), "{boot}");

    ok(dir, &["mkdir", "game.dsk", "-p", "UTILS/SUB"]);
    ok(dir, &["add", "game.dsk", "hello.bas"]);
    ok(
        dir,
        &[
            "add",
            "game.dsk",
            "hello.bas",
            "--dest",
            "UTILS",
            "--as",
            "COPY.BAS",
        ],
    );

    let listing = ok(dir, &["ls", "game.dsk", "-R"]);
    assert!(listing.contains("HELLO.BAS"), "{listing}");
    assert!(listing.contains("COPY.BAS"), "{listing}");

    // Overwrite is refused without --force, accepted with it.
    let err = fails(dir, &["add", "game.dsk", "hello.bas"]);
    assert!(err.contains("--force"), "{err}");
    ok(dir, &["add", "game.dsk", "hello.bas", "--force"]);

    // Extract everything and byte-compare the round trip.
    ok(dir, &["extract", "game.dsk", "--out", "dump"]);
    let original = std::fs::read(dir.join("hello.bas")).unwrap();
    assert_eq!(std::fs::read(dir.join("dump/HELLO.BAS")).unwrap(), original);
    assert_eq!(
        std::fs::read(dir.join("dump/UTILS/COPY.BAS")).unwrap(),
        original
    );

    ok(dir, &["mv", "game.dsk", "HELLO.BAS", "RENAMED.BAS"]);
    ok(dir, &["rm", "game.dsk", "RENAMED.BAS"]);
    ok(dir, &["rm", "game.dsk", "-r", "UTILS"]);
    let listing = ok(dir, &["ls", "game.dsk"]);
    assert_eq!(listing, "", "disk should be empty again");
}

#[test]
fn xsa_conversion_round_trips_and_edits_in_place() {
    let tmp = TempDir::new("xsa");
    let dir = &tmp.0;
    std::fs::write(dir.join("data.txt"), b"payload").unwrap();
    std::fs::write(dir.join("extra.txt"), b"more").unwrap();

    ok(dir, &["new", "disk.dsk", "--dos", "1"]);
    ok(dir, &["add", "disk.dsk", "data.txt"]);
    ok(dir, &["convert", "disk.dsk", "disk.xsa"]);
    assert!(
        std::fs::metadata(dir.join("disk.xsa")).unwrap().len()
            < std::fs::metadata(dir.join("disk.dsk")).unwrap().len(),
        "xsa must be smaller than the raw disk"
    );

    // The compressed image lists like any other...
    let listing = ok(dir, &["ls", "disk.xsa"]);
    assert!(listing.contains("DATA.TXT"), "{listing}");

    // ...converting back yields the identical raw disk...
    ok(dir, &["convert", "disk.xsa", "back.dsk"]);
    assert_eq!(
        std::fs::read(dir.join("back.dsk")).unwrap(),
        std::fs::read(dir.join("disk.dsk")).unwrap()
    );

    // ...and edits are recompressed in place: the .xsa stays an .xsa (magic
    // header) and reflects both the removal and the addition.
    ok(dir, &["rm", "disk.xsa", "DATA.TXT"]);
    ok(dir, &["add", "disk.xsa", "extra.txt"]);
    let header = &std::fs::read(dir.join("disk.xsa")).unwrap()[..4];
    assert_eq!(header, b"PCK\x08", "still a compressed XSA container");
    let listing = ok(dir, &["ls", "disk.xsa"]);
    assert!(listing.contains("EXTRA.TXT"), "{listing}");
    assert!(!listing.contains("DATA.TXT"), "{listing}");
}

#[test]
fn single_letter_aliases_drive_every_command() {
    let tmp = TempDir::new("aliases");
    let dir = &tmp.0;
    std::fs::write(dir.join("f.txt"), b"hi").unwrap();

    // new -> n, bootsector -> b
    ok(dir, &["n", "d.dsk", "--format", "720"]);
    assert!(ok(dir, &["b", "d.dsk"]).contains("MSX-DOS"));
    // mkdir -> d, add -> a
    ok(dir, &["d", "d.dsk", "DIR"]);
    ok(dir, &["a", "d.dsk", "f.txt", "--dest", "DIR"]);
    // ls -> l
    assert!(ok(dir, &["l", "d.dsk", "-R"]).contains("F.TXT"));
    // extract -> e
    ok(dir, &["e", "d.dsk", "DIR/F.TXT", "--out", "out"]);
    assert_eq!(std::fs::read(dir.join("out/F.TXT")).unwrap(), b"hi");
    // mv -> m, rm -> r
    ok(dir, &["m", "d.dsk", "DIR/F.TXT", "DIR/G.TXT"]);
    ok(dir, &["r", "d.dsk", "-r", "DIR"]);
    assert_eq!(ok(dir, &["l", "d.dsk"]), "", "disk should be empty again");
    // convert -> c
    ok(dir, &["c", "d.dsk", "d.xsa"]);
    assert!(std::fs::metadata(dir.join("d.xsa")).is_ok());
}

#[test]
fn sav_conversion_round_trips_and_edits_in_place() {
    let tmp = TempDir::new("sav");
    let dir = &tmp.0;
    std::fs::write(dir.join("data.txt"), b"payload").unwrap();
    std::fs::write(dir.join("extra.txt"), b"more").unwrap();

    // dsk -> sav: the MSXPLAYer journal is sparse, and lists like any disk.
    ok(dir, &["new", "disk.dsk"]);
    ok(dir, &["add", "disk.dsk", "data.txt"]);
    ok(dir, &["convert", "disk.dsk", "disk.sav"]);
    assert!(
        std::fs::metadata(dir.join("disk.sav")).unwrap().len()
            < std::fs::metadata(dir.join("disk.dsk")).unwrap().len(),
        "a mostly-empty journal must be smaller than the raw disk"
    );
    let listing = ok(dir, &["ls", "disk.sav"]);
    assert!(listing.contains("DATA.TXT"), "{listing}");

    // sav -> dsk restores the identical raw disk (the journal keeps the real
    // boot sector because it differs from the synthetic BPB stub).
    ok(dir, &["convert", "disk.sav", "back.dsk"]);
    assert_eq!(
        std::fs::read(dir.join("back.dsk")).unwrap(),
        std::fs::read(dir.join("disk.dsk")).unwrap()
    );

    // Edits re-journal in place.
    ok(dir, &["rm", "disk.sav", "DATA.TXT"]);
    ok(dir, &["add", "disk.sav", "extra.txt"]);
    let listing = ok(dir, &["ls", "disk.sav"]);
    assert!(listing.contains("EXTRA.TXT"), "{listing}");
    assert!(!listing.contains("DATA.TXT"), "{listing}");
}

#[test]
fn bootsector_install_upgrades_dos1_to_dos2() {
    let tmp = TempDir::new("boot");
    let dir = &tmp.0;
    ok(dir, &["new", "d.dsk", "--dos", "1"]);
    assert!(ok(dir, &["bootsector", "d.dsk"]).contains("MSX-DOS 1"));
    let after = ok(dir, &["bootsector", "d.dsk", "--dos", "2"]);
    assert!(after.contains("MSX-DOS 2 (volume serial"), "{after}");
}

#[test]
fn guards_report_clean_errors() {
    let tmp = TempDir::new("guards");
    let dir = &tmp.0;

    // new: refuses to clobber and rejects non-dsk containers.
    ok(dir, &["new", "d.dsk"]);
    assert!(fails(dir, &["new", "d.dsk"]).contains("--force"));
    assert!(fails(dir, &["new", "d.xsa"]).contains(".dsk extension"));

    // Missing image file.
    assert!(!fails(dir, &["ls", "missing.dsk"]).is_empty());

    // Usage errors come from clap with exit code 2.
    let out = run(dir, &["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
}
