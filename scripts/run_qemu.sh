#!/usr/bin/env bash
# Boot lbl in QEMU with UEFI firmware, using QEMU's built-in FAT driver to
# serve build/esp/ as the ESP. No disk image needed.
# Selects qemu-system-x86_64/OVMF or qemu-system-aarch64/AAVMF by host arch.
set -euo pipefail
cd "$(dirname "$0")/.."

# Honors the ARCH env var used by the Makefiles (amd64/arm64), defaulting to
# the host architecture.
HOST_ARCH="${ARCH:-$(uname -m)}"
case "$HOST_ARCH" in
  x86_64|amd64)
    QEMU_BIN="qemu-system-x86_64"
    OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE.fd}"
    OVMF_VARS="${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS.fd}"
    MACHINE="-machine q35"
    ;;
  aarch64|arm64)
    QEMU_BIN="qemu-system-aarch64"
    OVMF_CODE="${OVMF_CODE:-/usr/share/AAVMF/AAVMF_CODE.fd}"
    OVMF_VARS="${OVMF_VARS:-/usr/share/AAVMF/AAVMF_VARS.fd}"
    MACHINE="-machine virt"
    ;;
  *)
    echo "Unsupported architecture: $HOST_ARCH (supported: x86_64, aarch64)" >&2
    exit 1
    ;;
esac

if ! command -v "$QEMU_BIN" >/dev/null 2>&1; then
    echo "$QEMU_BIN not found." >&2
    exit 1
fi
if [ ! -f "$OVMF_CODE" ]; then
    echo "UEFI firmware not found at $OVMF_CODE." >&2
    echo "Or point OVMF_CODE/OVMF_VARS at your UEFI .fd files." >&2
    exit 1
fi

mkdir -p build
# OVMF_VARS must be writable; keep the original pristine.
if [ ! -f build/ovmf_vars.fd ]; then
    cp "$OVMF_VARS" build/ovmf_vars.fd
fi

exec "$QEMU_BIN" \
    $MACHINE \
    -cpu max \
    -m 512M \
    -drive file="$OVMF_CODE",format=raw,if=pflash,readonly=on \
    -drive file=build/ovmf_vars.fd,format=raw,if=pflash \
    -drive file=fat:ro:build/esp,format=raw \
    -serial mon:stdio
