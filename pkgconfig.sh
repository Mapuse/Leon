#!/bin/bash
# Leon is a pure-Rust project: the bootloader and kernel have no C
# dependencies. This wrapper is kept only for API compatibility with the
# other build-system files; it exposes the unified /system convention.
export PKG_CONFIG_SYSROOT_DIR="/"
export PKG_CONFIG_LIBDIR="/system/lib/pkgconfig"
exec pkg-config "$@"
