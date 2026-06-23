# MSX Disk Explorer

A cross-platform (Linux, Windows, macOS) desktop tool for browsing, inspecting,
and converting vintage **MSX floppy disk images**. A modern, open-source
successor to the classic Windows-only DskExplorer.

## Status

Working: open every common MSX image format, browse files and MSX-DOS 2
subdirectories, view file contents as HEX / TEXT / tokenized BASIC, search
within a file (text or hex), and extract files.

Graphics viewer: the full RECOIL-derived MSX/MSX2/MSX2+/V9990 format set —
SCREEN 2-12, Graph Saurus (SR*), GL/SH + .PLx palettes, .Sxx interlace,
YJK/YAE, V9990 .G9B, Dynamic Publisher (PCT/FNT/STP), DD-Graph (CMP),
MSX Interchange (MIF/MIG), Maki-chan (MAG/MKI/MAX), and PI/PIC. Companion
palette/interlace files are read from the same disk.

Editing (write-back to the source image): add, rename, and delete files, and
edit a file's bytes in hex. Cmd/Ctrl-click marks multiple files for batch
extract or delete. Drop a disk image on the window to open it, or drop other
files onto an open disk to add them; on macOS and Windows you can also drag
files out of the list into the OS file manager. The original boot sector is
always preserved, so MSX-DOS 1 boot code is never clobbered.

Conversions: create a new blank disk (360/720 KB), and Save As to convert any
open image to `.dsk` or `.xsa` (full XSA compressor + decompressor).

Disk views: a "view disk by sector" hex view with in-place sector editing and
disk-wide search (text or hex, jump between matches), and a graphical disk-usage
map (reserved / FAT / root / used / free) that outlines the selected file's
sectors; clicking a cell opens that sector.

Raw-track disks (`.dmk`): opened by decoding the per-track sector data into a
normal disk, plus an Analyze view listing every track's sectors, density, and
CRC status so non-standard or copy-protected tracks are visible.

Tapes (`.cas`, `.tsx`): opened as a list of files (each viewable in the same
HEX/TEXT/BASIC/SCREEN viewer and extractable), plus a Blocks view showing every
tape block — TSX `#35` custom-info, `#32` archive-info, and `#4B` MSX blocks
with their detected ASCII/BINARY/BASIC headers.

Status bar: with a disk open, shows the physical disk type (e.g. 3.5" Double
Sided, Double Density 2DD) and the BPB-derived filesystem facts (size, clusters,
sectors per cluster, bytes per sector, total sectors, volume label).

Clipboard & export: copy the current view to the OS clipboard (hex dump / text /
BASIC listing as text, or the decoded screen image as a bitmap), and save the
viewed MSX image as a PNG.

## Supported image formats (read)

| Format | Notes |
|--------|-------|
| `.dsk`, `.di1`, `.ds1`, `.di2`, `.ds2` | Raw sector dumps (360KB / 720KB) |
| `.img` | Raw with a leading side-count byte |
| `.msx` | 720KB, cylinder-interleaved sides |
| `.ddi` | DiskDupe image (header + raw) |
| `.xsa` | Compressed disk image (decompressed on open) |
| `.dmk` | David Keil raw-track image (read-only; normalized + analyzed) |
| `.cas` | MSX cassette tape image (files + block overview) |
| `.tsx` | MSX tape image (TZX 1.21 with `#4B` Kansas City blocks) |

## Architecture

A Cargo workspace with two crates:

- **`msx-disk`** — headless core library: image-format parsing, FAT12
  filesystem, file viewers (hex/text/BASIC/screen), and search. No UI
  dependencies, fully unit-tested.
- **`dskexplorer-gui`** — the [egui](https://github.com/emilk/egui) /
  `eframe` desktop application that renders the models produced by `msx-disk`.

## Building

```sh
cargo build --workspace
cargo run -p dskexplorer-gui
cargo test --workspace
```

## License

GPL-2.0-or-later. See [COPYING](COPYING).

The MSX/MSX2/MSX2+/V9990 graphics-format decoders are ported from
[RECOIL](https://recoil.sourceforge.net/) by Piotr Fusik, which is GPLv2+;
this is why the project is GPL-licensed.
