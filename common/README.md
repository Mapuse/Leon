# leon-common

Shared firmware-geometry types and boot config parser between `lbl` (the
bootloader), `lbl-kernel` (the EFI-stub kernel it chainloads), and `lbt` (the
host build tool).

There is **no handoff blob**: the kernel is a plain UEFI application that
acquires the GOP frame buffer and the ACPI BGRT itself after being started with
`LoadImage`/`StartImage`. What the crates share is the *representation* of
those firmware facts, so both sides (and the separate splash project) agree on
pixel formats, frame buffer geometry, and logo metadata. The crate has **zero
dependencies**, is `#![no_std]`, and all shared types are `#[repr(C)]`.

## Types

### `PixelFormat`

Mirrors the UEFI GOP pixel formats:

| Variant     | Meaning                                                     |
|-------------|-------------------------------------------------------------|
| `Rgbx = 0`  | 24-bit RGB, 32-bit stride, last byte unused                 |
| `Bgrx = 1`  | 24-bit BGR, 32-bit stride, last byte unused (most common on UEFI) |
| `Bitmask = 2` | Custom; red/green/blue masks describe the layout         |
| `BltOnly = 3` | Cannot be drawn to directly; only `BLT` is available      |

### `Framebuffer`

`base`, `size`, `width`, `height`, `stride` (pixels per scan line, `>= width`),
`format: PixelFormat`. The frame buffer must be mapped **without clearing it** —
the firmware logo is still in that memory and must stay visible (flicker-free
boot).

```rust
let offset = framebuffer.offset(x, y); // byte offset of pixel (x, y) in the buffer
```

### `Bgrt`

`image_address` (physical address of the raw BMP, "BM" magic), `offset_x` /
`offset_y` (logo position on screen), `image_width` / `image_height` (parsed
from the BMP header), `status` (bit 0 = logo displayed), `image_type` (0 =
bitmap).

```rust
let (x0, y0, x1, y1) = bgrt.rect(); // exclusive bounding box of the logo
```

## `boot.toml` parser

With the `boot-config` feature (on by default) the crate hosts a `no_std`,
allocation-based parser for `\EFI\leon\boot.toml`, shared between:

- `lbl`, which reads + validates the file at every boot (a broken file yields
  defaults, never a blocked boot), and
- `lbt`, which writes it via serde/toml.

The parser accepts both TOML basic strings (`"..."`, with escapes) and TOML
literal strings (`'...'`, no escapes) — the latter is what serde/toml emits for
backslash-heavy values such as `entries_file = '\EFI\leon\entries.jsonc'`.
Unknown keys are ignored so the bootloader tolerates forward-compatible files.

## Host integration

The `common` crate is shared by both firmware and host tooling. It provides
the exact same ABI for `Framebuffer`, `Bgrt`, and `boot.toml` semantics to:

- `lbt`, which discovers ESP contents, reads `bootinfo.json`, and writes
  `boot.toml`.
- `lbc`, which stages EFI trees and exercises the shared parser in
  regression tests.
- `lbm`, the menuconfig-style editor for `boot.toml`, backed by the same
  parser.

This shared dependency prevents drift between what the host tools write and
what the UEFI bootloader reads.

## `bootinfo.json` record

Every boot, the loader writes the live geometry + resolved config as JSON on
the boot volume at `\EFI\leon\bootinfo.json` (best-effort; the boot proceeds
even if the write fails). Host tools such as `lbt` read this file to mirror
*exactly* what this machine's firmware provided:

```json
{
  "framebuffer": { "base": 0, "size": 0, "width": 0, "height": 0,
                    "stride": 0, "format": "Bgrx" },
  "bgrt": { "image_address": 0, "offset_x": 0, "offset_y": 0,
            "image_width": 0, "image_height": 0, "status": 0, "image_type": 0 }
         | null,
  "boot_config": { "timeout": 0, "default_entry": "...", "theme": "...",
                   "splash": false, "entries_file": "..." }
}
```
