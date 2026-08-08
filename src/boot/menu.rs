//! Native splash menu.
//!
//! When `splash = true` in `\EFI\leon\boot.toml`, lbl draws a boxed, colored
//! menu on the UEFI text console before handing off: a title bar, every boot
//! entry discovered on the ESP with the selection highlighted, and a status
//! bar with a live countdown (and a shrinking progress bar). Arrow keys move
//! the selection (`Home`/`End`/`PgUp`/`PgDn` jump), Enter boots it, and the
//! countdown auto-boots `default_entry`.
//!
//! The layout is queried from the current text mode every boot, so the frame
//! is sized and centered for whatever console the firmware provides. All
//! glyphs come from the standard UEFI font (CP437-derived), and the whole
//! draw path tolerates firmware that cannot render a glyph.
//!
//! Every entry the menu shows — Leon's own kernel included — is a real UEFI
//! application that gets chainloaded with `LoadImage`/`StartImage` (see
//! `image`). Nothing is special-cased.
//!
//! The menu is strictly opt-in. With `splash` unset or `false` — or a missing
//! `boot.toml` — nothing is drawn and the boot stays silent, preserving the
//! flicker-free guarantee. A `timeout` of `0` (or unset) shows the menu for the
//! default 5 seconds.

use core::time::Duration;

use alloc::vec::Vec;

use uefi::boot;
use uefi::cstr16;
use uefi::proto::console::text::{Color, Key, Output, ScanCode};
use uefi::system;
use uefi::{CStr16, CString16, Char16};

use leon_common::boot_config::BootConfig;

use crate::secure_boot::{self, SecureBootState};

use super::entries::{Entry, cstr_lossy, push_u32};

/// Default menu timeout in seconds when `boot.toml` leaves it unset.
const DEFAULT_TIMEOUT: u32 = 5;

/// Glyphs from the standard UEFI font (CP437-derived). Used to build the frame
/// and the status bar; firmware that cannot render one simply skips it.
const SPACE: Char16 = glyph(' ');
const HLINE: Char16 = glyph('─'); // U+2500
const VLINE: Char16 = glyph('│'); // U+2502
const TL: Char16 = glyph('┌'); // U+250C
const TR: Char16 = glyph('┐'); // U+2510
const BL: Char16 = glyph('└'); // U+2514
const BR: Char16 = glyph('┘'); // U+2518
const HDIV: Char16 = glyph('├'); // U+251C
const UDIV: Char16 = glyph('┤'); // U+2524
const MARK: Char16 = glyph('►'); // U+25BA
const SCROLL_UP: Char16 = glyph('▲'); // U+25B2
const SCROLL_DOWN: Char16 = glyph('▼'); // U+25BC
const FILL: Char16 = glyph('█'); // U+2588
const EMPTY: Char16 = glyph('░'); // U+2591

/// Width of the countdown progress bar, in cells.
const BAR_WIDTH: usize = 12;

/// Builds a `Char16` from a single BMP character of the fixed glyph set.
const fn glyph(c: char) -> Char16 {
    // SAFETY: every glyph above is a valid BMP code point, so the truncation
    // to u16 cannot produce an invalid UCS-2 scalar.
    unsafe { Char16::from_u16_unchecked(c as u32 as u16) }
}

/// Screen-relative geometry of the menu frame, derived from the current text
/// mode. The box is centered and sized so the title, entries and status bar
/// always fit, with room for scrolling when there are more entries than rows.
struct Layout {
    cols: usize,
    rows: usize,
    /// Column of the left border.
    frame_left: usize,
    /// Width including both borders.
    frame_w: usize,
    /// Usable width between the borders.
    content_w: usize,
    /// Row of the top border.
    top: usize,
    /// Row of the separator under the title.
    header_row: usize,
    /// Row of the first visible entry.
    first_entry_row: usize,
    /// Number of entry rows that fit on screen.
    entry_rows: usize,
    /// Row of the status line.
    status_row: usize,
    /// Row of the optional Secure Boot warning, between the status line and the
    /// bottom border.
    warn_row: Option<usize>,
    /// Row of the bottom border.
    bottom: usize,
}

impl Layout {
    fn new(cols: usize, rows: usize, entries: usize, warn: bool) -> Self {
        let cols = cols.max(8);
        let rows = rows.max(6);
        let margin = if cols >= 24 { 2 } else { 0 };
        let frame_w = (cols - 2 * margin).max(8);
        let content_w = frame_w - 2;
        let room = rows.saturating_sub(if warn { 7 } else { 6 });
        let entry_rows = entries.min(room.max(1));
        let top = rows.saturating_sub(entry_rows + if warn { 7 } else { 6 }) / 2;
        let warn_row = warn.then_some(top + 3 + entry_rows + 2);
        Self {
            cols,
            rows,
            frame_left: margin,
            frame_w,
            content_w,
            top,
            header_row: top + 2,
            first_entry_row: top + 3,
            entry_rows,
            status_row: top + 3 + entry_rows + 1,
            warn_row,
            bottom: top + 3 + entry_rows + if warn { 3 } else { 2 },
        }
    }
}

/// Shows the menu (blocking) and returns the index of the entry to boot:
/// the user's choice, or `default_entry` when the countdown elapses.
///
/// `secure_boot` is the platform's reported Secure Boot state; when it is on,
/// a warning row is drawn above the bottom border.
pub fn run(cfg: &BootConfig, entries: &[Entry], secure_boot: SecureBootState) -> usize {
    if entries.is_empty() {
        return 0;
    }

    let warn = secure_boot::warning(secure_boot);
    let timeout = cfg.timeout.unwrap_or(DEFAULT_TIMEOUT);
    let default_idx = cfg
        .default_entry
        .as_deref()
        .and_then(|want| {
            entries
                .iter()
                .position(|e| cstr_lossy(e.label.as_ref()).eq(want))
        })
        .unwrap_or(0)
        .min(entries.len() - 1);

    system::with_stdin(|input| {
        system::with_stdout(|out| {
            let _ = out.set_color(Color::LightGray, Color::Black);
            let _ = out.clear();
            let _ = out.enable_cursor(false);

            let (cols, rows) = out
                .current_mode()
                .ok()
                .flatten()
                .map(|m| (m.columns(), m.rows()))
                .unwrap_or((80, 25));
            let layout = Layout::new(cols, rows, entries.len(), warn.is_some());
            let deadline_ms = timeout.saturating_mul(1000);
            let mut selected = default_idx;
            let mut prev_selected = selected;
            let mut scroll_top = 0usize;
            let mut elapsed_ms = 0u32;
            let mut drawn_second = u32::MAX;
            let mut dirty_full = true;
            let mut dirty_rows = false;
            loop {
                if dirty_full {
                    let remaining = remaining_seconds(elapsed_ms, deadline_ms);
                    let frame = Frame {
                        entries,
                        selected,
                        scroll_top,
                        remaining,
                        timeout,
                        warn,
                    };
                    draw_all(out, &layout, &frame);
                    drawn_second = remaining;
                    prev_selected = selected;
                    dirty_full = false;
                } else if dirty_rows {
                    draw_entry_at(out, &layout, entries, prev_selected, scroll_top, selected);
                    draw_entry_at(out, &layout, entries, selected, scroll_top, selected);
                    prev_selected = selected;
                    dirty_rows = false;
                }
                let remaining = remaining_seconds(elapsed_ms, deadline_ms);
                if remaining != drawn_second {
                    drawn_second = remaining;
                    draw_status(out, &layout, remaining, timeout);
                }
                match input.read_key() {
                    Ok(Some(Key::Special(ScanCode::UP))) if selected > 0 => {
                        prev_selected = selected;
                        selected -= 1;
                        if selected < scroll_top {
                            scroll_top = selected;
                            dirty_full = true;
                        } else {
                            dirty_rows = true;
                        }
                    }
                    Ok(Some(Key::Special(ScanCode::DOWN))) if selected + 1 < entries.len() => {
                        prev_selected = selected;
                        selected += 1;
                        if selected >= scroll_top + layout.entry_rows {
                            scroll_top = selected + 1 - layout.entry_rows;
                            dirty_full = true;
                        } else {
                            dirty_rows = true;
                        }
                    }
                    Ok(Some(Key::Special(ScanCode::HOME))) if selected > 0 => {
                        prev_selected = selected;
                        selected = 0;
                        scroll_top = 0;
                        dirty_full = true;
                    }
                    Ok(Some(Key::Special(ScanCode::END))) if selected + 1 < entries.len() => {
                        prev_selected = selected;
                        selected = entries.len() - 1;
                        if selected >= scroll_top + layout.entry_rows {
                            scroll_top = selected + 1 - layout.entry_rows;
                        }
                        dirty_full = true;
                    }
                    Ok(Some(Key::Special(ScanCode::PAGE_UP))) if selected > 0 => {
                        prev_selected = selected;
                        selected = selected.saturating_sub(layout.entry_rows);
                        if selected < scroll_top {
                            scroll_top = selected;
                        }
                        dirty_full = true;
                    }
                    Ok(Some(Key::Special(ScanCode::PAGE_DOWN))) if selected + 1 < entries.len() => {
                        prev_selected = selected;
                        selected = (selected + layout.entry_rows).min(entries.len() - 1);
                        if selected >= scroll_top + layout.entry_rows {
                            scroll_top = selected + 1 - layout.entry_rows;
                        }
                        dirty_full = true;
                    }
                    Ok(Some(Key::Printable(c))) if is_enter(c) => {
                        let _ = out.set_color(Color::LightGray, Color::Black);
                        let _ = out.enable_cursor(true);
                        return selected;
                    }
                    _ => {}
                }
                if elapsed_ms >= deadline_ms {
                    let _ = out.set_color(Color::LightGray, Color::Black);
                    let _ = out.enable_cursor(true);
                    return default_idx;
                }
                boot::stall(Duration::from_millis(100));
                elapsed_ms = elapsed_ms.saturating_add(100);
            }
        })
    })
}

/// Immutable per-frame state shared by the render functions.
struct Frame<'a> {
    entries: &'a [Entry],
    selected: usize,
    scroll_top: usize,
    remaining: u32,
    timeout: u32,
    warn: Option<&'a str>,
}

/// Renders the whole menu frame: borders, title, entries, status, warning and
/// scroll markers. Call once per repaint; incremental updates repaint only the
/// rows that changed.
fn draw_all(out: &mut Output, l: &Layout, f: &Frame) {
    hline(out, l, l.top, TL, HLINE, TR);
    draw_title(out, l);
    hline(out, l, l.header_row, HDIV, HLINE, UDIV);
    let mut i = 0;
    while i < l.entry_rows {
        let idx = f.scroll_top + i;
        if idx < f.entries.len() {
            draw_entry(
                out,
                l,
                &f.entries[idx],
                l.first_entry_row + i,
                idx == f.selected,
            );
        } else {
            clear_row(out, l, l.first_entry_row + i);
        }
        i += 1;
    }
    hline(out, l, l.status_row - 1, HDIV, HLINE, UDIV);
    draw_status(out, l, f.remaining, f.timeout);
    if let (Some(row), Some(warn)) = (l.warn_row, f.warn) {
        draw_warning(out, l, row, warn);
    }
    hline(out, l, l.bottom, BL, HLINE, BR);
    if f.scroll_top > 0 {
        write_glyph(out, l, l.frame_left + l.content_w, l.header_row, SCROLL_UP);
    }
    if f.scroll_top + l.entry_rows < f.entries.len() {
        write_glyph(
            out,
            l,
            l.frame_left + l.content_w,
            l.status_row - 1,
            SCROLL_DOWN,
        );
    }
    let _ = out.set_color(Color::LightGray, Color::Black);
}

/// Repaints a single entry row, when `idx` is currently visible.
fn draw_entry_at(
    out: &mut Output,
    l: &Layout,
    entries: &[Entry],
    idx: usize,
    scroll_top: usize,
    selected: usize,
) {
    if idx < scroll_top || idx >= scroll_top + l.entry_rows {
        return;
    }
    draw_entry(
        out,
        l,
        &entries[idx],
        l.first_entry_row + idx - scroll_top,
        idx == selected,
    );
}

/// The title bar: white on blue, centered in the frame.
fn draw_title(out: &mut Output, l: &Layout) {
    let _ = out.set_color(Color::White, Color::Blue);
    let mut buf = CString16::new();
    buf.push(VLINE);
    let title = cstr16!(" Leon Boot Manager ");
    let pad = l.content_w.saturating_sub(title.num_chars());
    for _ in 0..(pad / 2) {
        buf.push(SPACE);
    }
    buf.push_str(title);
    for _ in 0..(pad - pad / 2) {
        buf.push(SPACE);
    }
    buf.push(VLINE);
    write_at(out, l, l.frame_left, l.top + 1, buf.as_ref());
    let _ = out.set_color(Color::LightGray, Color::Black);
}

/// One entry row. The selected entry is reversed (black on light gray) and
/// marked with `►`; long labels are truncated with `...` so the highlight
/// always spans the full row.
fn draw_entry(out: &mut Output, l: &Layout, entry: &Entry, row: usize, selected: bool) {
    if row >= l.rows {
        return;
    }
    let (fg, bg) = if selected {
        (Color::Black, Color::LightGray)
    } else {
        (Color::LightGray, Color::Black)
    };
    let _ = out.set_color(fg, bg);
    let mut buf = CString16::new();
    buf.push(VLINE);
    buf.push(SPACE);
    buf.push(if selected { MARK } else { SPACE });
    buf.push(SPACE);
    let label = entry.label.as_ref().to_u16_slice();
    let max = l.content_w.saturating_sub(3);
    let mut added;
    if label.len() > max {
        for &u in &label[..max.saturating_sub(3)] {
            buf.push(Char16::try_from(u).unwrap_or(SPACE));
        }
        buf.push_str(cstr16!("..."));
        added = max;
    } else {
        for &u in label {
            buf.push(Char16::try_from(u).unwrap_or(SPACE));
        }
        added = label.len();
    }
    while added < max {
        buf.push(SPACE);
        added += 1;
    }
    buf.push(VLINE);
    write_at(out, l, l.frame_left, row, buf.as_ref());
    let _ = out.set_color(Color::LightGray, Color::Black);
}

/// Erases an entry row (used when the list is shorter than the frame).
fn clear_row(out: &mut Output, l: &Layout, row: usize) {
    if row >= l.rows {
        return;
    }
    let _ = out.set_color(Color::LightGray, Color::Black);
    let mut buf = CString16::new();
    buf.push(VLINE);
    for _ in 0..l.content_w {
        buf.push(SPACE);
    }
    buf.push(VLINE);
    write_at(out, l, l.frame_left, row, buf.as_ref());
}

/// The status bar: navigation help on the left, the countdown plus a
/// shrinking progress bar on the right.
fn draw_status(out: &mut Output, l: &Layout, remaining: u32, timeout: u32) {
    let _ = out.set_color(Color::LightGray, Color::Black);
    let help = cstr16!("↑/↓ move   Enter boot");
    let mut right = CString16::new();
    right.push_str(cstr16!("boot in "));
    push_u32(&mut right, remaining);
    right.push_str(cstr16!("s "));
    let filled = if timeout == 0 {
        0
    } else {
        (u64::from(remaining) * BAR_WIDTH as u64 / u64::from(timeout)) as usize
    };
    let pad = l
        .content_w
        .saturating_sub(help.num_chars() + right.num_chars() + BAR_WIDTH);

    let mut seg = CString16::new();
    seg.push(VLINE);
    seg.push(SPACE);
    seg.push_str(help);
    for _ in 0..pad {
        seg.push(SPACE);
    }
    write_at(out, l, l.frame_left, l.status_row, seg.as_ref());

    let right_col = l.frame_left + 1 + help.num_chars() + pad;
    let _ = out.set_color(Color::Yellow, Color::Black);
    write_at(out, l, right_col, l.status_row, right.as_ref());
    let mut bar = CString16::new();
    for _ in 0..filled.min(BAR_WIDTH) {
        bar.push(FILL);
    }
    write_at(
        out,
        l,
        right_col + right.num_chars(),
        l.status_row,
        bar.as_ref(),
    );
    let _ = out.set_color(Color::DarkGray, Color::Black);
    let mut rest = CString16::new();
    for _ in filled.min(BAR_WIDTH)..BAR_WIDTH {
        rest.push(EMPTY);
    }
    write_at(
        out,
        l,
        right_col + right.num_chars() + filled.min(BAR_WIDTH),
        l.status_row,
        rest.as_ref(),
    );
    let _ = out.set_color(Color::LightGray, Color::Black);
    write_glyph(out, l, l.frame_left + l.frame_w - 1, l.status_row, VLINE);
}

/// The Secure Boot warning row: yellow on black, between the status line and
/// the bottom border. Text is truncated with `...` when it would overflow.
fn draw_warning(out: &mut Output, l: &Layout, row: usize, warn: &str) {
    let _ = out.set_color(Color::Yellow, Color::Black);
    let mut buf = CString16::new();
    buf.push(VLINE);
    buf.push(SPACE);
    let max = l.content_w.saturating_sub(3);
    let chars: Vec<char> = warn.chars().collect();
    let (body, suffix) = if chars.len() > max {
        (&chars[..max.saturating_sub(3)], &['.', '.', '.'][..])
    } else {
        (chars.as_slice(), &[][..])
    };
    let mut used = 0;
    for &c in body.iter().chain(suffix.iter()) {
        buf.push(Char16::try_from(c).unwrap_or(SPACE));
        used += 1;
    }
    while used < max {
        buf.push(SPACE);
        used += 1;
    }
    buf.push(VLINE);
    write_at(out, l, l.frame_left, row, buf.as_ref());
    let _ = out.set_color(Color::LightGray, Color::Black);
}

/// Draws a horizontal border line between the two end caps.
fn hline(out: &mut Output, l: &Layout, row: usize, left: Char16, mid: Char16, right: Char16) {
    let mut buf = CString16::new();
    buf.push(left);
    for _ in 0..l.content_w {
        buf.push(mid);
    }
    buf.push(right);
    write_at(out, l, l.frame_left, row, buf.as_ref());
}

/// Writes a single glyph, used for the scroll markers and the status border.
fn write_glyph(out: &mut Output, l: &Layout, col: usize, row: usize, c: Char16) {
    let mut buf = CString16::new();
    buf.push(c);
    write_at(out, l, col, row, buf.as_ref());
}

/// Writes text at a cell, tolerating glyphs the console cannot render and
/// silently skipping anything off-screen.
fn write_at(out: &mut Output, l: &Layout, col: usize, row: usize, text: &CStr16) {
    if col >= l.cols || row >= l.rows {
        return;
    }
    let _ = out.set_cursor_position(col, row);
    let _ = out.output_string_lossy(text);
}

/// Seconds until the deadline, rounded up (the menu shows `timeout`, then 0).
fn remaining_seconds(elapsed_ms: u32, deadline_ms: u32) -> u32 {
    deadline_ms.saturating_sub(elapsed_ms).div_ceil(1000)
}

/// Whether a printable key means "boot the selection" (Enter, CR or LF).
fn is_enter(c: Char16) -> bool {
    let code: u16 = c.into();
    code == 0x0D || code == 0x0A
}
