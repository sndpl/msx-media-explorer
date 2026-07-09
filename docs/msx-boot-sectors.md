# MSX Boot Sectors: A Reference

Everything this project knows about sector 0 of an MSX floppy disk — its
layout, the MSX-DOS 1 vs MSX-DOS 2 differences, the quirks that break strict
PC FAT parsers, and how the code in this repository reads, repairs, detects,
and installs boot sectors.

The DOS 1 and DOS 2 boot blocks embedded in `msx-disk` are ports of openMSX's
`src/fdc/BootBlocks.cc` (GPL-2.0): real sector dumps captured on a Philips
NMS 8250 — the DOS 1 block from the Disk ROM's `_format`, the DOS 2 block from
MSX-DOS 2.20's FORMAT.

## How an MSX boots from disk

1. At power-on the Disk ROM (the BIOS extension in the disk interface) reads
   **logical sector 0** of the disk into RAM at address **`0xC000`**.
2. It jumps to **`0xC01E`** — offset `0x1E` inside the sector. This entry
   point is a hardware-era convention and cannot move; it is the pivot around
   which the whole DOS 1/DOS 2 difference is arranged.
3. The first instruction of every standard boot loader is `RET NC`
   (`0xD0`): unless the Disk ROM signalled with the carry flag that booting
   is possible, control returns immediately and the machine drops into BASIC.
4. If the boot proceeds, the loader uses the BDOS entry point at **`0xF37D`**
   to open and load `MSXDOS.SYS` at `0x0100`, then jumps there. On any
   failure it prints `Boot error / Press any key for retry` and loops.

A disk whose sector 0 contains no runnable loader is still perfectly usable
as a *data* disk — MSX-DOS reads its FAT filesystem regardless. The boot code
only matters when the machine tries to start from that disk.

## Sector layout

All MSX floppies use 512-byte sectors and FAT12. Sector 0 is laid out as a
standard DOS 2.0-era ("short") BPB boot sector:

| Offset | Size | Field | Notes |
|--------|------|-------|-------|
| `0x00` | 3 | Jump instruction | `EB FE 90` on real MSX formats (`JR $` + `NOP` — never executed, entry is at `0x1E`); `EB 3C 90` or `E9 xx xx` also accepted |
| `0x03` | 8 | OEM name | e.g. `"NMS 2.0P"` on NMS 8250 disks. Frequently overrun with garbage on DOS 1 disks |
| `0x0B` | 2 | Bytes per sector | Always 512 |
| `0x0D` | 1 | Sectors per cluster | 2 (1 on 180 KB) |
| `0x0E` | 2 | Reserved sectors | 1 (just the boot sector) |
| `0x10` | 1 | Number of FATs | 2 |
| `0x11` | 2 | Root directory entries | 112 (64 on 180 KB) |
| `0x13` | 2 | Total sectors (16-bit) | The authoritative count on every MSX floppy |
| `0x15` | 1 | Media descriptor | See table below |
| `0x16` | 2 | Sectors per FAT | 3 (720 KB) or 2 |
| `0x18` | 2 | Sectors per track | 9 |
| `0x1A` | 2 | Heads (sides) | 1 or 2 |
| `0x1C` | 2 | Hidden sectors | 0 — **and this is where MSX and PC layouts part ways** |
| `0x1E` | — | **Boot code entry point** | DOS 1: code starts here. DOS 2: a jump over the `VOL_ID` block (below) |
| `0x1FE` | 2 | `55 AA` signature | **Usually absent on MSX disks** — MSX-DOS never required it |

Canonical BPB values per format (what `msx-disk`'s `fs/boot.rs` synthesizes
and `create_blank` writes):

| Format | Size | Media | Sides × tracks × spt | Sec/cluster | Root entries | Sec/FAT |
|--------|------|-------|----------------------|-------------|--------------|---------|
| 3.5" DS 2DD | 720 KB | `0xF9` | 2 × 80 × 9 | 2 | 112 | 3 |
| 3.5" SS 1DD | 360 KB | `0xF8` | 1 × 80 × 9 | 2 | 112 | 2 |
| 5.25" SS DD | 180 KB | `0xFC` | 1 × 40 × 9 | 1 | 64 | 2 |
| 5.25" DS DD | 360 KB | `0xFD` | 2 × 40 × 9 | 2 | 112 | 2 |

The media descriptor is written twice: in the BPB at `0x15` *and* as the
first byte of each FAT — and on MSX the FAT copy is the one that matters
(see quirks below).

## Why MSX boot sectors break strict PC parsers

MSX-DOS 1 does not read the BPB at all. It derives the complete disk
geometry from the media-descriptor byte alone. Three consequences:

- **Garbage BPBs are common and harmless (to an MSX).** DOS 1-era formatters
  and custom game loaders leave overrun OEM strings, zeroed fields, or
  nonsense values (one real example, the *Brainstorm* loader, declares
  `total_sectors = 2`) in the BPB. The disk still works on real hardware.
- **No `55 AA` signature.** Both the DOS 1 and DOS 2 blocks captured from
  real hardware end in zeros at `0x1FE`; MSX never checks. Strict PC parsers
  (including the `fatfs` crate) refuse to mount without it.
- **The "extended" fields are actually code.** A PC parser reads offsets
  `0x1C`–`0x26` as hidden sectors, 32-bit total sectors, drive number, and
  the `0x29` extended-signature/volume-label block. On an MSX disk those
  bytes are the middle of the Z80 boot program, so a strict parser can
  hallucinate a bogus 32-bit sector count or a garbage volume label.

`msx-disk` handles this with a **mount-time repair shim**
(`fs/boot.rs::repair_boot_sector`): in a *private copy* of the image it adds
the `55 AA` signature, zeroes the pseudo-extended fields (`0x1C`–`0x26`),
and — only if the existing BPB is not self-consistent — synthesizes the
canonical BPB for the disk's byte size. The caller's original bytes are never
modified; every write transaction (`fs/write.rs`) snapshots sector 0 before
mounting and restores it before writing back, so real boot code always
survives editing.

## MSX-DOS 1 vs MSX-DOS 2

The two generations share everything up to offset `0x1D` and everything from
`0xAB` (the `MSXDOS  SYS` FCB) onward. The difference is a 16-byte metadata
block DOS 2 wedged in at the fixed entry point:

### DOS 1

Boot code starts directly at `0x1E`. Nothing else is special.

### DOS 2

Offset `0x1E` holds `18 10` (`JR +0x10`), hopping over the **`VOL_ID`
block** to the relocated code at `0x30`:

| Offset | Size | Content |
|--------|------|---------|
| `0x1E` | 2 | `18 10` — `JR +0x10` to the boot code at `0x30` |
| `0x20` | 6 | ASCII **`"VOL_ID"`** — the DOS 2 format marker |
| `0x26` | 1 | Undelete flag (`0x00` on a freshly formatted disk) |
| `0x27` | 4 | **Volume serial number** (this project reads/writes it little-endian; MSX-DOS 2's FORMAT fills it with a random value) |
| `0x2B` | 5 | Reserved, zero |
| `0x30` | — | Boot code (same program as DOS 1, reassembled) |

This block — not the boot code — is what MSX-DOS 2 actually cares about. It
checks for `"VOL_ID"` at `0x20` to decide whether the disk is in "its own"
format:

- **Present** → the disk supports `UNDEL` (undelete) and the disk cache can
  identify the disk across floppy swaps via the serial number.
- **Absent** → the disk is handled in DOS 1 compatibility mode: no undelete,
  degraded cache behaviour. MSX-DOS 2's `FIXDISK` utility upgrades such a
  disk in place — which is exactly what `mediaexplorer-cli bootsector
  <image> --dos 2` reproduces.

The reverse direction is a non-event: MSX-DOS 1 never looks, and happily
boots and uses a DOS 2-formatted disk.

Annotated hex of the DOS 2 block's distinctive region:

```
offset  bytes                                        meaning
0x00    EB FE 90                                     jump stub (never executed)
0x03    4E 4D 53 20 32 2E 30 50                      OEM "NMS 2.0P"
0x0B    00 02 02 01 00 02 70 00  A0 05 F9 03 00 09   BPB: 512 B/sec, 2 sec/clu, 1 resvd,
0x19    00 02 00 00 00                                 2 FATs, 112 root, 1440 total, F9,
                                                       3 sec/FAT, 9 spt, 2 heads, 0 hidden
0x1E    18 10                                        JR +0x10        <- DOS1 code starts here instead
0x20    56 4F 4C 5F 49 44                            "VOL_ID"
0x26    00                                           undelete flag
0x27    71 60 03 19                                  volume serial (patched per disk)
0x2B    00 00 00 00 00                               reserved
0x30    D0 ED 53 6A C0 ...                           boot loader code
```

### Byte-exact diff of the two embedded blocks

Verified against the openMSX dumps:

- Identical: `0x00`–`0x1D` (jump, OEM, 720 KB BPB) and `0xAB`–`0x1FF`
  (the `\0MSXDOS  SYS` FCB at `0xAB` and trailing zeros).
- Different: scattered ranges within `0x1E`–`0xAA` — the `VOL_ID` block plus
  the same loader reassembled 18 bytes later (every absolute address baked
  into the code shifts, e.g. self-modify targets `0xC059` → `0xC06A`).
- Last non-zero byte in both: `0xB6`. Neither has the `55 AA` signature.

### The boot loader program (both generations)

Functionally identical in DOS 1 and DOS 2:

1. `RET NC` — bail out to BASIC unless the Disk ROM set carry.
2. Stash the ROM-provided boot parameters (self-modifying stores), set the
   stack to `0xF51F`.
3. `LD DE,0xC0AB` / `LD C,0x0F` / `CALL 0xF37D` — BDOS *open file* on the
   FCB at `0xC0AB`, whose name field reads `MSXDOS  SYS`.
4. On failure → boot-error routine.
5. `LD DE,0x0100` / `LD C,0x1A` — BDOS *set DMA address* to `0x0100`.
6. Set the FCB random-record field to 1, then `LD HL,0x3F00` /
   `LD C,0x27` — BDOS *random block read*: load up to `0x3F00` bytes of
   `MSXDOS.SYS` at `0x0100`.
7. `JP 0x0100` — enter MSXDOS.SYS.

The only genuine code differences in the DOS 2 version, beyond relocation:

- The error message is printed with **BDOS function 9** (`$`-terminated
  string) instead of DOS 1's hand-rolled per-character loop — visible in the
  data: DOS 1's message ends `...retry\r\n\0`, DOS 2's ends `...retry\r\n$`.
- The error/warm-boot check is simplified (`SUB 2 / OR 0 / JP Z,0x4022`
  replacing DOS 1's compare plus a per-boot flag byte it kept at `0xC0D0`).

### Boot sector ≠ system disk

Both loaders load `MSXDOS.SYS` (the DOS 2 kernel proper lives in the
cartridge ROM; its disk components are `MSXDOS2.SYS`/`COMMAND2.COM`).
Installing a boot sector therefore does **not** make a disk bootable by
itself — that additionally requires the MSX-DOS system files, which are
copyrighted and never embedded by this project. A DOS 2 boot sector on a
plain data disk is still worthwhile purely for the `VOL_ID` block.

### Related: Nextor

Nextor (the modern MSX-DOS 2 successor) formats FAT12 volumes with the same
`VOL_ID` convention and FAT16 volumes with the PC-style `0x29` extended BPB
(volume ID + label + `"FAT16"` type string), and *does* write the `55 AA`
signature. openMSX's `BootBlocks.cc` carries both Nextor blocks alongside
the two ported here; this project does not currently install them.

## Detecting the DOS generation of a disk

The boot sector is the primary signal, but it can be wiped or custom, so
`msx-disk`'s `detect_dos_version()` (`fs/dos.rs`) classifies a disk as DOS 2
if **any** of these hold:

1. `"VOL_ID"` at boot-sector offset `0x20`;
2. the filesystem contains a subdirectory (only DOS 2 creates them);
3. `MSXDOS2.SYS` exists in the root directory.

Otherwise it reports DOS 1. Detection must run on the *raw* sector 0 — the
mount-time repair shim zeroes the marker region in its private copy.

## Where all of this lives in this repository

| Concern | Location |
|---------|----------|
| Canonical BPBs, mount-time repair shim, BPB validity check | `crates/msx-disk/src/fs/boot.rs` |
| Embedded DOS 1/DOS 2 boot blocks (openMSX port, provenance notes) | `crates/msx-disk/src/fs/bootblocks.rs` |
| `set_boot_block()` — install a boot block, preserving the disk's BPB (`0x0B`–`0x1D`) and its existing `0x1FE` signature bytes, patching the DOS 2 serial | `crates/msx-disk/src/fs/write.rs` |
| Blank-disk formatting (BPB + FAT seeding; new disks get a DOS 2 block via callers) | `crates/msx-disk/src/fs/write.rs::create_blank` |
| DOS generation detection | `crates/msx-disk/src/fs/dos.rs` |
| Boot-sector size-declaration repair (truncated/padded images) | `crates/msx-disk/src/fs/sizefix.rs` |
| Sector-0 preservation during file edits | `crates/msx-disk/src/fs/write.rs::transaction` |
| CLI: `new --dos 1\|2`, `bootsector [--dos 1\|2]` (show/install, FIXDISK-style) | `crates/mediaexplorer-cli/src/commands/{new,bootsector}.rs` |
| GUI: New Disk creates DOS 2 disks with a fresh serial | `crates/mediaexplorer-gui/src/app/actions.rs::new_disk` |
| BPB decode in the hex viewer's data inspector | `crates/mediaexplorer-gui/src/app/hexrender.rs` |

## References

- openMSX, `src/fdc/BootBlocks.cc` — the boot-block dumps ported here.
- [openMSX diskmanipulator manual](https://openmsx.org/manual/diskmanipulator.html)
  — formats DOS 2 by default, `-dos1`/`-nextor` options (the convention
  `mediaexplorer-cli new` follows).
- [The Ultimate MSX FAQ, MSX-DOS 2 section](https://www.faq.msxnet.org/dos2.html)
  — `VOL_ID`, `UNDEL`/cache behaviour, and `FIXDISK`.
