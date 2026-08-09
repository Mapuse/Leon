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
    OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
    OVMF_VARS="${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS_4M.fd}"
    MACHINE="-machine q35"
    CPU="-cpu max"
    GPU=""   # q35 provides a VGA with a linear-framebuffer GOP
    ;;
  aarch64|arm64)
    QEMU_BIN="qemu-system-aarch64"
    OVMF_CODE="${OVMF_CODE:-/usr/share/AAVMF/AAVMF_CODE.fd}"
    OVMF_VARS="${OVMF_VARS:-/usr/share/AAVMF/AAVMF_VARS.fd}"
    # AAVMF needs RAM below 4G (highmem=off) and a display device: virt has
    # no VGA, and virtio-gpu only exposes a BLT-only GOP, so use ramfb — a
    # linear framebuffer GOP the firmware drives. Without it the loader finds
    # no GOP and aborts.
    MACHINE="-machine virt,highmem=off"
    CPU="-cpu cortex-a57"
    GPU="-device ramfb"
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
# OVMF_VARS must be writable; keep the original pristine. Cache per arch and
# per source file: switching ARCH (amd64 vs arm64 use different flash sizes)
# or pointing OVMF_VARS at the Secure Boot snakeoil vars must never reuse a
# stale copy.
VARS_CACHE="build/ovmf_vars_${HOST_ARCH}_$(basename "$OVMF_VARS")"
if [ ! -f "$VARS_CACHE" ] || [ "$(stat -c %s "$VARS_CACHE" 2>/dev/null)" != "$(stat -c %s "$OVMF_VARS")" ]; then
    cp "$OVMF_VARS" "$VARS_CACHE"
fi

# Optional menuconfig preview: MENU_TIMEOUT=<seconds> rewrites the staged
# boot.toml so the boot-manager countdown runs for that long, holding the TUI
# on screen instead of auto-booting. `make qemu-preview` sets a long timeout
# for exactly this. The staged tree is a generated artifact, so patching it
# in place is safe; the next `make stage` restores the default.
if [ -n "${MENU_TIMEOUT:-}" ]; then
    if [ -f "build/esp/EFI/leon/boot.toml" ]; then
        sed -i -E "s/^timeout = [0-9]+/timeout = ${MENU_TIMEOUT}/" build/esp/EFI/leon/boot.toml
    else
        echo "run_qemu.sh: MENU_TIMEOUT set but build/esp/EFI/leon/boot.toml not found (run 'make stage' first)." >&2
        exit 1
    fi
fi

exec "$QEMU_BIN" \
    $MACHINE \
    $CPU \
    $GPU \
    -m 512M \
    -drive file="$OVMF_CODE",format=raw,if=pflash,readonly=on \
    -drive file="$VARS_CACHE",format=raw,if=pflash \
    -drive file=fat:rw:build/esp \
    -serial mon:stdio
