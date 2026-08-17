#!/bin/sh
# scripts/crossgen.sh — Generate Meson cross-file for Leon with auto-detection
# Usage: ./scripts/crossgen.sh [output_path]
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-${SCRIPT_DIR}/cross.txt}"

# ── Auto-detect architecture ───────────────────────────────────────────
HOST_ARCH=$(uname -m)
case "$HOST_ARCH" in
  x86_64)
    UEFI_TARGET="x86_64-unknown-uefi"
    MESON_CPU="x86_64"
    ;;
  aarch64)
    UEFI_TARGET="aarch64-unknown-uefi"
    MESON_CPU="aarch64"
    ;;
  *)
    echo "error: unsupported architecture: $HOST_ARCH (supported: x86_64, aarch64)" >&2
    exit 1
    ;;
esac

cat > "$OUT" <<EOF
[binaries]
rust = 'rustc'
cargo = 'cargo'

[properties]
uefi_target = '${UEFI_TARGET}'

[host_machine]
system = 'efi'
cpu_family = '${MESON_CPU}'
cpu = '${MESON_CPU}'
endian = 'little'
EOF

echo "cross.txt generated for ${MESON_CPU} → ${OUT}"
