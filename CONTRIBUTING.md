# Contributing

Thanks for considering a contribution. This document covers the things you
cannot infer from reading the source.

## Building and testing

```sh
cargo build --workspace
cargo run -p mediaexplorer-gui          # the GUI; the binary is named `mediaexplorer`
cargo run -p mediaexplorer-cli -- ls <image>
cargo test --workspace
cargo test -p msx-disk <name>           # a single test by substring
```

Headless examples under `crates/msx-disk/examples/` exercise the core library
without the GUI and are the fastest way to reproduce a parsing bug in
isolation. See `CLAUDE.md` for the full list.

## CI is strict

CI runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
and `cargo test --workspace` on Linux, Windows, and macOS with
`RUSTFLAGS: -D warnings`. **Any warning fails the build**, including unused
code. Run all three locally before opening a pull request. `rustfmt.toml` pins
`max_width = 100`.

A separate job runs `cargo deny check licenses bans sources`. A new dependency
whose licence is not in `deny.toml`'s allowlist will fail it. New dependencies
need justification anyway — each one is attack surface and maintenance burden.

## The one architectural rule

**`msx-disk` has zero UI dependencies.** Every function returns plain data —
sector buffers, directory trees, decoded view models — so the logic is
unit-testable and reusable from the CLI. Keep rendering in
`mediaexplorer-gui` and decoding or parsing in `msx-disk`. Command logic that
needs new disk behaviour belongs in `msx-disk`, not in the CLI.

`CLAUDE.md` documents the layering inside `msx-disk` in more detail, including
why filename charset handling deliberately uses a private-use-area converter
rather than a standard one.

## Tests

Unit tests live inline in each module as `#[cfg(test)] mod tests` and build
synthetic disks with `fatfs::format_volume`, so they need no fixtures.

`crates/msx-disk/tests/real_fixtures.rs` runs against real MSX images in the
workspace-root `tests/` directory. Those files are gitignored — some are
copyrighted — so every test there uses `skip_if_absent!` and skips gracefully
when its fixture is missing. CI stays green without them while developers who
have the files get real-world coverage.

**Do not commit binary fixtures.** When adding a real-format regression test,
follow the skip-if-absent pattern.

## Licence

The project is GPL-2.0-or-later, inherited from the RECOIL port in
`crates/msx-disk/src/recoil/`. Contributions are accepted under that licence.
Keep new files compatible.
