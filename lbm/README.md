# lbm — Leon Boot Menuconfig

A `menuconfig`-style TUI for editing [**`[Leon]**`](https://github.com/Mapuse/Leon)'s
`boot.toml`, built with [**`[cursive]`**](https://github.com/gyscos/cursive)
The whole app, every dialog and sub-screen included, runs one strict high-contrast black-and-white theme —
there's no per-screen styling to drift out of sync.

## Build

```sh
rustup target add x86_64-unknown-linux-gnu   # or your host triple
cargo build --release
./target/release/lbm
```

Needs `cursive` 0.20 with the `crossterm-backend` feature (already set in
`Cargo.toml`) — no system ncurses dependency, matching Leon's own
pure-Rust `lbt`/`lbc`.

## Usage

```sh
lbm                                              # ~/.config/leon/boot.toml
lbm /path/to/boot.toml                           # explicit config path
lbm /path/to/boot.toml /mnt/esp/EFI/leon/entries.jsonc  # + discovered entries
```
