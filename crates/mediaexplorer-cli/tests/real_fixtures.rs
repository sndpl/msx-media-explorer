//! CLI runs against real MSX disk images in the workspace-root `tests/`
//! directory. The fixtures are not committed (see `.gitignore`), so every test
//! skips gracefully when its file is absent — same pattern as
//! `crates/msx-disk/tests/real_fixtures.rs`.

use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> Option<PathBuf> {
    // CARGO_MANIFEST_DIR is crates/mediaexplorer-cli; fixtures are two levels up.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests")
        .join(name);
    path.exists().then_some(path)
}

macro_rules! skip_if_absent {
    ($name:expr) => {
        match fixture($name) {
            Some(p) => p,
            None => {
                eprintln!("skipping: fixture '{}' not present", $name);
                return;
            }
        }
    };
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mediaexplorer-cli"))
        .args(args)
        .output()
        .expect("binary runs")
}

#[test]
fn ls_lists_a_real_xsa_image() {
    let path = skip_if_absent!("TWINSAU2.XSA");
    let out = run(&["ls", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(listing.contains("AUTOEXEC.BAS"), "{listing}");
}

#[test]
fn mutating_a_real_dmk_image_is_refused() {
    // .dmk raw-track containers stay read-only (unlike .xsa, which recompresses
    // in place). The refusal happens before any write, so the fixture is safe.
    let path = skip_if_absent!("Brainstorm (1993)(Syntax Error)(DMK).DMK");
    let out = run(&["rm", path.to_str().unwrap(), "ANYTHING.TXT"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("read-only"), "{err}");
}

#[test]
fn converts_a_real_dmk_to_dsk() {
    let path = skip_if_absent!("Brainstorm (1993)(Syntax Error)(DMK).DMK");
    let dir = std::env::temp_dir().join(format!("mecli-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out_path = dir.join("brainstorm.dsk");

    let out = run(&[
        "convert",
        path.to_str().unwrap(),
        out_path.to_str().unwrap(),
        "--force",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The converted raw disk lists with the standard tooling.
    let ls = run(&["ls", out_path.to_str().unwrap(), "-l"]);
    assert!(
        ls.status.success(),
        "{}",
        String::from_utf8_lossy(&ls.stderr)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn extracts_a_file_from_a_real_dsk() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let out = run(&["ls", path.to_str().unwrap(), "-l"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(listing.contains("boot: MSX-DOS 2"), "{listing}");
}
