# MSX Disk Explorer

A cross-platform (Linux, Windows, macOS) desktop tool for browsing, inspecting,
and converting vintage **MSX floppy disk images**. A modern, open-source
successor to the classic Windows-only DskExplorer.

## Status

Working: open every common MSX image format, browse files and MSX-DOS 2
subdirectories, view file contents as HEX / TEXT / tokenized BASIC /
SCREEN 2,5,7,8,12 bitmaps, search within a file (text or hex), and extract
files.

Editing (write-back to the source image): add, rename, and delete files, and
edit a file's bytes in hex. The original boot sector is always preserved, so
MSX-DOS 1 boot code is never clobbered.

Conversions: create a new blank disk (360/720 KB), and Save As to convert any
open image to `.dsk` or `.xsa` (full XSA compressor + decompressor).

Still to come: graphical disk-usage / per-file sector maps, a view-and-edit
sector view, and drag-and-drop in/out of the OS file manager.

## Supported image formats (read)

| Format | Notes |
|--------|-------|
| `.dsk`, `.di1`, `.ds1`, `.di2`, `.ds2` | Raw sector dumps (360KB / 720KB) |
| `.img` | Raw with a leading side-count byte |
| `.msx` | 720KB, cylinder-interleaved sides |
| `.ddi` | DiskDupe image (header + raw) |
| `.xsa` | Compressed disk image (decompressed on open) |
| `.cas` | MSX cassette tape image (list / extract) |

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

Dual-licensed under MIT or Apache-2.0.
