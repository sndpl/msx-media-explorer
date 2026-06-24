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
- Cross-platform: Linux, Windows, and macOS.

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
- **Hex** — hex dump with an ASCII column and selectable bytes-per-row.
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
  page: **International** (Western) or **Japanese** (kana / hiragana).
- The charset is auto-detected from the disk's filenames on load, with a
  dropdown to pin a different one; the choice re-decodes filenames, text, BASIC
  listings, and the hex ASCII column live.

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

## Supported image formats (read)

| Format | Notes |
|--------|-------|
| `.dsk`, `.di1`, `.ds1`, `.di2`, `.ds2` | Raw sector dumps (360KB / 720KB) |
| `.img` | Raw with a leading side-count byte |
| `.msx` | 720KB, cylinder-interleaved sides |
| `.ddi` | DiskDupe image (header + raw) |
| `.xsa` | Compressed disk image (decompressed on open) |
| `.dmk` | David Keil raw-track image (read-only; normalized + analyzed) |
| `.dsk` (hard disk) | openMSX `MSX_IDE` multi-partition image (read-only; FAT12/FAT16) |
| `.cas` | MSX cassette tape image (files + block overview) |
| `.tsx` | MSX tape image (TZX 1.21 with `#4B` Kansas City blocks) |

## Architecture

A Cargo workspace with two crates:

- **`msx-disk`** — headless core library: image-format parsing, FAT12/FAT16
  filesystem (floppies via `fatfs`, hard-disk partitions via an MSX-aware
  reader), file viewers (hex/text/BASIC/screen), and search. No UI
  dependencies, fully unit-tested.
- **`mediaexplorer-gui`** — the [egui](https://github.com/emilk/egui) /
  `eframe` desktop application that renders the models produced by `msx-disk`.

## Building

```sh
cargo build --workspace
cargo run -p mediaexplorer-gui
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

## License

GPL-2.0-or-later. See [COPYING](COPYING).

The MSX/MSX2/MSX2+/V9990 graphics-format decoders are ported from
[RECOIL](https://recoil.sourceforge.net/) by Piotr Fusik, which is GPLv2+;
this is why the project is GPL-licensed.
