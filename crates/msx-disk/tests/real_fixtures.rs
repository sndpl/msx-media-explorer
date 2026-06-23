//! Integration tests against real MSX disk/archive fixtures.
//!
//! Fixtures live in the workspace-root `tests/` directory. They are not
//! committed (some are copyrighted game/system disks — see `.gitignore`), so
//! each test skips gracefully when its file is absent. This keeps CI green
//! while giving real-world coverage on developer machines.

use std::path::PathBuf;

use msx_disk::{cas, DiskFs, DiskImage, ImageFormat};

/// Resolve a fixture path under the workspace-root `tests/` directory, or
/// `None` if it does not exist.
fn fixture(name: &str) -> Option<PathBuf> {
    // CARGO_MANIFEST_DIR is crates/msx-disk; fixtures are two levels up.
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

#[test]
fn xsa_decompresses_to_mountable_disk() {
    let path = skip_if_absent!("TWINSAU2.XSA");
    let image = DiskImage::open(&path).expect("open xsa");
    assert_eq!(image.format(), ImageFormat::Xsa);
    // A real MSX disk image decompresses to a standard size.
    assert_eq!(image.data().len(), 737_280);

    let fs = DiskFs::from_image(&image).expect("mount decompressed xsa");
    let tree = fs.tree().expect("tree");
    assert!(!tree.is_empty(), "decompressed disk should list files");
    assert!(
        tree.iter().any(|e| e.name == "AUTOEXEC.BAS"),
        "expected AUTOEXEC.BAS among: {:?}",
        tree.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
}

#[test]
fn msxdos2_disk_lists_subdirectory_and_label() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let fs = DiskFs::from_image(&DiskImage::open(&path).expect("open")).expect("mount");

    assert_eq!(fs.volume_label().as_deref(), Some("DOS2 TOOLS"));

    let tree = fs.tree().expect("tree");
    assert!(tree.iter().any(|e| e.name == "MSXDOS2.SYS"));

    let tools = tree
        .iter()
        .find(|e| e.name == "TOOLS")
        .expect("TOOLS subdirectory");
    assert!(tools.is_dir);
    assert!(!tools.children.is_empty(), "TOOLS should contain tools");
}

#[test]
fn cas_tape_lists_files() {
    let path = skip_if_absent!("Cannon Ball (1983)(Hudson Soft).cas");
    let bytes = std::fs::read(&path).expect("read cas");
    let files = cas::list(&bytes);
    assert!(!files.is_empty(), "tape should contain at least one file");
    assert!(
        files.iter().all(|f| !f.data.is_empty()),
        "each file should have data"
    );
}

#[test]
fn plain_dsk_fixtures_mount_and_read_first_file() {
    for name in ["TOOLS.DSK", "MSX-DOS Hulp (1989)(Philips)(nl).dsk"] {
        let Some(path) = fixture(name) else {
            eprintln!("skipping: fixture '{name}' not present");
            continue;
        };
        let fs = DiskFs::from_image(&DiskImage::open(&path).expect("open")).expect("mount");
        let tree = fs.tree().expect("tree");
        assert!(!tree.is_empty(), "{name} should list files");

        // Reading the first file's bytes should match its reported size.
        if let Some(file) = tree.iter().find(|e| !e.is_dir) {
            let bytes = fs.read_file(&file.path).expect("read file");
            assert_eq!(bytes.len() as u64, file.size, "{} size mismatch", file.path);
        }
    }
}
