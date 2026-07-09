# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

MSX Media Explorer — a cross-platform (Linux/Windows/macOS) desktop tool for
browsing, inspecting, editing, and converting vintage MSX floppy/tape/HD disk
images. A modern successor to the Windows-only DskExplorer. The README has the
full user-facing feature list and supported-format table; read it for "what does
this do" questions.

## Commands

```sh
cargo build --workspace
cargo run -p mediaexplorer-gui          # launch the GUI (binary is named `mediaexplorer`)
cargo run -p mediaexplorer-cli -- ls <image>   # the CLI companion (ls/add/extract/rm/mv/mkdir/new/bootsector/convert)
cargo test --workspace                  # all unit + integration tests
cargo test -p msx-disk <name>           # run a single test by substring
cargo fmt --all --check                 # CI runs this; must pass
cargo clippy --workspace --all-targets  # CI runs this; see warnings note below
```

CI (`.github/workflows/ci.yml`) runs fmt + clippy + test on all three OSes with
`RUSTFLAGS: -D warnings`, so **any warning fails the build**. Treat clippy
warnings and unused code as hard errors locally. `rustfmt.toml` pins
`max_width = 100`.

### Headless CLI examples (live in `crates/msx-disk/examples/`)

These exercise the core library without the GUI — useful for reproducing parsing
bugs in isolation:

```sh
cargo run -p msx-disk --example lsdsk -- <image>              # list disk contents
cargo run -p msx-disk --example lscas -- <file.cas>           # list tape contents
cargo run -p msx-disk --example basview -- <image> <path>     # detokenize a .BAS
cargo run -p msx-disk --example fileinfo -- <file> ...        # Info-pane logic
cargo run -p msx-disk --example recoilpng -- <image> <file> <out.png>  # graphics decode
```

### Packaging a macOS `.app`/`.dmg`

`cargo-packager` consumes an already-built binary (no `before-packaging-command`).
See README "Packaging a macOS app"; config is in
`crates/mediaexplorer-gui/Cargo.toml` under `[package.metadata.packager]`. The
tagged-release workflow (`release.yml`) builds a universal binary automatically.

## Architecture

A Cargo workspace with a strict UI/logic split. **The cardinal rule: `msx-disk`
has zero UI dependencies.** Every function returns plain data (sector buffers,
directory trees, decoded view models) so logic is unit-testable and reusable from
a CLI. Keep rendering in the GUI crate and decoding/parsing in the library.

`crates/mediaexplorer-cli` is that CLI: one pure function per subcommand in
`src/commands/` (bytes in → bytes/render-plan out, unit-tested on synthetic
disks), with all host I/O — open, guards, `reencode` + atomic write — in
`main.rs`/`disk.rs`. Command logic that needs new disk behavior belongs in
`msx-disk`, not in the CLI.

### `crates/msx-disk` — headless core library

The data flows through three layers, each unaware of the one below its concern:

1. **`image/`** — container layer. Every supported container (`.dsk`, `.img`,
   `.msx`, `.ddi`, `.xsa`, `.dmk`, `.sav`) is read into a **normalized** flat
   buffer of 512-byte sectors in logical (LBA) order, exactly like a raw
   `.dsk`. Anything that precedes the sector data (the `.img` side byte, the
   `.ddi` header) is stashed in `DiskImage.prefix` so `reencode()` can write
   back into the same container. `.xsa` is recompressed and the MSXPLAYer
   `.sav` diff journal re-journaled by `reencode()` on save; only raw-track
   `.dmk` is read-only (`is_writable()` is false) — to edit one, save it as
   `.dsk`. Format is chosen by extension, falling back to magic-byte sniffing.

2. **`fs/`** — FAT12/FAT16 filesystem on top of a normalized buffer. Two paths:
   - **Floppy**: a single FAT volume mounted via the `fatfs` crate (`DiskFs`).
     Editable.
   - **Partitioned HD**: openMSX `MSX_IDE` images with up to 4 partitions, each a
     read-only `Volume` (`fs/partition.rs`, `fs/volume.rs`). FAT width is read
     from each partition's own boot sector with an MSX-correct rule, not a
     cluster-count heuristic.
   `boot.rs` repairs the boot sector in a private copy on mount so MSX-DOS
   compatibility is restored **without ever touching the caller's bytes** — the
   original boot code (MSX-DOS 1 boot loader) is preserved on write-back.
   `map.rs` classifies each sector (reserved/FAT/root/used/free) for the Map view;
   `write.rs` does add/rename/delete; `stats/` computes whole-disk insight and FAT
   chain integrity.

3. **`view/`, `fileinfo/`, `recoil/`, `archive/`, `charset/`** — decoders that
   turn file bytes into render-ready models: hex dump + data inspector
   (`view/hex.rs`), text, BASIC detokenizer, Z80/R800 disassembler, content-derived
   file info (BSAVE header, graphics/music metadata), LZH/LHA/PMA archive listing,
   and MSX graphics. `recoil/` is a **port of RECOIL** (Piotr Fusik, GPLv2+) —
   this is why the whole project is GPL-2.0-or-later.

`tape/`, `cas.rs`, `tsx.rs` handle cassette images on a parallel track to disks.

**Charset handling is subtle**: MSX filenames can contain bytes >= 0x80. The
filesystem mounts with a custom `PUA_OEM_CP_CONVERTER` (`charset/`) that maps high
bytes into the Unicode Private Use Area instead of dropping them to U+FFFD, so
names round-trip losslessly and can be re-decoded for display under the
International or Japanese code page. Don't "fix" filename handling to use a
standard converter.

### `crates/mediaexplorer-gui` — egui/eframe desktop app

- `state.rs` — `LoadedDisk`/`LoadedTape`: everything the UI needs about an open
  document, computed once on load. The `Backing` enum splits floppy (editable,
  `DiskFs`) from partitioned (read-only `Vec<Volume>`, surfaced as synthetic
  `P{n}` tree nodes). Expensive things (whole-image SHA-1, stats) are behind
  `OnceCell` and computed lazily.
- `app.rs` — the `eframe::App` impl and all rendering (large; ~5700 lines). Holds
  `ViewMode` (Info/Hex/Text/Basic/Disasm/Screen/Archive — which file tabs show
  depends on the selection) and `AppView` (Files/Sectors/Map/Stats/Analyze/Blocks
  — which top-level tabs show depends on the document type).
- `settings.rs` — user prefs persisted via eframe's `persistence` feature.
- `hexlayout.rs`, `tree_nav.rs` — extracted helpers for hex rendering and keyboard
  tree navigation.
- `macos.rs` (cfg macos) — native `muda` menu bar, app/About-panel name and icon.
- `dnd.rs` + the `drag` crate (macos/windows only) — native drag-out to the OS
  file manager; Linux falls back to the Extract dialog.
- Fonts: GNU Unifont is appended as a fallback so decoded MSX glyphs (kana,
  accented Latin, box-drawing) render instead of tofu.

## Testing conventions

- Unit tests live inline (`#[cfg(test)] mod tests`) in each module; they build
  synthetic disks (`fatfs::format_volume` + writes) so they need no fixtures.
- `crates/msx-disk/tests/real_fixtures.rs` runs against **real MSX disk/archive
  files in the workspace-root `tests/` directory**. Those files are gitignored
  (some are copyrighted), so each test uses `skip_if_absent!` to skip gracefully
  when its fixture is missing — CI stays green while developers with the files get
  real-world coverage. When adding a real-format regression test, follow this
  skip-if-absent pattern rather than committing the binary.

## Conventions to respect

- **Immutability / no in-place mutation of caller data** — the boot-sector-repair
  and reencode design depends on never mutating inputs.
- **No new dependencies without justification** — each is GPL-compatible and
  vetted; the `target.'cfg(...)'` blocks keep platform-specific crates off other
  platforms.
- License is **GPL-2.0-or-later** (inherited from the RECOIL port); keep new files
  compatible.
