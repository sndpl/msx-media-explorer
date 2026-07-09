# MSX Media Explorer

A cross-platform (Linux, Windows, macOS) desktop tool for browsing, inspecting,
and converting vintage **MSX floppy disk images**. A modern, open-source
successor to the classic Windows-only DskExplorer.

## Features

### Opening media

- Reads every common MSX floppy image format (see the table below), plus
  raw-track `.dmk` disks, openMSX multi-partition hard-disk images, and `.cas` /
  `.tsx` cassette tapes.
- Drop a disk or tape image on the window to open it.
- Cross-platform: Linux, Windows 10/11 (64-bit), and macOS.

### Browsing

- File tree with MSX-DOS 2 subdirectory nesting.
- Multi-partition hard disks (openMSX `MSX_IDE`, up to four FAT12/FAT16
  partitions): each partition is a top-level node ("Partition n — FAT12/FAT16,
  size, label") you expand and browse like a floppy. Partitions are read-only,
  and the disk-usage Map is hidden for them. The FAT width is read from each
  partition's own boot sector with an MSX-correct rule (not the
  cluster-count-only heuristic), so FAT12 partitions just above the FAT16
  cluster threshold are read without corruption.
- Tapes open as a list of files, each viewable and extractable.
- Cmd/Ctrl-click to mark multiple files for batch extract or delete.
- Status bar shows the physical disk type (e.g. 3.5" Double Sided, Double
  Density 2DD) and the BPB-derived filesystem facts (size, clusters, sectors per
  cluster, bytes per sector, total sectors, volume label).

### Viewing file contents

The viewer offers tabs that adapt to the selected file:

- **Info** — content-derived facts: a description of the file type, the
  BSAVE/BLOAD header when present, and graphics- or music-format details.
- **Hex** — hex dump with an ASCII column and selectable bytes-per-row, plus
  byte selection (click / shift-click / drag), copy of the selected bytes as hex
  or ASCII, go-to-offset, named bookmarks, and a data inspector that reads the
  bytes at the cursor as u8/i8/u16/i16/u24/u32/hex/binary and decodes MSX
  structures (BSAVE header, boot-sector BPB, FCB).
- **Text** — decoded text, with an option to show control characters.
- **BASIC** — detokenizes a tokenized MSX-BASIC program into a listing.
- **Screen** — renders MSX graphics (see below).
- **Archive** — lists and extracts the contents of `.lzh`/`.lha`/`.lzs`/`.pma`
  archives stored inside the image.
- Find within a file (text or hex), jumping between matches.

### File metadata (Info pane)

- One-line descriptions for a large table of known MSX extensions (disk/tape
  images, BASIC, machine code, graphics, music, archives, and more).
- Parses the 7-byte BSAVE/BLOAD header (start / end / exec addresses).
- Graphics-format specifics for recognized image extensions.
- Music/tracker metadata (title, author, channels, positions, subsongs) for
  MoonBlaster (`.mbm`/`.mwm`/`.mfm`), FAC SoundTracker (`.mus`), Tyfoon
  Pro-Tracker (`.pro`), SCC Blaffer NT (`.sbm`), SCC-Musixx (`.sng`),
  ProTracker 3 / Vortex Tracker II (`.pt3`), Amiga modules (`.mod`), Standard
  MIDI (`.mid`), and E-Tracker (`.etc`/`.cop`/`.saa`).
- CRC32 and SHA-1 checksums of the file, to match a dump against software
  databases (TOSEC, Generation MSX, openMSX's software DB).

### Graphics viewer

- The full RECOIL-derived MSX/MSX2/MSX2+/V9990 format set — SCREEN 2-12, Graph
  Saurus (SR*), GL/SH + `.PLx` palettes, `.Sxx` interlace, YJK/YAE, V9990
  `.G9B`, Dynamic Publisher (PCT/FNT/STP), DD-Graph (CMP), MSX Interchange
  (MIF/MIG), Maki-chan (MAG/MKI/MAX), and PI/PIC.
- Companion palette/interlace files are read from the same disk automatically.
- A format picker lets you force a decoder when a file's extension is unknown or
  its bytes don't decode under it.

### Character-set decoding

- Decodes MSX single-byte filenames and text through the correct regional code
  page: **International** (Western), **Japanese** (kana / hiragana),
  **Russian**, **Korean**, **Arabic**, or **Brazilian**.
- The charset is auto-detected from the disk's filenames on load; the
  **Text Encoding** menu pins a different one, re-decoding filenames, text,
  BASIC listings, and the hex ASCII column live.

### Editing (write-back to the source image)

- Add, rename, and delete files (single or batch).
- Edit a file's bytes directly in hex.
- Drop files onto an open, writable disk to add them.
- The original boot sector is always preserved, so MSX-DOS 1 boot code is never
  clobbered.

### Disk-level tools

- **Sectors** — a "view disk by sector" hex view with in-place sector editing
  and disk-wide search (text or hex, jump between matches).
- **Map** — a graphical disk-usage map (reserved / FAT / root / used / free)
  that outlines the selected file's sectors; click a cell to open that sector.
- **Stats** — whole-disk insight: used/free split, file and directory counts, a
  file-type breakdown, the largest files, fragmentation, a FAT-chain integrity
  check (lost / cross-linked / out-of-range clusters), and CRC32/SHA-1 of the
  whole image. Works for floppies and shows per-partition figures for hard-disk
  images.
- **Analyze** (`.dmk`) — lists every track's sectors, density, and CRC status,
  so non-standard or copy-protected tracks are visible.
- **Blocks** (tapes) — shows every tape block, including TSX `#35` custom-info,
  `#32` archive-info, and `#4B` MSX blocks with their detected
  ASCII/BINARY/BASIC headers.

### Conversion & creation

- Create a new blank disk (360 KB single-sided or 720 KB double-sided).
- Save As to convert any open image to `.dsk` or `.xsa` (full XSA compressor +
  decompressor).

### Clipboard & export

- Copy the current view to the OS clipboard (hex dump / text / BASIC listing as
  text, or the decoded screen image as a bitmap).
- Save the viewed MSX image as a PNG.

### Drag and drop

- Drag a disk/tape image in to open it, or other files onto an open disk to add
  them.
- On macOS and Windows, drag files out of the list into the OS file manager.

### Copying files between two disks

- **File → New Window** opens a second, independent window. Open a disk in
  each, then (macOS/Windows) drag files from one window's file tree straight
  into a folder in the other.
- On Linux (no native drag-out): extract the files to a host folder from the
  first window, then drop them onto the second.
- Known limitations: don't open the *same* image file in both windows for
  editing (each write saves the whole image — the last save wins), and file
  timestamps/attributes are not preserved by a copy (the destination stamps
  the current time, as with any added file).

## Command-line tool (`mediaexplorer-cli`)

A headless companion binary built on the same `msx-disk` core, for scripting
and batch work. It edits floppy images in any writable container
(`.dsk`-family, `.img`, `.msx`, `.ddi`, and `.xsa`, which is recompressed on
save); raw-track `.dmk` containers and partitioned hard-disk images can be
listed, extracted, and converted, but not modified in place.

| Command | What it does |
|---------|--------------|
| `ls` | List contents (`-l` long format with volume/DOS header, `-R` recursive) |
| `add` | Add or update host files (`--dest DIR`, `--as NAME`, `--force`) |
| `extract` | Copy a file, a directory (`-R`), or the whole image to the host |
| `rm` / `mv` / `mkdir` | Remove (`-r` recursive), rename/move, create directories (`-p`) |
| `new` | Create a blank formatted disk (`--format 720\|360ss\|360ds\|180`, `--dos 1\|2`) |
| `bootsector` | Show the MSX-DOS generation, or install a DOS 1/2 boot sector (`--dos`) |
| `convert` | Re-container an image: anything readable → `.dsk`, `.xsa`, or MSXPLAYer `.sav` |

```sh
mediaexplorer-cli new game.dsk --format 720     # blank disk, MSX-DOS 2 boot sector
mediaexplorer-cli mkdir game.dsk -p GAMES/RPG
mediaexplorer-cli add game.dsk hello.bas --dest GAMES
mediaexplorer-cli ls game.dsk -l -R
mediaexplorer-cli extract game.dsk GAMES -R --out ./dump
mediaexplorer-cli bootsector old.dsk --dos 2    # FIXDISK-style upgrade
mediaexplorer-cli convert image.xsa image.dsk   # also .dsk -> .xsa
```

Notes:

- `add`, `mv`, `new`, and `convert` refuse to overwrite without `--force`, and
  every image write is atomic (temp file + rename) so a crash never leaves a
  half-written disk.
- Filenames with bytes ≥ 0x80 (kana, accented Latin) are decoded/encoded via
  `--charset`; by default the charset is auto-detected per disk, like the GUI.
- `new` and `bootsector` install real MSX-DOS boot sectors (ported from
  openMSX). A DOS 2 boot sector carries the `VOL_ID` marker and a volume
  serial, which is what makes MSX-DOS 2's `UNDEL` and disk cache work. A
  *bootable system disk* additionally needs `MSXDOS(2).SYS` /
  `COMMAND(2).COM`, which are copyrighted and never embedded. See
  [docs/msx-boot-sectors.md](docs/msx-boot-sectors.md) for a full reference
  on MSX boot sectors.
- Exit codes: `0` success, `1` operational error, `2` usage error.

### Getting and running the CLI

Every GitHub release ships a prebuilt binary per platform. Download it, then:

```sh
# Linux / macOS: make it executable and put it on your PATH under a short name
chmod +x mediaexplorer-cli-x86_64-unknown-linux-gnu   # or -macos-universal
mv mediaexplorer-cli-* ~/.local/bin/mediaexplorer-cli
mediaexplorer-cli --help

# Windows: rename mediaexplorer-cli-x86_64-pc-windows-msvc.exe and run it
# from any terminal; no installer or dependencies needed.
```

macOS note: like the GUI, the binary is not notarized, so the first run may be
blocked by Gatekeeper — clear it with
`xattr -d com.apple.quarantine mediaexplorer-cli`.

Or build and run from source:

```sh
cargo run -p mediaexplorer-cli -- ls image.dsk   # run straight from the repo
cargo build --release -p mediaexplorer-cli       # -> target/release/mediaexplorer-cli
```

Every subcommand documents itself: `mediaexplorer-cli <command> --help`.

## Supported image formats (read)

| Format | Notes |
|--------|-------|
| `.dsk`, `.di1`, `.ds1`, `.di2`, `.ds2` | Raw sector dumps (360KB / 720KB) |
| `.img` | Raw with a leading side-count byte |
| `.msx` | 720KB, cylinder-interleaved sides |
| `.ddi` | DiskDupe image (header + raw) |
| `.xsa` | Compressed disk image (decompressed on open, recompressed on save) |
| `.dmk` | David Keil raw-track image (read-only; normalized + analyzed) |
| `.sav` | MSXPLAYer virtual floppy: a sector diff journal replayed onto an empty 720KB disk (boot sector synthesized when absent; re-journaled on save) |
| `.dsk` (hard disk) | openMSX `MSX_IDE` multi-partition image (read-only; FAT12/FAT16) |
| `.cas` | MSX cassette tape image (files + block overview) |
| `.tsx` | MSX tape image (TZX 1.21 with `#4B` Kansas City blocks) |

## Architecture

A Cargo workspace with three crates:

- **`msx-disk`** — headless core library: image-format parsing, FAT12/FAT16
  filesystem (floppies via `fatfs`, hard-disk partitions via an MSX-aware
  reader), file viewers (hex/text/BASIC/screen), and search. No UI
  dependencies, fully unit-tested.
- **`mediaexplorer-gui`** — the [egui](https://github.com/emilk/egui) /
  `eframe` desktop application that renders the models produced by `msx-disk`.
- **`mediaexplorer-cli`** — the command-line companion (see above), a thin
  shell over the same `msx-disk` primitives.

## Building

```sh
cargo build --workspace
cargo run -p mediaexplorer-gui                 # the desktop app
cargo run -p mediaexplorer-cli -- ls image.dsk # the command-line tool
cargo test --workspace
```

### Packaging a macOS app

`cargo run` and the bare release binary work fine, but double-clicking a bare
Unix executable in Finder opens it in Terminal. To get a real, double-clickable
`MSX Media Explorer.app` (and a `.dmg`), use
[cargo-packager](https://github.com/crabnebula-dev/cargo-packager):

```sh
cargo install cargo-packager --locked
cargo build --release -p mediaexplorer-gui
cargo packager --release -p mediaexplorer-gui --formats app --out-dir dist
# Seal the bundle with an ad-hoc signature so it passes strict verification:
codesign --force --deep --sign - "dist/MSX Media Explorer.app"
```

The bundle config lives in `crates/mediaexplorer-gui/Cargo.toml` under
`[package.metadata.packager]`; the icon is `crates/mediaexplorer-gui/assets/icons/`
(a placeholder — replace `icon.icns`/`icon.png` with real artwork later). The
tagged release workflow builds a universal (arm64 + x86_64) `.app` + `.dmg`
automatically.

The release `.dmg` is **ad-hoc signed, not notarized** (no paid Apple Developer
account). It runs, but on first launch Gatekeeper flags it as coming from an
unidentified developer: right-click the app and choose **Open** once, or run
`xattr -dr com.apple.quarantine "/Applications/MSX Media Explorer.app"`.

Note for dev builds: macOS 26 ("Tahoe") ignores the runtime Dock-icon API for
bare executables, so `cargo run` shows the generic exec icon in the Dock. Run
`scripts/macos-dev-app.sh` instead — it wraps the release binary in a minimal
`.app` so the Dock shows the real icon (optionally pass an image path to open).

## License

GPL-2.0-or-later. See [COPYING](COPYING).

The MSX/MSX2/MSX2+/V9990 graphics-format decoders are ported from
[RECOIL](https://recoil.sourceforge.net/) by Piotr Fusik, which is GPLv2+;
this is why the project is GPL-licensed.

The `.xsa` support implements the XelaSoft Archive format — this project uses
XSA code developed by Alex Wulms/XelaSoft (ported via openMSX's extractor).
The MSX-DOS 1/2 boot blocks installed by `new`/`bootsector` are ported from
[openMSX](https://openmsx.org/) (GPL-2.0).
