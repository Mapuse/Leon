project('lbl', 'rust',
  version : '0.7.0',
  license : 'MIT',
  default_options : [
    'optimization=3',
    'strip=true',
  ],
)

# ── Auto-detect architecture via gen-cross.sh ──────────────────────────
gen_cross = find_program('gen-cross.sh')
run_command(gen_cross, check : true)

build_dir = meson.project_build_root() / 'cargo'
src_dir = meson.project_source_root()

custom_target('lbl.efi',
  build_by_default : true,
  build_always_stale : true,
  install : true,
  install_dir : get_option('prefix') / 'EFI' / 'BOOT',
  command : ['sh', '-c',
    'case $$(uname -m) in x86_64) T=x86_64-unknown-uefi;; aarch64) T=aarch64-unknown-uefi;; *) echo "unsupported: $$(uname -m)"; exit 1;; esac; CARGO_TARGET_DIR={0} cargo build --release --locked --target $$T --manifest-path {1}/Cargo.toml && cp {0}/$$T/release/lbl.efi @OUTPUT@'
    .format(build_dir, src_dir),
  ],
  output : 'lbl.efi',
)

custom_target('lbl-kernel.efi',
  build_by_default : true,
  build_always_stale : true,
  install : true,
  install_dir : get_option('prefix') / 'EFI' / 'leon',
  command : ['sh', '-c',
    'case $$(uname -m) in x86_64) T=x86_64-unknown-uefi;; aarch64) T=aarch64-unknown-uefi;; *) echo "unsupported: $$(uname -m)"; exit 1;; esac; CARGO_TARGET_DIR={0} cargo build --release --locked --target $$T --manifest-path {1}/kernel/Cargo.toml && cp {0}/$$T/release/lbl-kernel.efi @OUTPUT@'
    .format(build_dir / 'kernel', src_dir),
  ],
  output : 'lbl-kernel.efi',
)

# lbt — host-side build tool (themes/plugins/boot config). Built for the
# ecosystem target (`*-unknown-linux-musl`) with the opt-in `python` feature;
# `cps` comes in as a git dependency (Cargo.lock pins its rev), same as with
# the Makefile path. The python-free static-pie variant is `lbt-core`.
custom_target('lbt',
  build_by_default : true,
  build_always_stale : true,
  install : true,
  install_dir : get_option('bindir'),
  command : ['sh', '-c',
    'case $$(uname -m) in x86_64) T=x86_64-unknown-linux-musl;; aarch64) T=aarch64-unknown-linux-musl;; *) echo "unsupported: $$(uname -m)"; exit 1;; esac; '
    'PYO3_CONFIG_FILE=$$( [ -f "{2}" ] && echo "{1}/share/leon/pyo3-config.toml" || echo "/etc/lbt/pyo3-musl-config.toml" ); '
    'CARGO_TARGET_DIR={0} cargo build --release --locked --features python --target $$T --manifest-path {1}/lbt/Cargo.toml && cp {0}/$$T/release/lbt @OUTPUT@'
    .format(build_dir / 'lbt', src_dir, src_dir),
  ],
  output : 'lbt',
)

# Install the committed pyo3 config into the system-wide location so Meson
# installs include a sane default for packaging. DESTDIR-aware path.
install_data('{0}/lbt/pyo3-musl-config.toml'.format(src_dir), install_dir : get_option('prefix') / 'etc' / 'lbt')

custom_target('lbt-core',
  build_by_default : true,
  build_always_stale : true,
  install : true,
  install_dir : get_option('bindir'),
  command : ['sh', '-c',
    'case $$(uname -m) in x86_64) T=x86_64-unknown-linux-musl;; aarch64) T=aarch64-unknown-linux-musl;; *) echo "unsupported: $$(uname -m)"; exit 1;; esac; CARGO_TARGET_DIR={0} cargo build --release --locked --target $$T --manifest-path {1}/lbt/Cargo.toml && cp {0}/$$T/release/lbt @OUTPUT@'
    .format(build_dir / 'lbt', src_dir),
  ],
  output : 'lbt-core',
)
