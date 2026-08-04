REPO_ROOT   := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
ESP         := $(REPO_ROOT)/build/esp
SYSROOT     := /
PREFIX      ?= /system
DESTDIR     ?=

# ── Auto-detect host architecture ──────────────────────────────────────
HOST_ARCH_RAW := $(shell uname -m)
ifeq ($(HOST_ARCH_RAW),x86_64)
  HOST_ARCH    := amd64
  RUST_TARGET  := x86_64-unknown-linux-musl
  CLANG_TARGET := x86_64-unknown-linux-musl
  CMAKE_ARCH   := x86_64
  MESON_CPU    := x86_64
else ifeq ($(HOST_ARCH_RAW),aarch64)
  HOST_ARCH    := arm64
  RUST_TARGET  := aarch64-unknown-linux-musl
  CLANG_TARGET := aarch64-unknown-linux-musl
  CMAKE_ARCH   := aarch64
  MESON_CPU    := aarch64
else
  $(error Unsupported architecture: $(HOST_ARCH_RAW). Supported: x86_64, aarch64)
endif

# ── Architecture selection (override with ARCH=amd64|arm64) ─────────────
# The bootloader and kernel keep UEFI targets; only the host tool (`lbt`) and
# the wider ecosystem build for `*-unknown-linux-musl` (see RUST_TARGET above).
ARCH        ?= $(HOST_ARCH)
ifeq ($(ARCH),amd64)
  UEFI_TARGET   := x86_64-unknown-uefi
  BOOT_FILE     := BOOTX64.EFI
  QEMU_BIN      := qemu-system-x86_64
else ifeq ($(ARCH),arm64)
  UEFI_TARGET   := aarch64-unknown-uefi
  BOOT_FILE     := BOOTAA64.EFI
  QEMU_BIN      := qemu-system-aarch64
else
  $(error Unsupported architecture: $(ARCH). Supported: amd64, arm64)
endif

PROFILE     ?= release
# The loader and the EFI-stub kernel are both plain UEFI applications, built
# with the same UEFI toolchain. The kernel is chainloaded like any other boot
# entry, so it is installed at `\EFI\leon\kernel.efi` (not `kernel.elf`).
BL_EFI      := $(REPO_ROOT)/target/$(UEFI_TARGET)/$(PROFILE)/lbl.efi
KERNEL_EFI  := $(REPO_ROOT)/kernel/target/$(UEFI_TARGET)/$(PROFILE)/lbl-kernel.efi

# ── Unified ecosystem toolchain: musl-libc + llvm/clang, /system prefix ──
CC            := clang --target=$(CLANG_TARGET) --sysroot=$(SYSROOT)
CXX           := clang++ --target=$(CLANG_TARGET) --sysroot=$(SYSROOT)
AR            := llvm-ar
STRIP         := llvm-strip

CFLAGS        := -O2 -nostdinc -isystem $(SYSROOT)$(PREFIX)/include
CXXFLAGS      := -O2 -nostdinc++ -isystem $(SYSROOT)$(PREFIX)/include
LDFLAGS       := -L$(SYSROOT)$(PREFIX)/lib -Wl,-rpath,$(PREFIX)/lib

export PKG_CONFIG_SYSROOT_DIR := $(SYSROOT)
export PKG_CONFIG_LIBDIR      := $(SYSROOT)$(PREFIX)/lib/pkgconfig:$(SYSROOT)$(PREFIX)/share/pkgconfig
export PKG_CONFIG_PATH        :=
