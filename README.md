##

```
██╗     ███████╗ ██████╗ ███╗   ██╗
██║     ██╔════╝██╔═══██╗████╗  ██║
██║     █████╗  ██║   ██║██╔██╗ ██║
██║     ██╔══╝  ██║   ██║██║╚██╗██║
███████╗███████╗╚██████╔╝██║ ╚████║
╚══════╝╚══════╝ ╚═════╝ ╚═╝  ╚═══╝
```

##

`▐▀` `-` `▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▌`

a Flicker-Free Bootloader Written in **`[Rust]`**, It acquires the **`[BGRT]`** motherboard logo and the **`[GOP]`** frame buffer without ever calling `set_mode`, then chainloads any `\EFI\*.efi` boot entry — its own kernel included — with the screen untouched.

- **`[Version]`**: **`[0.7.0]`**

`▐▄` `-` `▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▌`

<details>
<summary>Contents</summary>

## Table of Contents

- [**`[Overview]`**](#overview)
- [**`[Architecture]`**](#architecture)
- [**`[Boot]`**](#boot)
- [**`[BGRT]`**](#bgrt)
- [**`[GOP]`**](#gop)
- [**`[Entries]`**](#entries)
- [**`[Kernel]`**](#kernel)
- [**`[Logging]`**](#logging)
- [**`[Installation]`**](#installation)
- [**`[Binaries]`**](#binaries)
- [**`[Building]`**](#building)
- [**`[Filesystem]`**](#filesystem)
- [**`[Testing]`**](#testing)
- [**`[Structure]`**](#structure)
- [**`[Dependencies]`**](#dependencies)
- [**`[Contributing]`**](#contributing)

---

</details>

<details>
<summary>Overview</summary>

## Overview

- Chainloader — every boot entry on the ESP is auto-discovered from `\EFI\<vendor>\*.efi` and written to `\EFI\leon\entries.jsonc`; any of them can be chainloaded via `LoadImage`/`StartImage`
- Flicker-free handoff — never calls GOP `set_mode`, so the graphics hardware is never re-initialized and the firmware logo is never cleared
- BGRT acquisition — ACPI 2.0 RSDP → XSDT signature walk → BGRT parse with BMP validation
- GOP frame buffer capture — the current mode's buffer is lifted out untouched
- Optional `gop-ui` feature — adds a feature-gated GOP framebuffer renderer scaffold for the bootloader, currently used to draw a placeholder splash before the text-mode menu
- Boot config — `\EFI\leon\boot.toml` is written by `lbc config set` and parsed + validated by the loader at every boot (a broken file yields defaults, never a blocked boot)
- Geometry record — every boot records the real geometry + resolved config as `\EFI\leon\bootinfo.json` for host tooling
- Secure Boot — reads the `SecureBoot`/`SetupMode` global variables, warns on the menu and in the log when it is active, and reports a firmware `ACCESS_DENIED`/`SECURITY_VIOLATION` rejection of an unsigned entry instead of failing silently (`scripts/sign.sh` self-signs the loader + kernel; see `docs/secure-boot.md`)
- EFI-stub kernel — `lbl-kernel` is a plain UEFI application that acquires GOP + BGRT itself and never receives a handoff blob
- Shared ABI — `common/` is the single source of truth for the pixel formats, frame buffer geometry, BGRT metadata, and the `boot.toml` parser
- Ultra-silent logging — errors go to `\var\logs\leon\log.md` (capped at 64 KiB), nothing ever touches the screen
- `lbt` build tool — discover ESPs and build bootable images, plus the `lbm` menuconfig editor and `lbc` boot config/control companions (all pure Rust, no embedded Python/`cps`)
- Dual architecture — amd64 and arm64 from a single host
- Five build front-ends — Make, Ninja, Meson, Cargo, and a CMake toolchain, all with auto-detected architecture

---

</details>

<details>
<summary>Architecture</summary>

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                      lbl  (chainloader)                      │
│                                                              │
│       ┌──────────┐   ┌──────────┐   ┌────────────────┐       │
│       │   BGRT   │   │   GOP    │   │ boot.toml      │       │
│       │   find   │   │  capture │   │ (shared parser)│       │
│       └────┬─────┘   └────┬─────┘   └────────────────┘       │
│            │              │                                  │
│       ┌────┴─────┐   ┌────┴─────┐   ┌────────────────┐       │
│       │ RSDP →   │   │ current  │   │ discover       │       │
│       │ XSDT →   │   │  mode    │   │ \EFI\*\*.efi → │       │
│       │ BGRT+BMP │   │ (no set_ │   │ entries.jsonc  │       │
│       └──────────┘   │  mode)   │   │ + geometry dump│       │
│                      └──────────┘   └────────────────┘       │
└─────────────────────────────┼────────────────────────────────┘
                              |
                              | menu (optional)
                              │ LoadImage / StartImage
                              │ BootPolicy::BootSelection
                              ▼
┌──────────────────────────────────────────────────────────────┐
│           lbl-kernel  (EFI stub, no handoff blob)            │
│                                                              │
│     acquires GOP + BGRT itself     exit_boot_services()      │
│     validates the memory map       marker + halt             │
└──────────────────────────────────────────────────────────────┘
```

| Component | File | Purpose |
|-----------|------|---------|
| BGRT | `src/firmware/bgrt.rs` | ACPI 2.0 RSDP discovery, XSDT walk, BGRT parse, BMP header validation |
| GOP | `src/firmware/gop.rs` | Capture the current mode's frame buffer without `set_mode` |
| Boot pipeline | `src/boot/mod.rs` | `run()` — config → discover → record → menu → chainload |
| Entries | `src/boot/entries.rs` | Scan `\EFI\<vendor>\*.efi`, write `\EFI\leon\entries.jsonc` (JSONC) |
| Config | `src/boot/config.rs` | Read + validate `\EFI\leon\boot.toml` (best-effort defaults) |
| Menu | `src/boot/menu.rs` | Optional `splash = true` menu; boxed/colored frame sized to the text mode, timeout countdown bar, `default_entry`, any-entry boot |
| Chainload | `src/boot/image.rs` | `LoadImage`/`StartImage` of any entry via its device path |
| Secure Boot | `src/secure_boot.rs` | Read `SecureBoot`/`SetupMode` globals; menu + log warning; report firmware SB rejections |
| Geometry record | `src/record/dump.rs` | Write `\EFI\leon\bootinfo.json` (geometry + resolved config) |
| Logger | `src/logger/mod.rs` | Capped Markdown log, silently dropped on failure |
| Shared ABI | `common/src/geometry.rs` | `Framebuffer` / `Bgrt` / `PixelFormat` structs |
| Boot config parser | `common/src/boot_config.rs` | `no_std` parser for `boot.toml`, shared with `lbt` |
| Kernel | `kernel/src/{gop,bgrt,memmap,marker,main}.rs` | EFI-stub kernel — self-acquires GOP + BGRT, EBS, marker, halt |
| Build tool | `lbt/` | Host tool — ESP/boot-entry discovery, image builders, geometry |
| Config + boot control | `lbc/` | Host tool — `boot.toml` management (`config set`/`get`/…), ESP staging |
| Menuconfig editor | `lbm/` | Host TUI — menuconfig-style editor for `boot.toml` |

---

</details>

<details>
<summary>Boot</summary>

## Boot

```
1. Capture the GOP frame buffer
   current_mode_info() is queried; set_mode is never called, so the
   graphics hardware is never re-initialized.

2. Locate and validate the BGRT
   ACPI 2.0 RSDP (config table) -> XSDT signature walk -> BGRT parse
   -> BMP header validation (dimensions).

3. Read and validate the boot configuration
   \EFI\leon\boot.toml (written by `lbc config set`) is parsed with the
   shared leon_common::boot_config parser. A missing or broken file
   yields defaults and is logged; it never blocks the boot.

4. Discover boot entries
   Every \EFI\<vendor>\*.efi on the boot volume becomes an entry, written
   as JSONC to \EFI\leon\entries.jsonc.
   Additionally, Leon scans common `/boot` locations (`/boot`, `/boot/efi`,
      `/boot/EFI`) for standalone EFI binaries and EFI-stub kernels. Detection
      is now stricter:
      - `.efi` files are accepted only after verifying a valid PE/COFF header
         (`MZ` DOS header + `PE\0\0` signature).
      - ELF files are considered only if their first 64 KiB include the ASCII
         marker `EFI stub` (case-insensitive), a reliable sign the kernel was
         built with the EFI stub support.
      Files passing these checks are added as candidate boot entries so kernels
      installed to `/boot` are detected even when not present on a mounted ESP.

5. Record the geometry
   The live frame buffer + BGRT geometry and the resolved config are
   written to \EFI\leon\bootinfo.json for host tooling.

6. Check the Secure Boot state
   The `SecureBoot`/`SetupMode` global variables are read once. When Secure
   Boot is active a warning row appears in the boot menu and a line is
   appended to the boot log; the images still boot if they are signed with a
   key enrolled in the platform's db. If the firmware later rejects an entry
   at LoadImage time (ACCESS_DENIED / SECURITY_VIOLATION), the rejection is
   reported in the log with a hint to sign it or enroll its key.

7. Choose an entry
   With `splash = true` a menu appears; otherwise (or on timeout) the
   `default_entry` is booted, falling back to the first discovered entry.

8. Chainload
   The chosen image is loaded with LoadImage and started with StartImage
   (BootPolicy::BootSelection), still without touching the screen. The
   EFI-stub kernel then does its own GOP/BGRT acquisition, calls
   exit_boot_services() itself, draws a marker, and halts.
```

Every step is silent. If any step fails, the error is appended to the log file and the CPU parks in a spin loop — the screen is never touched.

---

</details>

<details>
<summary>BGRT</summary>

## BGRT

The Boot Graphics Resource Table is how the firmware tells software where the motherboard logo is on screen and where its raw image lives in memory. Leon only reads it; the logo memory is never touched.

```
RSDP:   find the ACPI 2.0 GUID entry in the EFI configuration table
        -> verify the 'R' signature -> require revision >= 2

XSDT:   read the XSDT address at RSDP offset 24
        -> scan the 64-bit table pointer array for the "BGRT" signature

BGRT:   parse status, image_type, image_address, offset_x, offset_y

BMP:    validate the "BM" magic at image_address, read width/height/bpp,
        sanity-check against the 16 KiB (16384 px) limits
```

If the firmware simply has no logo, `find()` returns `None` quietly — a perfectly valid silent boot.

| Field | Type | Description |
|-------|------|-------------|
| `image_address` | `u64` | Physical address of the raw BMP logo |
| `offset_x` | `i32` | Logo X offset on screen |
| `offset_y` | `i32` | Logo Y offset on screen |
| `image_width` | `u32` | Logo width in pixels |
| `image_height` | `u32` | Logo height in pixels |
| `status` | `u8` | BGRT status byte (bit 0 = `DISPLAYED`) |
| `image_type` | `u8` | BGRT image type (0 = bitmap) |

The kernel (and the separate splash project) can call `bgrt.rect()` to get the logo's on-screen bounding box `(x0, y0, x1, y1)` and position UI elements relative to it.

---

</details>

<details>
<summary>GOP</summary>

## GOP

The Graphics Output Protocol provides the frame buffer. The golden rule of a silent handoff is **never call `set_mode`** — calling it forces the graphics hardware to re-initialize, blanks the screen, and destroys the firmware logo.

```
1. find_handles::<GraphicsOutput>()          -> first GOP device
2. open_protocol_exclusive::<GraphicsOutput> -> open it
3. current_mode_info()                       -> resolution, stride, format
4. gop.frame_buffer()                        -> base + size (untouched)
```

Only the current mode is queried and lifted out. If no GOP device exists or the frame buffer is not usable (e.g. `BltOnly` with a zero buffer), boot fails silently to the log.

| Variant | Value | Description |
|---------|-------|-------------|
| `Rgbx` | `0` | 24-bit RGB, 32-bit stride, last byte unused |
| `Bgrx` | `1` | 24-bit BGR, 32-bit stride (most common on UEFI) |
| `Bitmask` | `2` | Custom; color masks describe the layout |
| `BltOnly` | `3` | Not directly drawable; only `BLT` |

---

</details>

<details>
<summary>Entries</summary>

## Entries

Leon is a chainloader in the systemd-boot style: it does not hardcode a kernel path, it discovers boot entries on the ESP at every boot.

```
1. Open the boot volume via the Simple File System protocol
2. Walk the \EFI\ directory: every <vendor>\*.efi becomes an Entry
3. Write them as JSONC to \EFI\leon\entries.jsonc:
   \EFI\leon\kernel.efi           -> the Leon kernel itself
   \EFI\Microsoft\Boot\bootmgfw.efi -> Windows Boot Manager
   \EFI\systemd\systemd-bootx64.efi -> another bootloader
   ...
4. Boot the entry chosen by the menu / default_entry / first
```

Chainloading is done with the standard UEFI protocol — no manual PE loading:

```
1. Open the DevicePath protocol on the entry's loaded image device
2. Keep the volume nodes, append the entry's media file path node
3. LoadImage with BootPolicy::BootSelection
4. StartImage — the loaded image runs, the screen is never touched
```

Because the entries file is written every boot, `lbt discover` and the `lbm` entry picker always see what this firmware actually exposes. The kernel at `\EFI\leon\kernel.efi` is just one of those entries.

---

</details>

<details>
<summary>Kernel</summary>

## Kernel

`lbl-kernel` is a plain UEFI application — an "EFI stub" — built for `x86_64-unknown-uefi` and `aarch64-unknown-uefi` with `uefi 0.39`. It receives no handoff blob; it acquires everything it needs from firmware itself after being chainloaded.

```
efi_main():                 # #[entry], no arguments in uefi 0.39
  capture_gop()             # current mode, frame buffer untouched
  find_bgrt()               # RSDP -> XSDT -> BGRT -> BMP dimensions
  mask_interrupts()         # cli on x86_64, msr daifset on aarch64
  exit_boot_services()      # MemoryMapOwned, validated before trust
  draw_marker()             # 16x16 px in a corner that avoids the logo
  halt()                    # hlt on x86_64, wfi on aarch64
```

- GOP and BGRT acquisition mirrors the bootloader's (the geometry module in `common/` is shared), so the stub proves the bootloader's flicker-free handoff end-to-end
- The memory map returned by `exit_boot_services()` is validated before anything is trusted: a non-empty map must have a real descriptor pointer, a descriptor size at least `size_of::<MemoryDescriptor>`, and a total byte range that fits in `u64`
- The marker is green (`0x2e, 0xcc, 0x71`) and is placed by `free_corner()`, which picks a corner that does not intersect `bgrt.rect()`; `write_pixel()` honors the `Bgrx` / `Rgbx` byte order
- The panic handler black-boxes the message and parks the CPU — there is no console

The real kernel maps the physical frame buffer into its own virtual address space without clearing it and hands it to the display pipeline.

---

</details>

<details>
<summary>Logging</summary>

## Logging

Nothing is ever printed to the screen. The only place a boot error shows up is the unified log file on the boot volume:

```
\var\logs\leon\log.md
```

- Records are appended, Markdown list style, one line per boot error
- The file is kept below a hard 64 KiB cap — once a new line would exceed it, the oldest lines are dropped down to a line boundary, so the log stays bounded and always ends with the most recent errors
- A log that is not valid UTF-8 is discarded and restarted rather than corrupted
- Directories are created on demand
- If the log cannot be written (no filesystem, no permission, boot services gone), the error is dropped silently — silence wins over everything
- The kernel/init chain continues the same file at the same path on the real root filesystem, giving one continuous boot-to-desktop log

---

</details>

<details>
<summary>Installation</summary>

## Installation

Leon is a pure-Rust UEFI application pair — a chainloading bootloader and an EFI-stub kernel — installed onto the ESP.

```sh
# Stage a bootable ESP tree under build/esp (auto-detects arch; runs `lbc stage`)
# Use `DESTDIR=/mnt/esp` to stage directly to a mounted ESP root.
make stage

# Install onto a mounted ESP (also installs docs/man pages)
make install DESTDIR=/mnt/esp

# Build a GPT ESP image with an ESP partition (requires mtools + fdisk/sgdisk) -> build/leon-esp.img
make esp

# Self-sign the staged loader + kernel for Secure Boot (after `scripts/sign.sh setup`)
make sign

# Boot under QEMU/OVMF (amd64) or QEMU/AAVMF (arm64)
make qemu
```

To preview and drive the menuconfig TUI by hand instead of watching it
auto-boot away, hold it on screen with a long countdown:

```sh
make qemu-preview          # menu stays up ~5 minutes; Esc disarms it
MENU_TIMEOUT=30 make qemu  # or pick your own hold in seconds
```

`qemu-preview` rewrites the staged `boot.toml` timeout before booting (the
next `make stage` restores the default). In the QEMU window the arrow keys
move the selection, Enter selects/boots, and Esc pauses the countdown;
`Ctrl-A` then `c` switches to the QEMU monitor (`Ctrl-A` then `x` quits).

On real hardware the bootloader must use the UEFI-canonical removable-media name — `BOOTX64.EFI` on amd64, `BOOTAA64.EFI` on arm64 — so it is found automatically by the firmware. The kernel lives at `EFI/leon/kernel.efi` and is discovered as a regular boot entry; every other entry on the ESP is chainloadable too.

For Secure Boot, generate a personal key set, enroll the `.esl` files in the firmware, and sign the staged tree — full instructions in `docs/secure-boot.md` (`scripts/sign.sh setup`, then `make sign` after every rebuild).

The bootloader reads its configuration from `EFI/leon/boot.toml` on the same volume. `lbt config set timeout 5` writes the config to `~/.config/leon/boot.toml` and mirrors it onto every mounted EFI System Partition, so the next boot picks it up.

---

</details>

<details>
<summary>Binaries</summary>

## Binaries

| Artifact | Architecture | Description |
|----------|--------------|-------------|
| `target/<uefi-target>/release/lbl.efi` | amd64 / arm64 | Chainloading bootloader PE32+ image |
| `kernel/target/<uefi-target>/release/lbl-kernel.efi` | amd64 / arm64 | EFI-stub kernel PE32+ image |
| `target/<musl-target>/release/lbt` | host | Build tool (`make lbt`) — discovery, image builders, geometry |
| `target/<musl-target>/release/lbc` | host | Config + boot control (`make lbc`) — `config set/get`, ESP staging |
| `target/<musl-target>/release/lbm` | host | Menuconfig editor (`make lbm`) — menuconfig TUI for `boot.toml` |
| `build/esp/EFI/BOOT/BOOTX64.EFI` | amd64 | Staged, UEFI-canonical boot file |
| `build/esp/EFI/BOOT/BOOTAA64.EFI` | arm64 | Staged, UEFI-canonical boot file |
| `build/esp/EFI/leon/kernel.efi` | both | Kernel as a discovered boot entry |
| `build/leon-esp.img` | both | GPT ESP image with an ESP partition (`make esp`) |

`lbt`, `lbc`, and `lbm` are std (host) binaries built for the ecosystem target (`x86_64`/`aarch64`-`unknown-linux-musl`) and live outside the ESP: `make stage`, `make esp`, and `make qemu` build only the bootloader + kernel and never need them (or their transitive dependencies).

`lbm` is the menuconfig-style TUI for editing the boot config — `lbm ~/.config/leon/boot.toml` (optionally followed by the bootloader's `entries.jsonc` to fill the entry picker). It is a pure-Rust `cursive`/crossterm app (no ncurses, no embedded Python), themed strict black-and-white, editing exactly the keys the bootloader parses (`timeout`, `splash`, `default_entry`, `theme`, `entries_file`). `lbc` and `lbt` remain the non-interactive config/staging and build tools.

**Release profile:**

```toml
[profile.release]
panic = "abort"      # No unwinding
lto = true           # Link-time optimization
opt-level = "z"      # Optimize for size
codegen-units = 1    # Single codegen unit
strip = true         # Strip debug symbols
```

---

</details>

<details>
<summary>Building</summary>

## Building

Leon ships five build front-ends. All of them auto-detect `amd64`/`arm64` from the host and select the correct UEFI target. No external `clang`/`lld` is required — the `rust-lld` bundled with rustup links the UEFI targets.

| ARCH | UEFI target | Boot file | QEMU |
|------|-------------|-----------|------|
| `amd64` | `x86_64-unknown-uefi` | `BOOTX64.EFI` | `qemu-system-x86_64` + OVMF |
| `arm64` | `aarch64-unknown-uefi` | `BOOTAA64.EFI` | `qemu-system-aarch64` + AAVMF |

Both the bootloader and the kernel target the same UEFI target for a given architecture.

**Rust targets:**

```sh
rustup target add x86_64-unknown-uefi
rustup target add aarch64-unknown-uefi
rustup target add x86_64-unknown-linux-musl   # host `lbt`
rustup target add aarch64-unknown-linux-musl  # host `lbt`
```

The bootloader and kernel are UEFI applications (`*-unknown-uefi`); the host tools (`lbt`, `lbc`, `lbm`) follow the ecosystem convention (`*-unknown-linux-musl`, `/system` prefix, clang/llvm). They are pure Rust and have no Python/pyo3 dependency.

### Make

```sh
make build                    # bootloader + kernel + lbt + lbc + lbm, auto arch
make lbt                      # lbt only (host, musl target)
make lbc                      # lbc only (host, musl target)
make lbm                      # lbm only (host, musl target)
make stage                    # ESP tree at build/esp (bootloader + kernel only, or DESTDIR=/mnt/esp)
make install DESTDIR=/mnt/esp # staged install onto a mounted ESP
make ARCH=arm64 build         # cross-build for arm64
make test                     # generic host tests (workspace, default features)
make tui-test                 # menuconfig regression suite for lbm
make clippy                   # lint everything with -D warnings
make qemu                     # boot under QEMU/OVMF or QEMU/AAVMF
make esp                      # GPT ESP image (mtools + fdisk/sgdisk)
make sign                     # stage + sign the ESP tree for Secure Boot
make clean
```

`make build`/`make lbt|lbc|lbm` compile the host tools too. `lbt`/`lbc`/`lbm` honor `PROFILE` (default `release`, the same profile the bootloader and kernel build). `make stage`/`install`/`esp`/`qemu` deliberately skip the host tools and only produce the ESP tree.

| Variable | Default | Description |
|----------|---------|-------------|
| `ARCH` | host (`amd64`/`arm64`) | Override the target architecture |
| `PROFILE` | `release` | Cargo profile |
| `DESTDIR` | — | Staged install root |
| `PREFIX` | `/system` | Install prefix |
| `SYSROOT` | `/` | Build root (the actual root, never a rootfs dir) |

### Ninja

```sh
ninja -f build.ninja                    # build + stage ESP tree under builddir/
ninja -f build.ninja builddir/install   # copy the staged tree to builddir/install
```

Ninja has no `-D` override; edit `DESTDIR`/`PREFIX` at the top of `build.ninja` to target another root.

### Meson

```sh
./scripts/crossgen.sh                              # generate cross.txt for host arch
meson setup builddir --cross-file cross.txt
meson compile -C builddir                   # lbl.efi + lbl-kernel.efi + lbt
meson install -C builddir
```

The Meson `lbt` target is a host cargo build for the musl target; like the Makefile it needs no network for dependencies (all of `lbt`'s deps are crates.io, pinned in `Cargo.lock`).

### Cargo (direct)

```sh
cargo build --target x86_64-unknown-uefi --release
cargo build --target aarch64-unknown-uefi --release
```

### CMake toolchain

```sh
cmake -B build -DCMAKE_TOOLCHAIN_FILE=toolchain.cmake -DCMAKE_SYSTEM_NAME=Generic
```

The CMake file performs the same arch detection and exposes `LEON_UEFI_TARGET` for any future C component. `scripts/pkgconfig.sh` is a passthrough exposing the unified `/system` convention (`PKG_CONFIG_SYSROOT_DIR=/`, `PKG_CONFIG_LIBDIR=/system/lib/pkgconfig`) — Leon itself has no C dependencies.

---

</details>

<details>
<summary>Filesystem</summary>

## Filesystem

```
EFI/
├── BOOT/
│   ├── BOOTX64.EFI        # lbl chainloader (amd64)
│   └── BOOTAA64.EFI       # lbl chainloader (arm64)
└── leon/
    ├── kernel.efi         # EFI-stub kernel (a discovered boot entry)
    ├── boot.toml          # boot config (written by `lbc config set`)
    ├── entries.jsonc      # discovered boot entries (written every boot)
    └── bootinfo.json      # geometry + resolved config record (every boot)

var/
└── logs/
    └── leon/
        └── log.md         # unified boot log (Markdown, capped at 64 KiB)
```

The bootloader cannot reach the installed OS's real `/var/logs` before the kernel boots, so the log is written to the ESP and continued by the kernel/init chain on the real root filesystem.

---

</details>

<details>
<summary>Testing</summary>

## Testing

```sh
# Lint everything (bootloader + kernel + lbt, both feature modes)
make clippy
make ARCH=arm64 clippy

# Cargo, all targets, deny warnings
cargo clippy --all-targets --target x86_64-unknown-uefi -- -D warnings
cargo clippy --all-targets --target aarch64-unknown-uefi -- -D warnings

# Host-side unit tests
cargo test -p lbt            # CLI, discovery, geometry, boot-config parity
cargo test -p lbm            # menuconfig round-trip + bootloader-parser parity
cargo test -p leon-common    # the shared no_std boot.toml parser
cargo test -p lbc
make tui-test                 # lbm menuconfig regression suite

# Format check
cargo fmt --check
```

The `lbc` and `lbm` test suites include a parity test that feeds everything `lbc config set`/`lbm` serializes through the bootloader's own `leon_common::boot_config` parser, so the host-written file and the loader's reader can never drift apart — including the single-quoted TOML literal strings the serializer emits for backslash-heavy paths like `\EFI\leon\entries.jsonc`.

**Runtime verification:**

| Method | Command | What it verifies |
|--------|---------|------------------|
| Menuconfig editor | `make tui-test` | `lbm` boot-config round-trip and bootloader-parser parity |
| QEMU/OVMF | `make qemu` | Silent chainload to the kernel marker under emulation |
| QEMU/AAVMF | `make ARCH=arm64 qemu` | The same on arm64 |
| QEMU Secure Boot | `make esp` + snakeoil-signed images (`docs/secure-boot.md`) | Menu warning, signed-kernel boot, unsigned-kernel `ACCESS_DENIED` reporting |
| Real hardware | `make install DESTDIR=/mnt/esp` | Firmware logo preserved end-to-end |

---

</details>

<details>
<summary>Structure</summary>

## Structure

```
Leon/
├── Cargo.toml              # Workspace (lbl v0.7.0, edition 2024)
├── LICENSE                 # MIT License
├── PROMPT.md               # Design blueprint
├── README.md               # This documentation
├── common/
│   └── src/
│       ├── lib.rs          # Re-exports the shared ABI
│       ├── geometry.rs     # Framebuffer / Bgrt / PixelFormat shared ABI
│       └── boot_config.rs  # Shared no_std parser for \EFI\leon\boot.toml
├── lbt/
│   ├── Cargo.toml          # Leon Build Tool (host, std; pure Rust)
│   └── src/
│       ├── main.rs         # Slim entry: CLI dispatch
│       ├── cli/            # Command tree + argument parsing
│       ├── discovery.rs    # ESP + boot-entry discovery (lsblk / mount table)
│       ├── geometry.rs     # Geometry + sysfs/BMP/dump parsing (host mirror)
│       └── commands/       # info / discover / build / image builders
├── lbc/
│   ├── Cargo.toml          # Leon Boot Configuration (host, std)
│   └── src/
│       ├── main.rs         # Slim entry: CLI dispatch
│       ├── cli/            # Command tree + argument parsing
│       ├── boot_config.rs  # Host boot.toml model + sync to mounted ESPs
│       └── commands/       # config set/get, stage, boot control
├── lbm/
│   ├── Cargo.toml          # Leon Boot Menuconfig (host TUI, cursive/crossterm)
│   └── src/main.rs         # menuconfig editor for boot.toml
├── src/
│   ├── main.rs             # Module wiring + uefi_main
│   ├── boot/
│   │   ├── mod.rs          # run(): config -> discover -> record -> menu -> chainload
│   │   ├── config.rs       # Read + validate \EFI\leon\boot.toml
│   │   ├── entries.rs      # \EFI\*.efi discovery -> entries.jsonc (JSONC)
│   │   ├── image.rs        # LoadImage / StartImage chainloading; SB rejection reporting
│   │   └── menu.rs         # Optional splash menu (boxed, colored, countdown bar, SB warning)
│   ├── secure_boot.rs      # SecureBoot/SetupMode state + warning text
│   ├── firmware/
│   │   ├── bgrt.rs         # ACPI RSDP/XSDT/BGRT + BMP dimensions
│   │   └── gop.rs          # Frame buffer capture (no set_mode)
│   ├── record/
│   │   ├── mod.rs          # Geometry record module
│   │   └── dump.rs         # Writes \EFI\leon\bootinfo.json
│   └── logger/
│       └── mod.rs          # Silent, capped Markdown logger
├── kernel/
│   ├── Cargo.toml          # lbl-kernel v0.7.0 (uefi 0.39, EFI stub)
│   ├── .cargo/config.toml  # Per-target rustflags (x86-64-v3 / generic)
│   └── src/
│       ├── main.rs         # EFI-stub kernel entry
│       ├── gop.rs          # GOP capture (mirrors bootloader)
│       ├── bgrt.rs         # BGRT discovery (mirrors bootloader)
│       ├── memmap.rs       # Memory map validation after EBS
│       └── marker.rs       # Green marker + free_corner(), honors Bgrx/Rgbx
├── docs/
│   ├── lbt.1               # Man page for the build tool
│   ├── lbl.1               # Man page for the bootloader
│   ├── leon-common.7       # Man page for the shared ABI + boot config
│   └── secure-boot.md      # Secure Boot signing + enrollment guide
├── keys/                   # Self-signed Secure Boot key set (git-ignored; scripts/sign.sh setup)
├── Makefile / env.mk       # Make front-end (auto arch; UEFI + musl)
├── build.ninja             # Ninja front-end
├── meson.build             # Meson front-end (lbl.efi + lbl-kernel.efi + lbt)
├── scripts/crossgen.sh / cross.txt# Meson cross-file generator
├── toolchain.cmake         # CMake arch detection
├── scripts/pkgconfig.sh            # /system pkg-config passthrough (no C deps)
├── .cargo/config.toml      # Per-target rustflags (rust-lld)
└── scripts/
    ├── run_qemu.sh         # QEMU/OVMF or QEMU/AAVMF boot
    ├── make_esp.sh         # GPT ESP image (mtools + fdisk/sgdisk)
    ├── sign.sh             # Secure Boot key setup + self-signing
    └── gen-esl.py          # X.509 -> EFI_SIGNATURE_LIST (.esl) writer
```

---

</details>

<details>
<summary>Dependencies</summary>

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `uefi` | 0.39 | Boot services, GOP, config tables, Simple File System, `LoadImage`/`StartImage`, global allocator (bootloader + kernel) |
| `leon-common` | 0.7.0 | Shared `Framebuffer`/`Bgrt`/`PixelFormat` ABI + the `boot.toml` parser (workspace) |
| `cursive` / `crossterm` | — | The `lbm` menuconfig editor (pure Rust, no ncurses/embedded Python) |
| `anyhow` / `toml` / `serde_json` | — | Host-tool CLI, config serialization, and JSON output |

The bootloader depends only on `uefi` and `leon-common`. The kernel depends on `uefi` and `leon-common` (`default-features = false`, keeping the `boot.toml` parser which needs `alloc` out of its code). `lbt`/`lbc`/`lbm` are std host binaries with no optional features.

---

</details>

<details>
<summary>Contributing</summary>

## Contributing

Leon on [[**`[GitHub]`**]](https://github.com/Mapuse). Issues and pull requests are welcome.

```sh
git clone https://github.com/Mapuse/Leon.git
cd Leon
make build
make clippy
```

Follow existing code style. No comments unless requested. Silence rules apply: the bootloader never prints to the screen, and every failure path must route through the log file.

---

</details>

## Credits

**`[Leon]`** is part of the **`[Cudane]`** ecosystem.

- **`[Cudane]`** — The Distribution.
- **`[Cesar]`** — The Init System.
- **`[MCX]`** — Package Manager.

## License

**MIT License** ─ See [**`[LICENSE]`**](https://github.com/Mapuse/.github/blob/profile/LICENSE) for More Details.
