#!/usr/bin/env python3
"""Prototype: leon menu built on the local stdlib `ui_framework`.

This is an initial port demonstrating a dynamic, widget-based UI without
third-party deps. It's intentionally small; we'll expand widgets and
layout once you're happy with direction.
"""
from __future__ import annotations

import time
from ui_framework import Form, ListBox, DetailWidget, Term, read_key, paint, _term_size
from leon_menu import load_context


def _sort_entries(entries):
    try:
        return sorted(entries, key=lambda e: e.get("label", "").lower())
    except Exception:
        return entries


def run(*args, **kwargs):
    ctx = load_context()
    entries = ctx.get("entries", [])
    entries = _sort_entries(entries)

    form = Form("❖ LEON Boot Manager")
    lb = ListBox(items=entries, name="menu")
    detail = DetailWidget(name="detail")
    form.add(lb)
    form.add(detail)

    term = Term()
    with term:
        idle_ms = 250
        last_paint = 0
        search_mode = False
        query = ""
        while True:
            cols, rows = _term_size()
            if term.dirty:
                form.title = (
                    f"❖ Leon Boot Manager — Search: {query or '_'}"
                    if search_mode
                    else "❖ Leon Boot Manager"
                )
                # ensure detail follows selection
                if lb.visible:
                    idx = lb.visible[lb.sel] if lb.sel < len(lb.visible) else None
                    entry = lb.items[idx] if idx is not None and idx < len(lb.items) else None
                else:
                    entry = None
                detail.set_entry(entry)
                paint(form.render(cols, rows), cols, rows)
                term.dirty = False
                last_paint = time.monotonic()
            key = read_key(term, 0.25)
            if key is None:
                continue
            # handle quit
            if key.name in ("q", "ctrl+c", "ctrl+q"):
                break
            if search_mode:
                if key.name == "esc":
                    search_mode = False
                    query = ""
                    lb.filter("")
                elif key.name in ("backspace", "delete"):
                    query = query[:-1]
                    lb.filter(query)
                elif key.name in ("enter",):
                    search_mode = False
                elif key.name == "char":
                    query += key.char
                    lb.filter(query)
                term.dirty = True
                continue
            # global keys
            if key.name == "char" and key.char == "/":
                search_mode = True
                query = ""
                lb.filter("")
                term.dirty = True
                continue
            if key.name in ("r", "R"):
                ctx = load_context()
                entries = _sort_entries(ctx.get("entries", []))
                lb.set_items(entries)
                term.dirty = True
                continue
            # delegate to listbox for navigation
            lb.handle_key(key)
            term.dirty = True
    return True


if __name__ == "__main__":
    run()
