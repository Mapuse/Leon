"""A tiny stdlib-only UI framework inspired by npyscreen semantics.

This provides a minimal Form/Widget/ListBox API and a terminal renderer
usable from embedded Python without external deps.
"""
from __future__ import annotations

import os
import sys
import time
import signal

try:
    import termios
    import tty as _tty
except Exception:
    termios = None
    _tty = None


class Key:
    __slots__ = ("name", "char")

    def __init__(self, name, char=None):
        self.name = name
        self.char = char


def _term_size():
    try:
        return os.get_terminal_size()
    except OSError:
        return (80, 24)


class Term:
    def __init__(self):
        self.fd = sys.stdin.fileno()
        self.tty = sys.stdin.isatty() and sys.stdout.isatty()
        self.saved = None
        self.cols, self.rows = _term_size()
        self.dirty = True

    def __enter__(self):
        if self.tty and termios is not None:
            try:
                self.saved = termios.tcgetattr(self.fd)
                _tty.setraw(self.fd)
            except Exception:
                self.saved = None
            try:
                signal.signal(signal.SIGWINCH, self._on_resize)
            except Exception:
                pass
            sys.stdout.write("\x1b[?1049h\x1b[?25l")
            sys.stdout.flush()
        return self

    def __exit__(self, *exc):
        if self.tty:
            if self.saved is not None and termios is not None:
                try:
                    termios.tcsetattr(self.fd, termios.TCSADRAIN, self.saved)
                except Exception:
                    pass
            sys.stdout.write("\x1b[?25h\x1b[?1049l\x1b[0m\n")
            sys.stdout.flush()

    def _on_resize(self, *_):
        self.cols, self.rows = _term_size()
        self.dirty = True

    def read_byte(self, timeout=0.0):
        try:
            if timeout > 0:
                import select

                r, _, _ = select.select([self.fd], [], [], timeout)
                if not r:
                    return b""
            return os.read(self.fd, 1)
        except Exception:
            return b""


def read_key(term: Term, timeout=0.0):
    b = term.read_byte(timeout)
    if not b:
        return None
    c = b[0]
    if c == 0x1b:
        nxt = term.read_byte(0.05)
        if not nxt:
            return Key("esc")
        n = nxt[0]
        if n == 0x5b:
            buf = b""
            while True:
                x = term.read_byte(0.05)
                if not x:
                    break
                buf += x
                if 0x40 <= x[0] <= 0x7e:
                    break
            s = buf.decode("latin-1")
            final = s[-1] if s else ""
            if final == "A":
                return Key("up")
            if final == "B":
                return Key("down")
            if final == "C":
                return Key("right")
            if final == "D":
                return Key("left")
            return Key("esc")
        if n == 0x4f:
            x = term.read_byte(0.05)
            if not x:
                return Key("esc")
            ss3 = {80: "f1", 81: "f2", 82: "f3", 83: "f4"}
            return Key(ss3.get(x[0], "esc"))
        if n >= 0x20:
            return Key("alt+" + chr(n).lower(), chr(n))
        return Key("esc")
    if c == 0x7f:
        return Key("backspace")
    if c in (0x0d, 0x0a):
        return Key("enter")
    if c == 0x09:
        return Key("tab")
    if 1 <= c <= 26:
        return Key("ctrl+" + chr(96 + c))
    if 0x20 <= c < 0x7f:
        return Key("char", chr(c))
    return None


class Widget:
    def __init__(self, name=None):
        self.name = name or "widget"
        self.focus = False
        self.width = 0
        self.height = 0

    def render(self, w, h) -> list[str]:
        """Return list of lines for this widget, length <= h."""
        return [" " * w for _ in range(h)]

    def handle_key(self, key: Key):
        return None


class ListBox(Widget):
    def __init__(self, items=None, name=None):
        super().__init__(name)
        self.items = items or []
        self.sel = 0
        self.offset = 0
        self.query = ""
        self.visible = list(range(len(self.items)))

    def render(self, w, h):
        out = []
        n = len(self.visible)
        for r in range(h):
            idx = self.offset + r
            if idx < n:
                item = self.items[self.visible[idx]]
                label = item.get("label") if isinstance(item, dict) else str(item)
                prefix = "> " if idx == self.sel else "  "
                s = prefix + label
                out.append(s[:w].ljust(w))
            else:
                out.append(" " * w)
        return out

    def handle_key(self, key: Key):
        if key is None or not self.items:
            return None
        if key.name in ("up", "k"):
            self.sel = max(0, self.sel - 1)
            if self.sel < self.offset:
                self.offset = self.sel
        elif key.name in ("down", "j"):
            self.sel = min(len(self.items) - 1, self.sel + 1)
            if self.sel >= self.offset + self.height:
                self.offset = self.sel - self.height + 1
        elif key.name in ("home",):
            self.sel = 0
            self.offset = 0
        elif key.name in ("end",):
            self.sel = max(0, len(self.items) - 1)
            self.offset = max(0, len(self.items) - self.height)
        return None

    def set_items(self, items):
        self.items = items
        self.query = ""
        self.visible = list(range(len(self.items)))
        self.sel = 0
        self.offset = 0

    def filter(self, q: str):
        q = q.strip().lower()
        if not q:
            self.visible = list(range(len(self.items)))
        else:
            self.visible = [i for i in range(len(self.items)) if q in (self.items[i].get("label","") if isinstance(self.items[i], dict) else str(self.items[i])).lower() or q in (self.items[i].get("path","") if isinstance(self.items[i], dict) else "").lower()]
        self.sel = 0
        self.offset = 0


class DetailWidget(Widget):
    def __init__(self, name=None):
        super().__init__(name)
        self.entry = None

    def set_entry(self, e):
        self.entry = e

    def render(self, w, h):
        out = []
        if not self.entry:
            for _ in range(h):
                out.append(" " * w)
            return out
        label = self.entry.get("label", "") if isinstance(self.entry, dict) else str(self.entry)
        path = self.entry.get("path", "") if isinstance(self.entry, dict) else ""
        out.append((" " + label)[:w].ljust(w))
        out.append(" " * w)
        out.append((" path:")[:w].ljust(w))
        chunk_width = max(1, w - 1)
        for ln in (path[i:i+chunk_width] for i in range(0, len(path), chunk_width)):
            if len(out) >= h:
                break
            out.append((" " + ln)[:w].ljust(w))
        while len(out) < h:
            out.append(" " * w)
        return out


class Form:
    def __init__(self, title=""):
        self.title = title
        self.children: list[Widget] = []
        self.focus_index = 0

    def add(self, w: Widget):
        self.children.append(w)

    def _layout(self, cols, rows):
        # simple vertical layout: stack children equally
        n = max(1, len(self.children))
        content_rows = max(1, rows - 3)
        h_each = max(1, content_rows // n)
        positions = []
        y = 2
        for i, c in enumerate(self.children):
            c.width = cols - 2
            c.height = h_each
            positions.append((1, y, c.width, c.height))
            y += h_each
        return positions

    def render(self, cols, rows):
        lines = []
        top = f" {self.title} ".ljust(max(0, cols - 2))
        lines.append("┌" + "─" * max(0, cols - 2) + "┐")
        lines.append("│" + top[: max(0, cols - 2)] + "│")
        content_rows = max(1, rows - 3)
        # Two-column layout: left + right when two children present.
        if len(self.children) >= 2 and cols >= 40:
            left_w = max(20, int((cols - 3) * 0.66))
            right_w = cols - 3 - left_w
            if right_w < 16:
                left_w = max(20, cols - 3 - 16)
                right_w = cols - 3 - left_w
            if right_w >= 16:
                self.children[0].width = left_w
                self.children[0].height = content_rows
                self.children[1].width = right_w
                self.children[1].height = content_rows
                left_lines = self.children[0].render(left_w, content_rows)
                right_lines = self.children[1].render(right_w, content_rows)
                for i in range(content_rows):
                    l = left_lines[i] if i < len(left_lines) else " " * left_w
                    r = right_lines[i] if i < len(right_lines) else " " * right_w
                    lines.append("│" + l[:left_w].ljust(left_w) + " " + r[:right_w].ljust(right_w) + "│")
            else:
                positions = self._layout(cols, rows)
                for (x, y, w, h), child in zip(positions, self.children):
                    child_lines = child.render(w, h)
                    for ln in child_lines:
                        lines.append("│" + ln[:w].ljust(w) + "│")
        else:
            positions = self._layout(cols, rows)
            for (x, y, w, h), child in zip(positions, self.children):
                child_lines = child.render(w, h)
                for ln in child_lines:
                    lines.append("│" + ln[:w].ljust(w) + "│")
        # fill remaining rows
        while len(lines) < rows - 1:
            lines.append("│" + " " * max(0, cols - 2) + "│")
        lines.append("└" + "─" * max(0, cols - 2) + "┘")
        return lines


def paint(lines, cols, rows):
    buf = "\x1b[H"
    for r in range(rows):
        if r < len(lines):
            buf += lines[r] + "\r\n"
        else:
            buf += " " * cols + "\r\n"
    sys.stdout.write(buf)
    sys.stdout.flush()


def run_form(form: Form, idle_ms=250):
    term = Term()
    with term:
        last_paint = time.monotonic()
        while True:
            cols, rows = _term_size()
            if term.dirty:
                paint(form.render(cols, rows), cols, rows)
                term.dirty = False
                last_paint = time.monotonic()
            elif idle_ms > 0 and time.monotonic() - last_paint >= idle_ms / 1000.0:
                term.dirty = True
            key = read_key(term, 0.25)
            if key is None:
                continue
            # send key to focused widget
            if form.children:
                w = form.children[0]
                res = w.handle_key(key)
                if res == "quit":
                    break
            if key.name in ("q", "ctrl+c", "ctrl+q"):
                break
            term.dirty = True
