#!/usr/bin/env python3
# leon_menu.py — Leon's default boot-manager TUI.
#
# A full-screen, keyboard-driven preview of what Leon renders at boot: the
# discovered boot entries, the live framebuffer + BGRT geometry, and the boot
# config. It is pure stdlib (no curses, no third-party modules) so it runs
# inside the statically-linked musl CPython that `make lbt` embeds.
#
# Context is handed in by `lbt tui` through environment variables:
#   LEON_FB_WIDTH/HEIGHT/STRIDE/FORMAT     — live framebuffer geometry
#   LEON_BGRT_RECT, LEON_LOGO_CENTER_X/Y   — ACPI BGRT logo placement
#   LEON_BGRT_STATUS, LEON_BGRT_TYPE       — BGRT metadata
#   LEON_BOOT_ENTRIES                      — one `label<TAB>path` per line
#   LEON_BOOT_TIMEOUT/DEFAULT/SPLASH       — boot config
# Every key is optional: standalone (`python3 leon_menu.py`) it reads the same
# sysfs the bootloader uses and shows a friendly empty/placeholder state.

import os
import sys
import time
import signal

try:
    import select
except ImportError:  # minimal embedded static builds without the select extension
    select = None

try:
    import termios
    import tty as _tty
except ImportError:  # minimal embedded builds without the termios extension
    termios = None
    _tty = None


# ── palette ────────────────────────────────────────────────────────────────
ACCENT = (125, 211, 252)    # #7dd3fc — Leon sky
TEXT = (226, 232, 240)
MUTED = (148, 163, 184)
DIM = (82, 82, 82)
BG = (10, 13, 18)
BORDER = (58, 70, 92)
OK = (74, 222, 128)
WARN = (250, 204, 21)
ERR = (248, 113, 113)

# Box-drawing + accents.
TLC, TRC, BLC, BRC = "┌", "┐", "└", "┘"
HLINE, VLINE = "─", "│"
TM, RT, LT, CROSS = "┬", "┤", "├", "┼"
SEL = "▶"
BRAND = "LEON"

BOLD = "\x1b[1m"
RESET = "\x1b[0m"
HIDE = "\x1b[?25l"
SHOW = "\x1b[?25h"
ALT_ON = "\x1b[?1049h"
ALT_OFF = "\x1b[?1049l"
HOME_POS = "\x1b[H"


def _style(rgb, bold=False, bg=None, dim=False):
    parts = [f"38;2;{rgb[0]};{rgb[1]};{rgb[2]}"]
    if bg:
        parts.append(f"48;2;{bg[0]};{bg[1]};{bg[2]}")
    if bold:
        parts.append("1")
    if dim:
        parts.append("2")
    return "\x1b[" + ";".join(parts) + "m"


def _fg(rgb):
    return _style(rgb)


def _term_size():
    try:
        cols, rows = os.get_terminal_size()
        return cols, rows
    except OSError:
        return 80, 24


def _visible_len(s):
    """Display width of a string, ignoring ANSI SGR escapes."""
    if "\x1b" not in s:
        return len(s)
    n = 0
    i = 0
    while i < len(s):
        if s[i] == "\x1b":
            j = s.find("m", i)
            if j < 0:
                j = len(s)
            i = j + 1
            continue
        n += 1
        i += 1
    return n


def _trunc(s, width):
    """Truncate to `width` visible columns, keeping ANSI escapes intact."""
    if width <= 0:
        return ""
    if "\x1b" not in s:
        if len(s) <= width:
            return s
        if width <= 1:
            return s[:1]
        return s[: width - 1] + "…"
    out = []
    n = 0
    i = 0
    while i < len(s):
        if s[i] == "\x1b":
            j = s.find("m", i)
            if j < 0:
                j = len(s)
            out.append(s[i : j + 1])
            i = j + 1
            continue
        if n < width:
            out.append(s[i])
            n += 1
        elif n == width and width > 1:
            out.append("…")
            n += 1
        i += 1
    return "".join(out)


def _center(s, width):
    return s if len(s) >= width else s + " " * (width - len(s))


class Term:
    """Owns the terminal: raw input, alt screen, cursor, resize handling."""

    def __init__(self):
        self.fd = sys.stdin.fileno()
        self.tty = sys.stdin.isatty() and sys.stdout.isatty()
        self.saved = None
        self.cols, self.rows = _term_size()
        self.dirty = True

    def _on_resize(self, *_):
        self.cols, self.rows = _term_size()
        self.dirty = True

    def __enter__(self):
        if self.tty and termios is not None:
            try:
                self.saved = termios.tcgetattr(self.fd)
                _tty.setraw(self.fd)
            except (termios.error, OSError):
                self.saved = None
            try:
                signal.signal(signal.SIGWINCH, self._on_resize)
            except (ValueError, OSError, AttributeError):
                pass
            sys.stdout.write(ALT_ON + HIDE)
            sys.stdout.flush()
        return self

    def __exit__(self, *exc):
        if self.tty:
            if self.saved is not None and termios is not None:
                try:
                    termios.tcsetattr(self.fd, termios.TCSADRAIN, self.saved)
                except (termios.error, OSError):
                    pass
            sys.stdout.write(SHOW + ALT_OFF + RESET + "\n")
            sys.stdout.flush()

    def read_byte(self, timeout=0.0):
        """Read one byte. timeout>0 returns b'' when nothing arrives."""
        if timeout > 0 and select is None:
            return self._read_byte_poll(timeout)
        try:
            if timeout > 0:
                r, _, _ = select.select([self.fd], [], [], timeout)
                if not r:
                    return b""
            return os.read(self.fd, 1)
        except (OSError, ValueError):
            return b""

    def _read_byte_poll(self, timeout):
        """Timed read without `select`: flip the fd non-blocking and poll."""
        try:
            os.set_blocking(self.fd, False)
        except (OSError, ValueError):
            pass
        try:
            deadline = time.monotonic() + timeout
            while True:
                try:
                    return os.read(self.fd, 1)
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        return b""
                    time.sleep(0.005)
                except OSError:
                    return b""
        finally:
            try:
                os.set_blocking(self.fd, True)
            except (OSError, ValueError):
                pass


class Key:
    """A decoded key press. `name` is a stable id, `char` the printable char."""
    __slots__ = ("name", "char")

    def __init__(self, name, char=None):
        self.name = name
        self.char = char


def _decode_csi(params, final):
    """Map a CSI sequence to a key name."""
    if params:
        p0 = params[0]
        if ";" in p0 or ":" in p0:
            return _decode_csi([p0.split(";")[0]], final)
        if final == "~":
            table = {
                "1": "home", "2": "insert", "3": "delete", "4": "end",
                "5": "pgup", "6": "pgdn", "7": "home", "8": "end",
                "11": "f1", "12": "f2", "13": "f3", "14": "f4", "15": "f5",
                "17": "f6", "18": "f7", "19": "f8", "20": "f9", "21": "f10",
                "23": "f11", "24": "f12",
            }
            return table.get(p0)
    if final == "A":
        return "up"
    if final == "B":
        return "down"
    if final == "C":
        return "right"
    if final == "D":
        return "left"
    if final == "H":
        return "home"
    if final == "F":
        return "end"
    if final == "Z":
        return "backtab"
    return None


def read_key(term, timeout=0.0):
    """Read one key press, decoding escapes for arrows/function keys/etc."""
    b = term.read_byte(timeout)
    if not b:
        return None
    c = b[0]
    if c == 0x1b:  # ESC — bare Esc, CSI, SS3, or Alt+key
        nxt = term.read_byte(0.05)
        if not nxt:
            return Key("esc")
        n = nxt[0]
        if n == 0x5b:  # [
            buf = b""
            while True:
                x = term.read_byte(0.05)
                if not x:
                    break
                buf += x
                if 0x40 <= x[0] <= 0x7e:
                    break
            s = buf.decode("latin-1")
            params = s[:-1].split(";") if len(s) > 1 else []
            name = _decode_csi(params, s[-1] if s else "")
            return Key(name) if name else Key("esc")
        if n == 0x4f:  # O — SS3
            x = term.read_byte(0.05)
            if not x:
                return Key("esc")
            ch = chr(x[0])
            ss3 = {"P": "f1", "Q": "f2", "R": "f3", "S": "f4",
                   "H": "home", "F": "end",
                   "A": "up", "B": "down", "C": "right", "D": "left"}
            return Key(ss3[ch]) if ch in ss3 else Key("esc")
        if n >= 0x20:  # Alt+<key>
            return Key("alt+" + chr(n).lower(), chr(n))
        return Key("esc")
    if c == 0x7f:
        return Key("backspace")
    if c in (0x0d, 0x0a):
        return Key("enter")
    if c == 0x09:
        return Key("tab")
    if c == 0x00:
        return Key("ctrl+space", " ")
    if 1 <= c <= 26:
        return Key("ctrl+" + chr(96 + c))
    if c in (0x1c, 0x1d, 0x1e, 0x1f):
        return Key("ctrl+" + "\\]^_"[c - 0x1c])
    if 0x20 <= c < 0x7f:
        return Key("char", chr(c))
    return None


# ── context ────────────────────────────────────────────────────────────────
def _read_sysfs_int(path):
    try:
        with open(path) as f:
            return int(f.read().strip().split(",")[0])
    except (OSError, ValueError):
        return 0


def load_context():
    """Geometry/entries/config from env (set by `lbt tui`) or live sysfs."""
    e = os.environ
    width = int(e.get("LEON_FB_WIDTH", "0") or 0)
    height = int(e.get("LEON_FB_HEIGHT", "0") or 0)
    stride = int(e.get("LEON_FB_STRIDE", "0") or 0)
    fmt = e.get("LEON_FB_FORMAT", "")

    if not width and os.path.exists("/sys/class/graphics/fb0/virtual_size"):
        try:
            with open("/sys/class/graphics/fb0/virtual_size") as f:
                w, _, h = f.read().strip().partition(",")
                width, height = int(w), int(h)
        except (OSError, ValueError):
            pass
        stride = _read_sysfs_int("/sys/class/graphics/fb0/stride") or width
        bpp = _read_sysfs_int("/sys/class/graphics/fb0/bits_per_pixel")
        fmt = f"{bpp}bpp"

    logo = None
    bgrt = e.get("LEON_BGRT_RECT", "")
    if bgrt and bgrt != "none":
        try:
            x0, y0, x1, y1 = (int(v) for v in bgrt.split(","))
            logo = (x0, y0, x1, y1)
        except ValueError:
            logo = None
    if logo is None and os.path.exists("/sys/firmware/acpi/bgrt/xoffset"):
        try:
            with open("/sys/firmware/acpi/bgrt/xoffset") as f:
                x0 = int(f.read().strip())
            with open("/sys/firmware/acpi/bgrt/yoffset") as f:
                y0 = int(f.read().strip())
            iw = _read_sysfs_int("/sys/firmware/acpi/bgrt/image_width")
            ih = _read_sysfs_int("/sys/firmware/acpi/bgrt/image_height")
            logo = (x0, y0, x0 + iw, y0 + ih)
        except (OSError, ValueError):
            logo = None

    entries = []
    for line in e.get("LEON_BOOT_ENTRIES", "").splitlines():
        if not line.strip():
            continue
        label, _, path = line.partition("\t")
        entries.append({"label": label.strip(), "path": path.strip()})

    def bval(key):
        return e.get("LEON_BOOT_" + key, "").strip()

    return {
        "width": width,
        "height": height,
        "stride": stride,
        "format": fmt,
        "logo": logo,
        "entries": entries,
        "timeout": bval("TIMEOUT"),
        "default": bval("DEFAULT"),
        "splash": bval("SPLASH"),
    }


# ── state ──────────────────────────────────────────────────────────────────
class State:
    def __init__(self, ctx):
        self.ctx = ctx
        self.entries = ctx["entries"]
        self.sel = 0
        self.offset = 0
        self.focus = "menu"
        self.query = ""
        self.show_help = False
        self.show_details = True
        self.status = ""
        self.status_color = MUTED
        self.visible = list(range(len(self.entries)))
        self.details_scroll = 0

    def matches(self):
        q = self.query.strip().lower()
        if not q:
            self.visible = list(range(len(self.entries)))
        else:
            self.visible = [i for i in range(len(self.entries))
                            if q in self.entries[i]["label"].lower()
                            or q in self.entries[i]["path"].lower()]
        self.clamp()

    def clamp(self):
        if not self.visible:
            self.sel = 0
            self.offset = 0
            return
        if self.sel >= len(self.visible):
            self.sel = len(self.visible) - 1
        if self.sel < 0:
            self.sel = 0

    def move(self, delta):
        if not self.visible:
            return
        self.sel = (self.sel + delta) % len(self.visible)
        self.ensure_visible()

    def ensure_visible(self):
        if self.sel < self.offset:
            self.offset = self.sel
        elif self.sel >= self.offset + self._rows():
            self.offset = self.sel - self._rows() + 1

    def _rows(self):
        return max(1, self.ctx.get("_menu_rows", 3))

    def current(self):
        if not self.visible or self.sel >= len(self.visible):
            return None
        return self.entries[self.visible[self.sel]]

    def boot(self):
        entry = self.current()
        if not entry:
            self.status = "No boot entry selected"
            self.status_color = WARN
            return
        self.status = "WOULD BOOT: " + entry["label"] + "  (chainload \\" + \
            entry["path"].replace("/", "\\") + ")"
        self.status_color = OK


# ── rendering ──────────────────────────────────────────────────────────────
def _wrap(s, width):
    if len(s) <= width:
        return [s]
    return [s[i:i + width] for i in range(0, len(s), width)]


def render(state, cols, rows):
    """Compose the whole frame as a list of styled lines (incl. borders)."""
    if cols < 34:
        return [_fg(ERR) + _trunc(f"terminal too small: {cols}x{rows}", cols) + RESET]
    ctx = state.ctx
    w = cols

    help_open = state.show_help
    if help_open:
        menu_rows = max(3, rows - 20)
    else:
        menu_rows = max(3, rows - 7)
    ctx["_menu_rows"] = menu_rows

    details = state.show_details and w >= 88
    if details:
        right_w = max(26, int(w * 0.34))
        left_w = w - right_w - 3
    else:
        right_w = 0
        left_w = w - 2
    help_h = max(1, rows - menu_rows - 6)

    lines = []

    def hline(width, color=BORDER):
        return _fg(color) + HLINE * max(1, width) + RESET

    # header
    brand = " " + BOLD + "❖ " + BRAND + " Boot Manager" + RESET
    geom = f"{ctx['width'] or '?'}x{ctx['height'] or '?'}"
    if ctx["stride"]:
        geom += f" stride {ctx['stride']}"
    if ctx["format"]:
        geom += " · " + ctx["format"]
    if details:
        lines.append(_fg(BORDER) + TLC + hline(left_w) + TM + hline(right_w) + TRC + RESET)
        lines.append(_fg(BORDER) + VLINE + RESET +
                     _fg(TEXT) + _trunc(brand, left_w) + RESET +
                     _fg(BORDER) + VLINE + RESET +
                     _fg(BOLD) + _trunc(geom, right_w) + RESET +
                     _fg(BORDER) + VLINE + RESET)
    else:
        lines.append(_fg(BORDER) + TLC + hline(left_w) + TRC + RESET)
        lines.append(_fg(BORDER) + VLINE + RESET +
                     _fg(TEXT) + _trunc(brand, left_w) + RESET +
                     _fg(BORDER) + VLINE + RESET)

    sub = []
    if ctx["logo"]:
        x0, y0, x1, y1 = ctx["logo"]
        sub.append(f"BGRT logo {x1 - x0}x{y1 - y0} @ {x0},{y0}")
    else:
        sub.append("no BGRT logo")
    if ctx["timeout"]:
        sub.append("timeout " + ctx["timeout"] + "s")
    if ctx["splash"]:
        sub.append("splash " + ("on" if ctx["splash"] == "true" else "off"))
    if ctx["default"]:
        sub.append("default: " + ctx["default"])
    sub_line = " · ".join(sub)
    right_bit = f" {len(ctx['entries'])} entries"
    # Live clock keeps the frame alive even when idle ("always displaying").
    clock = time.strftime("%H:%M:%S")
    sub_text = _style(MUTED) + _trunc(" " + sub_line, left_w - len(clock) - 2) + RESET
    sub_pad = max(0, left_w - _visible_len(sub_text) - len(clock))
    sub_row = sub_text + " " * sub_pad + _style(DIM) + clock + RESET
    if details:
        lines.append(_fg(BORDER) + LT + hline(left_w) + CROSS + hline(right_w) + RT + RESET)
        lines.append(_fg(BORDER) + VLINE + RESET +
                     sub_row +
                     _fg(BORDER) + VLINE + RESET +
                     _fg(DIM) + _trunc(right_bit, right_w) + RESET +
                     _fg(BORDER) + VLINE + RESET)
    else:
        lines.append(_fg(BORDER) + LT + hline(left_w) + RT + RESET)
        lines.append(_fg(BORDER) + VLINE + RESET +
                     sub_row +
                     _fg(BORDER) + VLINE + RESET)

    # body separator
    if details:
        lines.append(_fg(BORDER) + LT + hline(left_w) + CROSS + hline(right_w) + RT + RESET)
    else:
        lines.append(_fg(BORDER) + LT + hline(left_w) + RT + RESET)

    # menu + detail rows
    n = len(state.visible)
    for r in range(menu_rows):
        idx = state.offset + r
        if idx < n:
            i = state.visible[idx]
            entry = ctx["entries"][i]
            selected = idx == state.sel
            num = str(r + 1)
            label = entry["label"]
            # scroll indicators on the fold edges (non-selected rows)
            mark = " "
            if not selected and idx == state.offset and state.offset > 0:
                mark = "▴"
            elif not selected and idx == state.offset + menu_rows - 1 and idx < n - 1:
                mark = "▾"
            if selected:
                body = _style(TEXT, bold=True, bg=(56, 132, 168)) + " " + SEL + " " + \
                    _trunc(f"{num}. {label}", left_w - 4) + RESET
            else:
                body = _style(TEXT, dim=True) + " " + mark + " " + \
                    _trunc(f"{num}. {label}", left_w - 4) + RESET
            left = _fg(BORDER) + VLINE + RESET + _trunc(body, left_w) + \
                _fg(BORDER) + VLINE + RESET
        else:
            left = _fg(BORDER) + VLINE + RESET + " " * left_w + _fg(BORDER) + VLINE + RESET
        if details:
            right = _render_detail(state, r, right_w)
            lines.append(left + right)
        else:
            lines.append(left)

    # separator after body
    if details:
        lines.append(_fg(BORDER) + LT + hline(left_w) + CROSS + hline(right_w) + RT + RESET)
    else:
        lines.append(_fg(BORDER) + LT + hline(left_w) + RT + RESET)

    # status / search / help region
    if state.focus == "search":
        lines.append(_render_search(state, w))
        for _ in range(help_h - 1):
            lines.append(" " * w)
    elif help_open:
        lines.extend(_render_help(w, help_h))
    else:
        lines.append(_render_status(state, w))
        for _ in range(help_h - 1):
            lines.append(" " * w)

    # bottom border
    lines.append(_fg(BORDER) + BLC + hline(w - 2) + BRC + RESET)
    return lines


def _render_detail(state, r, width):
    entry = state.current()
    if not entry:
        return _fg(BORDER) + VLINE + RESET + " " * width + _fg(BORDER) + VLINE + RESET
    body = []
    body.append(_style(TEXT, bold=True) + _trunc(" " + entry["label"], width - 2))
    body.append("")
    body.append(_style(MUTED) + " path:" + RESET)
    body.extend(" " + _trunc(ln, width - 2) for ln in _wrap(entry["path"], width - 2))
    body.append("")
    body.append(_style(MUTED) + " vendor:" + RESET)
    vendor = entry["path"].split("/")[1] if "/" in entry["path"] else "?"
    body.append(" " + _trunc(vendor, width - 2))
    body.append("")
    body.append(_style(MUTED) + " target:" + RESET)
    body.append(" " + _trunc("\\" + entry["path"].replace("/", "\\"), width - 2))
    idx = r + state.details_scroll
    if idx < 0 or idx >= len(body):
        return _fg(BORDER) + VLINE + RESET + " " * width + _fg(BORDER) + VLINE + RESET
    return _fg(BORDER) + VLINE + RESET + _trunc(body[idx], width) + _fg(BORDER) + VLINE + RESET


def _render_search(state, w):
    body = "  / " + state.query + "▌"
    hint = "  enter: filter · esc: cancel · ⌫: delete"
    line = _style(TEXT, bg=(56, 132, 168)) + _trunc(body, w - 2) + RESET
    return _fg(BORDER) + VLINE + RESET + line + " " * max(0, w - 4 - len(body) - len(hint)) + \
        _style(DIM) + _trunc(hint, w - 2) + RESET + _fg(BORDER) + VLINE + RESET


def _render_status(state, w):
    body = state.status or f" {len(state.visible)}/{len(state.entries)} entries"
    if state.query.strip():
        body += f"  ·  filter: /{state.query}"
    keys = "↑↓/jk move · ↵ boot · / search · r refresh · h help · q quit"
    vis = _visible_len(body)
    avail = max(0, w - 4 - len(keys))
    pad = max(1, w - 4 - vis - len(keys))
    line = _fg(BORDER) + VLINE + RESET + \
        _style(state.status_color) + _trunc(body, avail) + RESET + " " * pad + \
        _style(DIM) + keys + RESET + _fg(BORDER) + VLINE + RESET
    return line


def _render_help(w, height):
    help_lines = [
        ("↑↓ or j/k", "move selection"),
        ("↵ or space", "boot selected entry (preview)"),
        ("PgUp/PgDn, ←/→", "page through entries"),
        ("Home/End or g/G", "first / last entry"),
        ("1–9, 0", "jump to entry number"),
        ("/", "search-as-you-type filter"),
        ("n / N", "next / previous match"),
        ("r", "re-read geometry + entries"),
        ("s", "splash boot preview"),
        ("d", "toggle detail panel"),
        ("h or ?", "this help"),
        ("Tab / Shift+Tab", "move focus"),
        ("Ctrl+L", "redraw"),
        ("q, Ctrl+C, Ctrl+Q", "quit"),
    ]
    out = [_fg(BORDER) + VLINE + RESET + " " * (w - 2) + _fg(BORDER) + VLINE + RESET]
    for k, d in help_lines[: max(0, height - 2)]:
        row = _style(ACCENT, bold=True) + _trunc(k, 18) + RESET + "  " + _trunc(d, w - 26)
        out.append(_fg(BORDER) + VLINE + RESET + row + " " * max(0, w - 2 - _visible_len(row)) +
                   _fg(BORDER) + VLINE + RESET)
    while len(out) < height:
        out.append(_fg(BORDER) + VLINE + RESET + " " * (w - 2) + _fg(BORDER) + VLINE + RESET)
    return out


def paint(lines, cols, rows):
    """Write the frame to stdout with the cursor parked at the top."""
    buf = HOME_POS
    for r in range(rows):
        if r < len(lines):
            buf += lines[r] + RESET + "\r\n"
        else:
            buf += " " * cols + "\r\n"
    sys.stdout.write(buf)
    sys.stdout.flush()


# ── splash preview ─────────────────────────────────────────────────────────
def splash_preview(state, term):
    state.focus = "splash"
    entry = state.current()
    name = entry["label"] if entry else "Leon"
    cols, rows = _term_size()
    steps = 24
    for i in range(steps + 1):
        progress = int(40 * i / steps)
        bar = "█" * progress + "░" * (40 - progress)
        lines = [" " * cols] * rows
        mid = max(3, rows // 2 - 3)
        lines[mid] = _center(_style(ACCENT, bold=True) + "❖ " + BRAND + RESET, cols)
        lines[mid + 1] = _center(_style(MUTED) + "flicker-free boot manager" + RESET, cols)
        lines[mid + 3] = _center("Booting " + name + "…", cols)
        lines[mid + 4] = _center(_style(OK) + bar + RESET, cols)
        lines[mid + 6] = _center(_style(DIM) + "press any key to return" + RESET, cols)
        paint(lines, cols, rows)
        k = term.read_byte(0.05)
        if k:
            break
        time.sleep(0.03)
    state.focus = "menu"
    state.dirty = True


# ── actions ────────────────────────────────────────────────────────────────
def _cycle_focus(state, delta=1):
    order = ["menu", "details", "keys"]
    i = order.index(state.focus) if state.focus in order else 0
    state.focus = order[(i + delta) % len(order)]
    state.dirty = True


def _action_search(state, key):
    name = key.name
    if name == "char" and key.char in "nN" and state.query.strip():
        state.move(1 if key.char == "n" else -1)
    elif name == "char":
        state.query += key.char
    elif name in ("backspace", "delete"):
        state.query = state.query[:-1]
    elif name in ("enter", "tab"):
        state.focus = "menu"
    elif name == "esc":
        state.query = ""
        state.focus = "menu"
    elif name == "ctrl+u":
        state.query = ""
    elif name == "ctrl+w":
        state.query = state.query.rsplit(" ", 1)[0]
    elif name in ("up", "k"):
        state.move(-1)
    elif name in ("down", "j"):
        state.move(1)
    elif name == "home":
        if state.visible:
            state.sel = 0
    elif name == "end":
        if state.visible:
            state.sel = len(state.visible) - 1
    elif name in ("q", "ctrl+q", "ctrl+c") and not state.query:
        return "quit"
    state.matches()
    return None


def action(state, key, term):
    name = key.name
    if name == "char" and key.char in "jkJK ":
        key = Key("space" if key.char == " " else key.char.lower())
        name = key.name
    if state.focus == "search":
        return _action_search(state, key)

    if state.focus == "keys":
        if name in ("esc", "h", "?"):
            state.show_help = False
            state.focus = "menu"
        elif name == "tab":
            _cycle_focus(state)
        elif name == "backtab":
            _cycle_focus(state, -1)
        elif name in ("q", "ctrl+q", "ctrl+c"):
            return "quit"
        else:
            state.focus = "menu"
            return action(state, key, term)
        return None

    if state.focus == "details":
        if name == "esc":
            state.focus = "menu"
        elif name in ("up", "k", "ctrl+p"):
            state.details_scroll = max(0, state.details_scroll - 1)
        elif name in ("down", "j", "ctrl+n"):
            state.details_scroll += 1
        elif name in ("pgup", "left"):
            state.details_scroll = max(0, state.details_scroll - 5)
        elif name in ("pgdn", "right"):
            state.details_scroll += 5
        elif name == "tab":
            _cycle_focus(state)
        elif name == "backtab":
            _cycle_focus(state, -1)
        elif name in ("q", "ctrl+q", "ctrl+c"):
            return "quit"
        else:
            state.focus = "menu"
            return action(state, key, term)
        return None

    # focus == menu
    if name in ("up", "k", "ctrl+p"):
        state.move(-1)
    elif name in ("down", "j", "ctrl+n"):
        state.move(1)
    elif name in ("pgup", "left", "ctrl+b"):
        state.move(-max(1, state._rows() - 1))
    elif name in ("pgdn", "right", "ctrl+f"):
        state.move(max(1, state._rows() - 1))
    elif name == "home":
        if state.visible:
            state.sel = 0
    elif name == "end":
        if state.visible:
            state.sel = len(state.visible) - 1
    elif name == "alt+up":
        state.move(-10)
    elif name == "alt+down":
        state.move(10)
    elif name in ("enter", "space", "b"):
        state.boot()
    elif name == "tab":
        _cycle_focus(state)
    elif name == "backtab":
        _cycle_focus(state, -1)
    elif name in ("q", "ctrl+q", "ctrl+c", "ctrl+x", "ctrl+z"):
        return "quit"
    elif name == "esc":
        if state.query.strip():
            state.query = ""
            state.matches()
            state.status = ""
        else:
            return "quit"
    elif name == "f1":
        state.show_help = not state.show_help
    elif name == "f5":
        return "refresh"
    elif name == "f10":
        return "quit"
    elif name == "f11":
        state.show_details = not state.show_details
    elif name == "ctrl+l":
        pass  # redraw
    elif name == "ctrl+s":
        splash_preview(state, term)
    elif name == "char":
        c = key.char
        if c == "/":
            state.focus = "search"
            state.query = ""
            state.status = ""
        elif c in "0123456789":
            n = 10 if c == "0" else int(c)
            if state.visible and n <= len(state.visible):
                state.sel = n - 1
                state.ensure_visible()
        elif c in "gG":
            if state.visible:
                state.sel = 0 if c == "g" else len(state.visible) - 1
                state.ensure_visible()
        elif c in "nN" and state.query.strip():
            state.move(1 if c == "n" else -1)
        elif c in "rR":
            return "refresh"
        elif c in "sS":
            splash_preview(state, term)
        elif c in "dD":
            state.show_details = not state.show_details
        elif c in "h?":
            state.show_help = not state.show_help
            state.focus = "keys" if state.show_help else "menu"
        elif c in "cC":
            state.query = ""
            state.matches()
        elif c in "qQ":
            return "quit"
    return None


# ── entry point (cps contract: `run()`) ────────────────────────────────────
def run(*args, **kwargs):
    """cps TUI contract. Returns True on a clean exit."""
    state = State(load_context())
    term = Term()
    with term:
        try:
            # Live-refresh cadence in ms (0 disables): the frame repaints on a
            # timer even with no keys, so the TUI is *always displaying* (the
            # sub-line clock ticks, resize is picked up without SIGWINCH).
            idle_ms = int(os.environ.get("LEON_TUI_IDLE_MS", "1000") or "0")
            last_paint = time.monotonic()
            while True:
                cols, rows = _term_size()
                if (cols, rows) != (term.cols, term.rows):
                    term.cols, term.rows = cols, rows
                    term.dirty = True
                if term.dirty:
                    paint(render(state, cols, rows), cols, rows)
                    term.dirty = False
                    last_paint = time.monotonic()
                elif idle_ms > 0 and time.monotonic() - last_paint >= idle_ms / 1000.0:
                    term.dirty = True
                key = read_key(term, 0.25)
                if key is None:
                    continue
                result = action(state, key, term)
                if result == "quit":
                    break
                if result == "refresh":
                    state.ctx = load_context()
                    state.entries = state.ctx["entries"]
                    state.matches()
                    state.status = "refreshed geometry + boot entries"
                    state.status_color = MUTED
                term.dirty = True
        except (KeyboardInterrupt, EOFError):
            pass
    return True


if __name__ == "__main__":
    sys.exit(0 if run() else 1)
