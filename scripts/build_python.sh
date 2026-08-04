#!/bin/sh
# build_python.sh — Build a static musl CPython (libpython) for Leon's host
# tool (`lbt`). Installs into PREFIX (default /system), optionally staged via
# DESTDIR. Writes a pyo3 config file so `make lbt` links against the static
# lib through PYO3_CONFIG_FILE.
# Usage: ./build_python.sh [--prefix PREFIX] [--destdir DIR] [--cache DIR]
set -e

PREFIX="${PREFIX:-/system}"
DESTDIR="${DESTDIR:-}"
PY_VER="${PY_VER:-3.14.2}"
PY_ABI="${PY_ABI:-3.14}"
MUSL_VER="${MUSL_VER:-1.2.5}"
NCURSES_VER="${NCURSES_VER:-6.5}"
CACHE="${CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/leon}"

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix)   PREFIX="$2";   shift 2 ;;
    --destdir)  DESTDIR="$2";  shift 2 ;;
    --cache)    CACHE="$2";    shift 2 ;;
    *) echo "usage: $0 [--prefix PREFIX] [--destdir DIR] [--cache DIR]" >&2; exit 1 ;;
  esac
done

case "$(uname -m)" in
  x86_64)
    CLANG_TARGET="x86_64-unknown-linux-musl"
    BUILD_TRIPLE="x86_64-linux-gnu"
    ASM_DIR="x86_64-linux-gnu"
    MULTIARCH="x86_64-linux-musl"
    ;;
  aarch64)
    CLANG_TARGET="aarch64-unknown-linux-musl"
    BUILD_TRIPLE="aarch64-linux-gnu"
    ASM_DIR="aarch64-linux-gnu"
    MULTIARCH="aarch64-linux-musl"
    ;;
  *)
    echo "error: unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

INSTALL_ROOT="$DESTDIR$PREFIX"
CONFIG_FILE="$INSTALL_ROOT/share/leon/pyo3-config.toml"

if [ -f "$CONFIG_FILE" ] && [ -f "$INSTALL_ROOT/lib/libpython$PY_ABI.a" ]; then
  echo "musl CPython already provisioned at $INSTALL_ROOT"
  exit 0
fi

mkdir -p "$CACHE"
if ! mkdir -p "$INSTALL_ROOT"; then
  echo "error: $PREFIX is not writable — rerun with --destdir DIR" >&2
  exit 1
fi

fetch() {
  url="$1"; out="$2"
  if [ ! -f "$out" ]; then
    echo "fetching $url"
    curl -fL --retry 3 -o "$out" "$url" || wget -O "$out" "$url"
  fi
}

PY_TARBALL="$CACHE/Python-$PY_VER.tgz"
MUSL_TARBALL="$CACHE/musl-$MUSL_VER.tar.gz"
NCURSES_TARBALL="$CACHE/ncurses-$NCURSES_VER.tar.gz"
fetch "https://www.python.org/ftp/python/$PY_VER/Python-$PY_VER.tgz" "$PY_TARBALL"
fetch "https://musl.libc.org/releases/musl-$MUSL_VER.tar.gz" "$MUSL_TARBALL"
fetch "https://invisible-mirror.net/archives/ncurses/ncurses-$NCURSES_VER.tar.gz" "$NCURSES_TARBALL"

MUSL_PREFIX="$CACHE/toolchain/musl"
if [ ! -x "$MUSL_PREFIX/bin/musl-gcc" ]; then
  echo "building musl $MUSL_VER"
  rm -rf "$CACHE/musl-build"
  mkdir -p "$CACHE/musl-build"
  tar -xzf "$MUSL_TARBALL" -C "$CACHE/musl-build" --strip-components=1
  (cd "$CACHE/musl-build" \
    && ./configure --prefix="$MUSL_PREFIX" --disable-shared \
    && make -j"$(nproc)" && make install)
  mkdir -p "$MUSL_PREFIX/include"
  cp -r /usr/include/linux "$MUSL_PREFIX/include/"
  cp -r /usr/include/asm-generic "$MUSL_PREFIX/include/"
  cp -r "/usr/include/$ASM_DIR/asm" "$MUSL_PREFIX/include/"
fi

if command -v clang >/dev/null 2>&1; then
  REAL_CC="clang --target=$CLANG_TARGET --sysroot=$MUSL_PREFIX"
else
  REAL_CC="$MUSL_PREFIX/bin/musl-gcc"
fi
MUSL_CC_WRAP="$CACHE/toolchain/musl-multiarch-gcc"
cat > "$MUSL_CC_WRAP" <<EOF
#!/bin/sh
if [ "\$1" = "--print-multiarch" ]; then
  echo "$MULTIARCH"
  exit 0
fi
exec $REAL_CC "\$@"
EOF
chmod +x "$MUSL_CC_WRAP"
export CC="$MUSL_CC_WRAP" AR=ar RANLIB=ranlib

# ncurses is required by CPython's `_curses` extension, which the host TUI
# needs for npyscreen. Build a wide-char, static copy into the musl sysroot so
# configure picks it up when building the interpreter. No run-time tools
# (progs/tic) are built: at runtime the embedded curses reads the host's
# terminfo database (e.g. /usr/share/terminfo).
if [ ! -f "$MUSL_PREFIX/lib/libncursesw.a" ]; then
  echo "building ncurses $NCURSES_VER"
  rm -rf "$CACHE/ncurses-build"
  mkdir -p "$CACHE/ncurses-build"
  tar -xzf "$NCURSES_TARBALL" -C "$CACHE/ncurses-build" --strip-components=1
  (cd "$CACHE/ncurses-build" \
    && ./configure \
      --host="$CLANG_TARGET" \
      --build="$BUILD_TRIPLE" \
      --prefix="$MUSL_PREFIX" \
      --enable-widec \
      --with-normal \
      --without-shared \
      --without-debug \
      --without-ada \
      --without-cxx \
      --without-manpages \
      --without-progs \
      --without-tests \
      --without-dlsym \
    && make -j"$(nproc)" \
    && make install)
fi

CONFIG_SITE="$CACHE/toolchain/config.site"
cat > "$CONFIG_SITE" <<'EOF'
ac_cv_file__dev_ptmx=yes
ac_cv_file__dev_ptc=no
EOF
export CONFIG_SITE

echo "building CPython $PY_VER for $CLANG_TARGET"
rm -rf "$CACHE/python-build"
mkdir -p "$CACHE/python-build"
tar -xzf "$PY_TARBALL" -C "$CACHE/python-build" --strip-components=1
BUILD_PYTHON="${BUILD_PYTHON:-$(command -v python3 || true)}"
(
  cd "$CACHE/python-build"
  ./configure \
    --host="$CLANG_TARGET" \
    --build="$BUILD_TRIPLE" \
    --prefix="$PREFIX" \
    --with-build-python="$BUILD_PYTHON" \
    --disable-ipv6 \
    --disable-shared \
    --with-static-libpython \
    --without-ensurepip \
    --without-doc-strings
  make -j"$(nproc)"
  make install DESTDIR="$DESTDIR"
)

# `_curses` is a hard requirement (the host TUI runs npyscreen). Fail loudly if
# configure didn't pick up the ncurses build above.
CURSES_SO=$(find "$CACHE/python-build/build" -name "_curses*.so" 2>/dev/null | head -n 1)
if [ -z "$CURSES_SO" ]; then
  echo "error: CPython built without _curses — ncurses was not detected" >&2
  exit 1
fi
echo "built $CURSES_SO"

mkdir -p "$INSTALL_ROOT/share/leon"
cat > "$CONFIG_FILE" <<EOF
version=$PY_ABI
implementation=CPython
shared=false
abi3=false
lib_name=python$PY_ABI
lib_dir=$INSTALL_ROOT/lib
pointer_width=64
build_flags=-C target-feature=+crt-static
suppress_build_script_link_lines=false
EOF

# System-wide copy for the shared cps/pyo3 tooling (same dir cps searches for
# /etc/leon). DESTDIR-aware so a staged install never touches the live /etc.
ETC_CONFIG="$DESTDIR/etc/lbt/pyo3-musl-config.toml"
mkdir -p "$(dirname "$ETC_CONFIG")"
cp "$CONFIG_FILE" "$ETC_CONFIG"

echo "provisioned musl CPython at $INSTALL_ROOT"
echo "pyo3 config: $CONFIG_FILE"
