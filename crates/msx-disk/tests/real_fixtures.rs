//! Integration tests against real MSX disk/archive fixtures.
//!
//! Fixtures live in the workspace-root `tests/` directory. They are not
//! committed (some are copyrighted game/system disks — see `.gitignore`), so
//! each test skips gracefully when its file is absent. This keeps CI green
//! while giving real-world coverage on developer machines.

use std::path::PathBuf;

use msx_disk::{cas, fs::write, view::basic, DiskFs, DiskImage, ImageFormat};

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
fn detokenizes_real_basic_program() {
    let path = skip_if_absent!("TWINSAU2.XSA");
    let fs = DiskFs::from_image(&DiskImage::open(&path).expect("open")).expect("mount");
    let bytes = fs.read_file("AUTOEXEC.BAS").expect("read AUTOEXEC.BAS");
    let listing = basic::detokenize(&bytes);

    assert!(listing.starts_with("10 "), "should start with line 10");
    assert!(listing.contains("DEFINT"), "expected DEFINT keyword");
    assert!(listing.contains("&H"), "expected hex literal");
    assert!(listing.trim_end().ends_with("END"), "should end with END");
}

#[test]
fn write_back_to_real_disk_roundtrip() {
    // Add a file to a real MSX-DOS 2 disk (in memory) and confirm it reads back
    // while existing files and the original boot sector are preserved.
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let image = DiskImage::open(&path).expect("open");
    let original = image.data().to_vec();

    let modified = write::add_files(&original, &[("PHASE2.TXT", b"written by dskexplorer")])
        .expect("add file");

    // Original boot sector (BPB + boot code) is untouched.
    assert_eq!(&modified[..512], &original[..512]);

    let fs = DiskFs::mount(modified).expect("remount");
    assert_eq!(
        fs.read_file("PHASE2.TXT").unwrap(),
        b"written by dskexplorer"
    );
    // An existing file still reads correctly.
    assert!(!fs.read_file("MSXDOS2.SYS").unwrap().is_empty());
}

#[test]
fn compress_real_disk_to_xsa_and_reopen() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let original = DiskImage::open(&path).expect("open").data().to_vec();

    let xsa = msx_disk::image::xsa::compress(&original);
    assert!(xsa.starts_with(b"PCK\x08"));
    assert!(xsa.len() < original.len(), "should compress");

    // Reopen through the public container path and confirm an exact match.
    let reopened = DiskImage::open_bytes(ImageFormat::Xsa, xsa).expect("reopen xsa");
    assert_eq!(reopened.data(), &original[..]);

    let fs = DiskFs::from_image(&reopened).expect("mount");
    assert!(fs.tree().unwrap().iter().any(|e| e.name == "MSXDOS2.SYS"));
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
