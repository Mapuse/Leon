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
# 64 MiB GPT disk image with a single EFI System partition starting at sector
# 2048 (1 MiB alignment). A raw FAT volume is not picked up as a bootable disk
# by OVMF/AAVMF; firmware expects an ESP partition on a GPT (or MBR) disk.
dd if=/dev/zero of="$ESP" bs=1M count=64 status=none
if command -v sgdisk >/dev/null 2>&1; then
    sgdisk -o "$ESP" >/dev/null
    sgdisk -n 1:2048:0 -t 1:ef00 "$ESP" >/dev/null
elif command -v fdisk >/dev/null 2>&1; then
    printf 'g\nn\n1\n\n\nt\n1\nw\n' | fdisk "$ESP" >/dev/null
else
    echo "sgdisk or fdisk not found." >&2
    exit 1
fi
# The ESP partition lives at byte offset 1 MiB; mtools addresses it directly.
mformat -i "$ESP"@@1048576 -F -h 64 -s 32 ::
mmd -i "$ESP"@@1048576 ::/EFI ::/EFI/BOOT ::/EFI/leon
mcopy -i "$ESP"@@1048576 build/esp/EFI/BOOT/$BOOT_FILE ::/EFI/BOOT/
mcopy -i "$ESP"@@1048576 build/esp/EFI/leon/kernel.efi ::/EFI/leon/
echo "ESP image written: $ESP"
echo "For real hardware, copy build/esp/EFI/BOOT/$BOOT_FILE to your ESP's EFI/BOOT/"
echo "and build/esp/EFI/leon/kernel.efi to EFI/leon/kernel.efi."
