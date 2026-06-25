//! Integration tests against real MSX disk/archive fixtures.
//!
//! Fixtures live in the workspace-root `tests/` directory. They are not
//! committed (some are copyrighted game/system disks — see `.gitignore`), so
//! each test skips gracefully when its file is absent. This keeps CI green
//! while giving real-world coverage on developer machines.

use std::path::PathBuf;

use msx_disk::fs::map::{self, SectorKind};
use msx_disk::fs::partition;
use msx_disk::fs::{FatType, Volume};
use msx_disk::{
    cas, fs::write, view::basic, view::disasm, DirEntry, DiskFs, DiskImage, ImageFormat,
};

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
fn detects_dos2_from_real_boot_sector() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let image = DiskImage::open(&path).expect("open");
    let tree = DiskFs::from_image(&image)
        .expect("mount")
        .tree()
        .expect("tree");
    assert_eq!(
        msx_disk::fs::detect_dos_version(image.data(), &tree),
        msx_disk::fs::DosVersion::Dos2,
    );
}

#[test]
fn detokenizes_real_basic_program() {
    let path = skip_if_absent!("TWINSAU2.XSA");
    let fs = DiskFs::from_image(&DiskImage::open(&path).expect("open")).expect("mount");
    let bytes = fs.read_file("AUTOEXEC.BAS").expect("read AUTOEXEC.BAS");
    let listing = basic::detokenize(&bytes, msx_disk::MsxCharset::International);

    assert!(listing.starts_with("10 "), "should start with line 10");
    assert!(listing.contains("DEFINT"), "expected DEFINT keyword");
    assert!(listing.contains("&H"), "expected hex literal");
    assert!(listing.trim_end().ends_with("END"), "should end with END");
}

#[test]
fn disassembles_a_real_com_tool() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let fs = DiskFs::from_image(&DiskImage::open(&path).expect("open")).expect("mount");
    let tree = fs.tree().expect("tree");

    // Pick the first `.COM` executable anywhere on the disk.
    let Some(entry) = tree
        .iter()
        .flat_map(DirEntry::walk)
        .find(|e| !e.is_dir && e.name.to_ascii_uppercase().ends_with(".COM"))
    else {
        eprintln!("skipping: no .COM file on the fixture");
        return;
    };

    let bytes = fs.read_file(&entry.path).expect("read .com");
    let img = disasm::locate(&entry.name, &bytes);
    assert_eq!(img.origin, 0x0100, ".com loads at the MSX-DOS TPA");

    let listing = disasm::disassemble(img.code, img.origin, img.exec);
    assert!(
        listing.contains("\n0100  "),
        "first instruction line should be at 0x0100"
    );
    // A real program decodes to recognizable instructions, not just data.
    assert!(
        listing
            .lines()
            .any(|l| l.contains("CALL ") || l.contains("JP ") || l.contains("LD ")),
        "expected real Z80 instructions in the disassembly of {}",
        entry.name
    );
}

#[test]
fn write_back_to_real_disk_roundtrip() {
    // Add a file to a real MSX-DOS 2 disk (in memory) and confirm it reads back
    // while existing files and the original boot sector are preserved.
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let image = DiskImage::open(&path).expect("open");
    let original = image.data().to_vec();

    let modified = write::add_files(&original, &[("PHASE2.TXT", b"written by mediaexplorer")])
        .expect("add file");

    // Original boot sector (BPB + boot code) is untouched.
    assert_eq!(&modified[..512], &original[..512]);

    let fs = DiskFs::mount(modified).expect("remount");
    assert_eq!(
        fs.read_file("PHASE2.TXT").unwrap(),
        b"written by mediaexplorer"
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
fn sector_map_on_real_disk() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let data = DiskImage::open(&path).expect("open").data().to_vec();

    let disk_map = map::disk_usage(&data).expect("disk usage");
    assert_eq!(disk_map.sector_count, 1440);
    assert_eq!(disk_map.kinds[0], SectorKind::Reserved);
    assert!(disk_map.kinds.contains(&SectorKind::DataUsed));

    // MSXDOS2.SYS occupies several used data sectors.
    let sectors = map::file_sectors(&data, "MSXDOS2.SYS");
    assert!(!sectors.is_empty());
    for s in &sectors {
        assert_eq!(disk_map.kinds[*s], SectorKind::DataUsed);
    }
}

#[test]
fn dmk_normalizes_to_standard_size_and_mounts() {
    // This fixture is a copy-protected bootable game (track 0 deliberately
    // omits sector 8 and the BPB is DOS1 boot-code garbage), so it normalizes
    // to a standard 720KB image and mounts, but has no FAT files to list.
    let path = skip_if_absent!("Ancient Ys Vanished - Omen (1987)(Falcom).dmk");
    let image = DiskImage::open(&path).expect("open dmk");
    assert_eq!(image.format(), ImageFormat::Dmk);
    assert!(!image.is_writable(), "dmk is a read-only container");
    assert_eq!(image.data().len(), 737_280, "snaps to standard 720KB");

    // Mounting succeeds even though the protected disk lists no files.
    let fs = DiskFs::from_image(&image).expect("mount normalized dmk");
    let _ = fs.tree().expect("tree reads without error");
}

#[test]
fn dmk_analyze_reports_real_track_layout() {
    use msx_disk::image::dmk;
    let path = skip_if_absent!("Ancient Ys Vanished - Omen (1987)(Falcom).dmk");
    let bytes = std::fs::read(&path).expect("read dmk");
    let analysis = dmk::analyze(&bytes).expect("analyze");

    assert_eq!(analysis.sides, 2);
    assert_eq!(analysis.tracks, 82);
    assert_eq!(analysis.track_infos.len(), 82 * 2);
    let standard = analysis
        .track_infos
        .iter()
        .filter(|t| t.is_standard())
        .count();
    assert!(
        standard >= 150,
        "expected most tracks standard, got {standard}/164"
    );
}

#[test]
fn single_sided_dmk_with_garbage_bpb_mounts_and_lists_files() {
    // Brainstorm is a single-sided (360KB) game whose DMK header nonetheless
    // declares two sides: all sectors are recorded on head 0, side 1 is
    // unformatted, and the boot sector is a custom loader whose BPB carries a
    // bogus total_sectors. openMSX reads it fine because it serves sectors by
    // their recorded C/H/R and MSX-DOS only ever requests head 0. We must:
    //   1. detect the disk is single-sided from the data, not trust the header,
    //      so the FAT/root dir land at the right LBAs (a 360KB image), and
    //   2. reject the implausibly small BPB so the canonical 360KB BPB is
    //      synthesized.
    let path = skip_if_absent!("Brainstorm (1993)(Syntax Error)(DMK).DMK");
    let image = DiskImage::open(&path).expect("open dmk");
    assert_eq!(image.format(), ImageFormat::Dmk);
    assert_eq!(
        image.data().len(),
        368_640,
        "single-sided disk must normalize to 360KB, not a 720KB interleave"
    );

    let fs = DiskFs::from_image(&image).expect("mount single-sided dmk");
    let tree = fs.tree().expect("tree");
    assert!(
        tree.iter().any(|e| e.name == "AUTOEXEC.BAS"),
        "expected AUTOEXEC.BAS among: {:?}",
        tree.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert!(
        tree.iter().filter(|e| e.name.starts_with("BS.")).count() >= 10,
        "expected the BS.* data files"
    );
}

/// Decode every decodable member of an archive and confirm each one's length
/// and CRC-16 match the values recorded in its header. A CRC match is strong
/// evidence the decompressor is byte-for-byte correct.
fn assert_archive_members_decode(bytes: &[u8]) {
    let members = msx_disk::archive::list(bytes).expect("list archive");
    assert!(!members.is_empty(), "archive should contain members");
    let mut decoded_any = false;
    for (i, m) in members.iter().enumerate() {
        if m.is_directory || !m.decodable {
            continue;
        }
        let data = msx_disk::archive::extract(bytes, i).expect("extract member");
        assert_eq!(
            data.len() as u64,
            m.original_size,
            "size mismatch for member {}",
            m.path
        );
        assert_eq!(
            msx_disk::archive::crc_ok(bytes, i),
            Some(true),
            "crc mismatch for member {} ({})",
            m.path,
            m.method.tag()
        );
        decoded_any = true;
    }
    assert!(decoded_any, "expected at least one decodable member");
}

// PMarc fixtures from Simon Howard's lhasa test suite (ISC), committed in-crate
// so the hand-ported pm1/pm2 decoders are validated on every run.
#[test]
fn pma_pm0_stored_decodes() {
    assert_archive_members_decode(include_bytes!("data/pm0.pma"));
}

// pm1.pma (25 KB output) exceeds every pm1 position threshold, and pm2.pma
// (18 KB output) crosses all pm2 tree-rebuild boundaries (1K/2K/4K/8K), so
// these two cover the full decoder code paths.
#[test]
fn pma_pm1_decodes() {
    assert_archive_members_decode(include_bytes!("data/pm1.pma"));
}

#[test]
fn pma_pm2_decodes() {
    assert_archive_members_decode(include_bytes!("data/pm2.pma"));
}

/// A real-world MSX `.pma` (not from the lhasa suite); skipped when absent.
#[test]
fn real_world_pma_decodes() {
    let path = skip_if_absent!("1250-232.pma");
    let bytes = std::fs::read(&path).expect("read pma");
    assert_archive_members_decode(&bytes);
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
fn tsx_tape_parses_blocks_and_files() {
    use msx_disk::tape::{self, TapeBlock, TapeFormat};
    let path = skip_if_absent!(
        "Album de Platino (1987)(Codemasters, SERMA)(Tape 2 - BMX Simulator)(ES)(en)[!][RUN'CAS-'][v0.8.5b].tsx"
    );
    let bytes = std::fs::read(&path).expect("read tsx");
    let t = tape::open(&bytes, TapeFormat::Tsx);

    // Custom-info ripper note and an archive-info block with the title.
    assert!(t.blocks.iter().any(|b| matches!(
        b,
        TapeBlock::CustomInfo { id, .. } if id == "TSX.RIPPER"
    )));
    let title = t.blocks.iter().find_map(|b| match b {
        TapeBlock::ArchiveInfo(pairs) => pairs.iter().find(|(f, _)| f == "Title").map(|(_, v)| v),
        _ => None,
    });
    assert_eq!(title.map(String::as_str), Some("Album de Platino"));

    // The #4B blocks yield logical files, the first an ASCII loader named BMX.
    let files = t.files();
    assert!(
        files.len() >= 3,
        "expected several files, got {}",
        files.len()
    );
    assert_eq!(files[0].name, "BMX");
    assert_eq!(files[0].kind, msx_disk::cas::CasFileKind::Ascii);
}

#[test]
fn hd_image_is_partitioned_into_four_fat12_volumes() {
    // The openMSX MSX_IDE hard-disk image is an MBR with four FAT12 partitions.
    // It proves the whole HD path end to end: partition detection, the
    // MSX-correct FAT12 detection (where fatfs would wrongly pick FAT16 and
    // corrupt reads), directory listing, and a length-correct file read.
    let path = skip_if_absent!("hd.dsk");
    let image = DiskImage::open(&path).expect("open hd.dsk");
    let data = image.data();

    assert!(
        partition::is_partitioned(data),
        "hd.dsk should be partitioned"
    );

    let parts = partition::parse_partition_table(data).expect("partition table");
    assert_eq!(parts.len(), 4, "expected four partitions");
    // Entries are stored in reverse physical order; sorting yields LBA 1 first.
    assert_eq!(parts[0].lba_start, 1, "first partition starts at LBA 1");

    let mut read_a_subdir_file = false;
    for entry in &parts {
        let vol = Volume::from_partition(&image, entry).expect("mount partition");
        assert_eq!(
            vol.fat_type(),
            FatType::Fat12,
            "partition at LBA {} must be FAT12",
            entry.lba_start
        );

        let tree = vol.tree();
        assert!(
            !tree.is_empty(),
            "partition at LBA {} should list files",
            entry.lba_start
        );

        // Read a file and confirm its byte length matches its directory size.
        // Prefer one inside a subdirectory: that is the case fatfs-as-FAT16
        // would corrupt (wrong cluster math on the directory's own chain).
        for file in tree.iter().flat_map(|e| e.walk()) {
            if file.is_dir {
                continue;
            }
            let bytes = vol.read_file(&file.path).expect("read file");
            assert_eq!(
                bytes.len() as u64,
                file.size,
                "size mismatch for {} in LBA {}",
                file.path,
                entry.lba_start
            );
            if file.path.contains('/') {
                read_a_subdir_file = true;
            }
        }
    }
    assert!(
        read_a_subdir_file,
        "expected at least one file inside a subdirectory across the partitions"
    );
}

#[test]
fn dark_castle_screen5_pics_decode_when_forced() {
    use msx_disk::recoil::{self, ImageFormat, NoCompanions};
    let path = skip_if_absent!("Dark Castle.dsk");
    let fs = DiskFs::from_image(&DiskImage::open(&path).expect("open")).expect("mount");

    let read = |name: &str| {
        fs.tree()
            .expect("tree")
            .iter()
            .flat_map(|e| e.walk())
            .find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(name))
            .map(|e| fs.read_file(&e.path).expect("read"))
    };

    // WAKU.PIC is a full-page SCREEN 5 image (VRAM start 0): the strict path.
    let waku = read("WAKU.PIC").expect("WAKU.PIC present");
    let img = recoil::decode_as(ImageFormat::Screen5, &waku, &NoCompanions).expect("WAKU decodes");
    assert_eq!((img.width, img.height), (256, 212));

    // MOJI.PIC is SCREEN 5 data BLOAD'd to VRAM 0x6100; it must render via the
    // offset rebuild, not be rejected.
    let moji = read("MOJI.PIC").expect("MOJI.PIC present");
    let img = recoil::decode_as(ImageFormat::Screen5, &moji, &NoCompanions).expect("MOJI decodes");
    assert_eq!((img.width, img.height), (256, 212));
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

#[test]
fn msxdos2_disk_stats_are_sane_and_clean() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let image = DiskImage::open(&path).expect("open");
    let data = image.data();

    let stats = msx_disk::stats::disk_stats(data, 5).expect("stats");
    assert!(stats.file_count > 0, "a real system disk has files");
    assert!(stats.dir_count > 0, "it has the TOOLS subdirectory");
    // Used + free is consistent and bounded by the total.
    assert!(stats.used_bytes + stats.free_bytes <= stats.total_bytes);
    // The free figure must agree with the standalone geometry helper.
    let geo = map::fs_geometry(data).expect("geometry");
    assert_eq!(stats.free_bytes, geo.free_bytes());
    // A well-formed system disk has no lost/cross-linked/bad clusters.
    assert!(
        stats.integrity.is_clean(),
        "unexpected integrity findings: {:?}",
        stats.integrity
    );
    // The largest-files list is capped and sorted.
    assert!(stats.largest_files.len() <= 5);
    assert!(stats
        .largest_files
        .windows(2)
        .all(|w| w[0].size >= w[1].size));
}

#[test]
fn checksums_match_manual_hash_of_file_bytes() {
    let path = skip_if_absent!("MSX-DOS2 TOOLS.dsk");
    let fs = DiskFs::from_image(&DiskImage::open(&path).expect("open")).expect("mount");
    let tree = fs.tree().expect("tree");
    let file = tree
        .iter()
        .flat_map(DirEntry::walk)
        .find(|e| !e.is_dir)
        .expect("a file");

    // file_checksums must equal hashing the read-back bytes directly.
    let bytes = fs.read_file(&file.path).expect("read");
    assert_eq!(
        msx_disk::verify::file_checksums(&fs, &file.path).expect("checksums"),
        msx_disk::Checksums::of(&bytes)
    );
}

#[test]
fn hd_partitions_report_per_volume_stats() {
    // Proves the stats/integrity analysis works on the read-only HD path, which
    // the whole-image `disk_stats` cannot reach.
    let path = skip_if_absent!("hd.dsk");
    let image = DiskImage::open(&path).expect("open hd.dsk");
    let parts = partition::parse_partition_table(image.data()).expect("partition table");

    for entry in &parts {
        let vol = Volume::from_partition(&image, entry).expect("mount partition");
        let stats = vol.stats(5);
        let geo = vol.fs_geometry();
        assert_eq!(stats.free_bytes, geo.free_bytes());
        assert!(stats.used_bytes + stats.free_bytes <= stats.total_bytes);
        // A per-file checksum resolves for at least the first file.
        if let Some(file) = vol.tree().iter().flat_map(|e| e.walk()).find(|e| !e.is_dir) {
            assert!(vol.file_checksums(&file.path).is_some());
        }
    }
}
