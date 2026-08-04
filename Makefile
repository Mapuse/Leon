include env.mk

# ── cps: external package (lbt dependency), fetched from upstream ────────
CPS_URL ?= https://github.com/Mapuse/CPS
CPS_DIR ?= $(HOME)/.cargo/git/cps
CPS_REF ?= d8d5f7da49917ea7636147b8a65f3541311c45ab

# Clone-if-missing + pull-if-exists, then pin the checkout to CPS_REF.
$(CPS_DIR):
	git clone $(CPS_URL) $(CPS_DIR)

.PHONY: cps-update
cps-update: $(CPS_DIR)
	git -C $(CPS_DIR) fetch origin
	git -C $(CPS_DIR) checkout $(CPS_REF)

.PHONY: all build bootloader kernel lbt lbt-core python install-pyo3-config stage install install-man uninstall clean test clippy qemu esp

all: build

build: bootloader kernel lbt

bootloader:
	CARGO_TARGET_DIR=$(CURDIR)/target cargo build --locked --target $(UEFI_TARGET) --profile $(PROFILE)

kernel:
	$(MAKE) -C kernel TARGET=$(UEFI_TARGET) PROFILE=$(PROFILE) build

# musl CPython provisioning for the python-featured `lbt`. Installs into
# PREFIX (default /system); `PY_DESTDIR` is only a standard DESTDIR staging
# root for when PREFIX isn't writable.
PY_PREFIX  ?= $(PREFIX)
PY_DESTDIR ?=
PYO3_CONFIG := $(PY_DESTDIR)$(PY_PREFIX)/share/leon/pyo3-config.toml

# Committed, canonical pyo3 build config for the shared cps/pyo3 tooling.
# `make python` provisions a *resolved* config (absolute lib_dir) alongside the
# CPython it builds; this file is the system-wide default, installed to the
# same directory cps searches for /etc/leon (DESTDIR-aware, so it stages fine).
PYO3_MUSL_CONFIG := $(CURDIR)/lbt/pyo3-musl-config.toml
PYO3_SYSTEM_CONFIG := /etc/lbt/pyo3-musl-config.toml

.PHONY: python
python:
	./scripts/build_python.sh --prefix $(PY_PREFIX) --destdir "$(PY_DESTDIR)"

# Copy the committed pyo3 config into the system-wide cps location.
install-pyo3-config:
	install -d $(DESTDIR)/etc/lbt
	install -m 644 $(PYO3_MUSL_CONFIG) $(DESTDIR)/etc/lbt/pyo3-musl-config.toml

# Host-side companion tool (`lbt`), explicitly not a default member so the
# UEFI-only `cargo build`/`clippy` above never try to compile it. Two variants
# of the same crate share the binary name `lbt` but are built into *separate*
# target dirs so neither can clobber the other:
#   `make lbt`      — `python` feature (themes/plugins/TUIs) -> target/.../lbt
#   `make lbt-core` — python-free static-pie                -> target/lbt-core/.../lbt
# The python variant links the musl libpython that the `python` target
# provisions. Keep the dirs apart: a python-gated subcommand (`lbt theme ...`)
# on the python-free binary just reports `unrecognized subcommand`. Both honor
# `PROFILE` (default `release`, same as the bootloader/kernel).
lbt: cps-update
	# Prefer a locally-provisioned pyo3 config, fall back to the system-wide
	# `/etc/lbt/pyo3-musl-config.toml` so `cargo` can be invoked without
	# manual env flags when the system is preconfigured.
	PYO3_CONFIG_FILE=$$( [ -f "$(PYO3_CONFIG)" ] && echo "$(PYO3_CONFIG)" || echo "$(PYO3_SYSTEM_CONFIG)" ); \
	CARGO_TARGET_DIR=$(CURDIR)/target \
	cargo build --locked --target $(RUST_TARGET) -p lbt --features python --profile $(PROFILE)

lbt-core: cps-update
	CARGO_TARGET_DIR=$(CURDIR)/target/lbt-core cargo build --locked --target $(RUST_TARGET) -p lbt --profile $(PROFILE)

# Stage a ready-to-boot ESP tree under build/esp with the UEFI-canonical boot
# file name for this architecture plus the EFI-stub kernel at its path under
# \EFI\leon\. Only needs bootloader + kernel: `lbt` (and its cps clone) is not
# part of the ESP.
stage: bootloader kernel
	mkdir -p $(ESP)/EFI/BOOT $(ESP)/EFI/leon
	install -m 644 $(BL_EFI) $(ESP)/EFI/BOOT/$(BOOT_FILE)
	install -m 644 $(KERNEL_EFI) $(ESP)/EFI/leon/kernel.efi

# Install onto a mounted ESP (e.g. `make install DESTDIR=/mnt/esp`). The EFI
# tree goes at the ESP root (boot files must sit in \EFI\BOOT); only host-side
# artifacts (man pages) use the $(PREFIX) convention.
install: stage install-man install-pyo3-config
	install -d $(DESTDIR)/EFI/BOOT $(DESTDIR)/EFI/leon
	install -m 644 $(BL_EFI) $(DESTDIR)/EFI/BOOT/$(BOOT_FILE)
	install -m 644 $(KERNEL_EFI) $(DESTDIR)/EFI/leon/kernel.efi

install-man:
	install -d $(DESTDIR)$(PREFIX)/share/man/man1 $(DESTDIR)$(PREFIX)/share/man/man7
	install -m 644 docs/lbt.1 docs/lbl.1 $(DESTDIR)$(PREFIX)/share/man/man1/
	install -m 644 docs/leon-common.7 $(DESTDIR)$(PREFIX)/share/man/man7/

uninstall:
	rm -f $(DESTDIR)/EFI/BOOT/$(BOOT_FILE) $(DESTDIR)/EFI/leon/kernel.efi
	rm -f $(DESTDIR)$(PREFIX)/share/man/man1/lbt.1 $(DESTDIR)$(PREFIX)/share/man/man1/lbl.1 $(DESTDIR)$(PREFIX)/share/man/man7/leon-common.7

# Generic host test run: the host-testable workspace crates (`common`, `lbt`)
# with default features. The `leon` UEFI binary is excluded — a `#![no_main]`
# EFI application has no host test harness — and no target/feature flags are
# hard-coded here.
test:
	CARGO_TARGET_DIR=$(CURDIR)/target cargo test --locked --workspace --exclude leon

clippy:
	cargo clippy --all-targets --target $(UEFI_TARGET) -- -D warnings
	$(MAKE) -C kernel TARGET=$(UEFI_TARGET) clippy
	CARGO_TARGET_DIR=$(CURDIR)/target cargo clippy -p lbt --features python -- -D warnings
	CARGO_TARGET_DIR=$(CURDIR)/target cargo clippy -p lbt -- -D warnings

qemu: stage
	ARCH=$(ARCH) ./scripts/run_qemu.sh

esp: stage
	ARCH=$(ARCH) ./scripts/make_esp.sh

clean:
	cargo clean
	$(MAKE) -C kernel clean
	rm -rf build target/lbt-core
