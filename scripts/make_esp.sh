#!/usr/bin/env bash
# Build a real bootable FAT ESP image from the staged boot volume (build/esp).
# Requires mtools. Useful for writing to a USB stick or testing with other
# firmware; for QEMU/OVMF you can skip this and use `make qemu`.
# Copies the correct UEFI boot file (BOOTX64.EFI / BOOTAA64.EFI) for the arch.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v mformat >/dev/null 2>&1; then
    echo "mtools not found." >&2
    exit 1
fi

# Honors the ARCH env var used by the Makefiles (amd64/arm64), defaulting to
# the host architecture. Cross-building an ESP image for another arch works.
HOST_ARCH="${ARCH:-$(uname -m)}"
case "$HOST_ARCH" in
  x86_64|amd64) BOOT_FILE="BOOTX64.EFI" ;;
  aarch64|arm64) BOOT_FILE="BOOTAA64.EFI" ;;
  *)
    echo "Unsupported architecture: $HOST_ARCH (supported: x86_64, aarch64)" >&2
    exit 1
    ;;
esac

ESP=build/leon-esp.img
rm -f "$ESP"
# 64 MiB FAT16 ESP.
dd if=/dev/zero of="$ESP" bs=1M count=64 status=none
mformat -i "$ESP" -F -h 64 -s 32 ::
mmd -i "$ESP" ::/EFI ::/EFI/BOOT ::/EFI/leon
mcopy -i "$ESP" build/esp/EFI/BOOT/$BOOT_FILE ::/EFI/BOOT/
mcopy -i "$ESP" build/esp/EFI/leon/kernel.efi ::/EFI/leon/
echo "ESP image written: $ESP"
echo "For real hardware, copy build/esp/EFI/BOOT/$BOOT_FILE to your ESP's EFI/BOOT/"
echo "and build/esp/EFI/leon/kernel.efi to EFI/leon/kernel.efi."
