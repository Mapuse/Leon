//! Menuconfig-style boot menu — the on-device port of `lbm`.
//!
//! When `splash = true` in `\EFI\leon\boot.toml`, lbl draws a kernel
//! menuconfig-style menu on the UEFI text console: a boxed list of `[Label]
//! (value)` rows, navigated with the arrow keys, that edits the very keys
//! `lbm` edits — `timeout`, `splash`, `theme`, `entries_file`,
//! `default_entry` — and writes them straight back to `boot.toml` on the ESP.
//!
//! The layout mirrors `lbm/src/main.rs` row for row: `[Boot Timeout
//! (seconds)]`, `[Show Boot Menu (splash)]` as a `[*]` toggle, `[Splash
//! Theme]`, `[Entries File]`, `[Default Boot Entry]`, `[Discovered Boot
//! Entries]`, and `[Secure Boot Status]`. Enter opens the value editor for a
//! row (the string/number editors take typed text like `lbm`'s dialogs), the
//! toggle row flips, the two `--->` rows open their sub-screens, and the first
//! row — `[Boot Now]` — boots. A live countdown on the status bar auto-boots
//! `default_entry`; `Esc` pauses it (and backs out of sub-screens).
//!
//! The menu is strictly opt-in and shares the flicker-free rules of the rest
//! of the loader: it draws only to the text console, never touches the GOP
//! frame buffer, and with `splash` unset or `false` nothing is drawn at all.
//! A `timeout` of `0` (or unset) shows the menu for the default 5 seconds.
//!
//! Config edits are serialized with
//! `leon_common::boot_config::serialize_boot_config` — the exact inverse of
//! the parser the bootloader runs on every boot — so whatever the menu writes
//! is guaranteed to parse again next boot. Writing is best-effort: a
//! read-only volume never derails the menu, and the in-memory config still
//! drives this boot.

use core::time::Duration;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use uefi::boot;
use uefi::cstr16;
use uefi::fs::FileSystem;
use uefi::proto::console::text::{Color, Input, Key, Output, ScanCode};
use uefi::system;
use uefi::{Char16, CStr16, CString16};

use leon_common::boot_config::BootConfig;

use crate::secure_boot::{self, SecureBootState};

use super::config;
use super::entries::{cstr_lossy, Entry};
use super::serial::{Console, MenuKey};

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

/// Width of the countdown progress bar, in cells.
const BAR_WIDTH: usize = 12;

/// Builds a `Char16` from a single BMP character of the fixed glyph set.
const fn glyph(c: char) -> Char16 {
    // SAFETY: every glyph above is a valid BMP code point, so the truncation
    // to u16 cannot produce an invalid UCS-2 scalar.
    unsafe { Char16::from_u16_unchecked(c as u32 as u16) }
}

// ─────────────────────────────────────────────────────────────────────────
// Menu structure (mirrors `lbm/src/main.rs`)
// ─────────────────────────────────────────────────────────────────────────

/// The selectable rows of the main menu, in display order. `MainAction` is the
/// copy of `lbm`'s `MenuAction`, plus the boot rows the loader needs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MainAction {
    BootNow,
    ToggleSplash,
    EditTimeout,
    EditTheme,
    EditEntriesFile,
    EditDefaultEntry,
    ViewEntries,
    ViewSecureBoot,
}

/// Which top-level screen is showing. The value editors are modal sub-loops
/// ([`edit_text`]) rather than screens.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Entries(EntriesMode),
    SecureBoot,
}

/// What Enter does on the entries sub-screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EntriesMode {
    /// Enter boots the highlighted entry (`[Discovered Boot Entries]`).
    Boot,
    /// Enter makes the highlighted entry the default (`[Default Boot Entry]`).
    SetDefault,
}

/// `lbm`'s `pad_label`: a 34-cell label column, then the value.
fn pad(label: &str, value: &str) -> String {
    format!("{label:<34}{value}")
}

/// The main menu rows, exactly the set `lbm` shows, with the current values
/// formatted the same way (`(5)`, `[*]`, `--->`, ...).
fn main_rows(cfg: &BootConfig) -> Vec<(MainAction, String)> {
    vec![
        (
            MainAction::BootNow,
            pad("[Boot Now]", "--->"),
        ),
        (
            MainAction::EditTimeout,
            pad(
                "[Boot Timeout (seconds)]",
                &format!("({})", cfg.timeout.unwrap_or(DEFAULT_TIMEOUT)),
            ),
        ),
        (
            MainAction::ToggleSplash,
            pad(
                "[Show Boot Menu (splash)]",
                if cfg.splash.unwrap_or(true) { "[*]" } else { "[ ]" },
            ),
        ),
        (
            MainAction::EditTheme,
            pad(
                "[Splash Theme]",
                &format!("({})", cfg.theme.as_deref().unwrap_or("default")),
            ),
        ),
        (
            MainAction::EditEntriesFile,
            pad(
                "[Entries File]",
                cfg.entries_file.as_deref().unwrap_or(r"\EFI\leon\entries.jsonc"),
            ),
        ),
        (
            MainAction::EditDefaultEntry,
            pad(
                "[Default Boot Entry]",
                &format!("({})", cfg.default_entry.as_deref().unwrap_or("auto")),
            ),
        ),
        (
            MainAction::ViewEntries,
            pad("[Discovered Boot Entries]", "--->"),
        ),
        (
            MainAction::ViewSecureBoot,
            pad("[Secure Boot Status]", "--->"),
        ),
    ]
}

/// The index of the entry `default_entry` names, else the first entry.
pub fn default_index(cfg: &BootConfig, entries: &[Entry]) -> usize {
    cfg.default_entry
        .as_deref()
        .and_then(|want| {
            entries
                .iter()
                .position(|e| cstr_lossy(e.label.as_ref()).eq(want))
        })
        .unwrap_or(0)
        .min(entries.len() - 1)
}

// ─────────────────────────────────────────────────────────────────────────
// Geometry
// ─────────────────────────────────────────────────────────────────────────

/// Screen-relative geometry of the menu frame, derived from the current text
/// mode. The box is centered and sized so the title, rows and status bar
/// always fit, with room for scrolling when there are more rows than screen
/// cells.
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
    /// Row of the first visible row.
    first_entry_row: usize,
    /// Number of rows that fit on screen.
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

/// Keeps the scroll window pinned to the selection: the selected row never
/// leaves the visible window.
fn clamp_scroll(selected: usize, scroll_top: usize, visible: usize) -> usize {
    if selected < scroll_top {
        selected
    } else if selected >= scroll_top + visible {
        selected + 1 - visible
    } else {
        scroll_top
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Rendering
// ─────────────────────────────────────────────────────────────────────────

/// Shows the menu (blocking) and returns the index of the entry to boot: the
/// user's choice, `[Boot Now]`, or `default_entry` when the countdown elapses.
///
/// `fs` is the boot-volume filesystem (used to persist config edits to
/// `boot.toml`; `None` simply means edits are not saved). `cfg` is mutated by
/// the menu and saved back on every committed edit, so the values shown match
/// what the next boot reads.
pub fn run(
    mut fs: Option<&mut FileSystem>,
    cfg: &mut BootConfig,
    entries: &[Entry],
    secure_boot: SecureBootState,
) -> usize {
    if entries.is_empty() {
        return 0;
    }

    let warn = secure_boot::warning(secure_boot);
    let timeout = cfg.timeout.unwrap_or(DEFAULT_TIMEOUT);

    system::with_stdin(|input| {
        system::with_stdout(|out| {
            let _ = out.set_color(Color::White, Color::Black);
            let _ = out.clear();
            let _ = out.enable_cursor(false);

            let (cols, rows) = out
                .current_mode()
                .ok()
                .flatten()
                .map(|m| (m.columns(), m.rows()))
                .unwrap_or((80, 25));
            let row_count = main_rows(cfg).len();
            let layout = Layout::new(cols, rows, row_count.max(entries.len()), warn.is_some());
            let deadline_ms = timeout.saturating_mul(1000);

            let mut screen = Screen::Main;
            let mut selected = 0usize;
            let mut scroll_top = 0usize;
            let mut elapsed_ms = 0u32;
            let mut drawn_second = u32::MAX;
            let mut disarmed = false;
            let mut serial = Console::open();
            let mut boot_idx = None;
            let mut dirty = true;

            loop {
                let remaining = remaining_seconds(elapsed_ms, deadline_ms);
                if dirty || remaining != drawn_second {
                    let view = View {
                        cfg: &*cfg,
                        entries,
                        selected,
                        scroll_top,
                        remaining,
                        timeout,
                        disarmed,
                        warn,
                    };
                    draw_screen(out, &layout, screen, &view);
                    drawn_second = remaining;
                    dirty = false;
                }

                let mut keys = Vec::new();
                while let Ok(Some(key)) = input.read_key() {
                    if let Some(mk) = key_from_conin(key) {
                        keys.push(mk);
                    }
                }
                if let Some(console) = serial.as_mut() {
                    keys.extend(console.poll());
                }

                for key in keys {
                    match screen {
                        Screen::Main => match key {
                            MenuKey::Up | MenuKey::Down => {
                                let rows = main_rows(cfg).len();
                                let next = match key {
                                    MenuKey::Up => selected.saturating_sub(1),
                                    _ => (selected + 1).min(rows - 1),
                                };
                                if next != selected {
                                    selected = next;
                                    scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                    dirty = true;
                                }
                            }
                            MenuKey::Home => {
                                selected = 0;
                                scroll_top = 0;
                                dirty = true;
                            }
                            MenuKey::End => {
                                selected = main_rows(cfg).len() - 1;
                                scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                dirty = true;
                            }
                            MenuKey::PageUp => {
                                selected = selected.saturating_sub(layout.entry_rows);
                                scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                dirty = true;
                            }
                            MenuKey::PageDown => {
                                selected =
                                    (selected + layout.entry_rows).min(main_rows(cfg).len() - 1);
                                scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                dirty = true;
                            }
                            MenuKey::Esc => {
                                if !disarmed {
                                    disarmed = true;
                                    dirty = true;
                                }
                            }
                            MenuKey::Enter => {
                                let action = main_rows(cfg)[selected].0;
                                match action {
                                    MainAction::BootNow => {
                                        boot_idx = Some(default_index(cfg, entries));
                                    }
                                    MainAction::ToggleSplash => {
                                        cfg.splash = Some(!cfg.splash.unwrap_or(true));
                                        save_config(&mut fs, cfg);
                                        dirty = true;
                                    }
                                    MainAction::EditTimeout => {
                                        disarmed = true;
                                        let current = cfg
                                            .timeout
                                            .map(|t| t.to_string())
                                            .unwrap_or_default();
                                        if let Some(value) = edit_text(
                                            input,
                                            out,
                                            &layout,
                                            &mut serial,
                                            &EditSpec {
                                                title: cstr16!(" Boot Timeout (seconds) "),
                                                initial: &current,
                                                validate: is_u32,
                                                hint: "Whole number of seconds, e.g. 5",
                                            },
                                        ) && let Ok(n) = value.parse::<u32>()
                                        {
                                            cfg.timeout = Some(n);
                                            save_config(&mut fs, cfg);
                                        }
                                        dirty = true;
                                        break;
                                    }
                                    MainAction::EditTheme => {
                                        disarmed = true;
                                        let current = cfg.theme.clone().unwrap_or_default();
                                        if let Some(value) = edit_text(
                                            input,
                                            out,
                                            &layout,
                                            &mut serial,
                                            &EditSpec {
                                                title: cstr16!(" Splash Theme "),
                                                initial: &current,
                                                validate: |_| true,
                                                hint: "Theme file name; blank resets to default",
                                            },
                                        ) {
                                            cfg.theme = opt_str(value);
                                            save_config(&mut fs, cfg);
                                        }
                                        dirty = true;
                                        break;
                                    }
                                    MainAction::EditEntriesFile => {
                                        disarmed = true;
                                        let current = cfg.entries_file.clone().unwrap_or_default();
                                        if let Some(value) = edit_text(
                                            input,
                                            out,
                                            &layout,
                                            &mut serial,
                                            &EditSpec {
                                                title: cstr16!(" Entries File "),
                                                initial: &current,
                                                validate: |_| true,
                                                hint: r"ESP path, e.g. \EFI\leon\entries.jsonc",
                                            },
                                        ) {
                                            cfg.entries_file = opt_str(value);
                                            save_config(&mut fs, cfg);
                                        }
                                        dirty = true;
                                        break;
                                    }
                                    MainAction::EditDefaultEntry => {
                                        screen = Screen::Entries(EntriesMode::SetDefault);
                                        selected = 0;
                                        scroll_top = 0;
                                        disarmed = true;
                                        dirty = true;
                                        break;
                                    }
                                    MainAction::ViewEntries => {
                                        screen = Screen::Entries(EntriesMode::Boot);
                                        selected = default_index(cfg, entries);
                                        scroll_top = clamp_scroll(selected, 0, layout.entry_rows);
                                        disarmed = true;
                                        dirty = true;
                                        break;
                                    }
                                    MainAction::ViewSecureBoot => {
                                        screen = Screen::SecureBoot;
                                        disarmed = true;
                                        dirty = true;
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        },
                        Screen::Entries(mode) => match key {
                            MenuKey::Up | MenuKey::Down => {
                                let count = entries_row_count(entries, mode);
                                let next = match key {
                                    MenuKey::Up => selected.saturating_sub(1),
                                    _ => (selected + 1).min(count - 1),
                                };
                                if next != selected {
                                    selected = next;
                                    scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                    dirty = true;
                                }
                            }
                            MenuKey::Home => {
                                selected = 0;
                                scroll_top = 0;
                                dirty = true;
                            }
                            MenuKey::End => {
                                selected = entries_row_count(entries, mode) - 1;
                                scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                dirty = true;
                            }
                            MenuKey::PageUp => {
                                selected = selected.saturating_sub(layout.entry_rows);
                                scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                dirty = true;
                            }
                            MenuKey::PageDown => {
                                selected = (selected + layout.entry_rows)
                                    .min(entries_row_count(entries, mode) - 1);
                                scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                dirty = true;
                            }
                            MenuKey::Printable(c) => {
                                if let Some(n) = digit_value(c).filter(|&n| n < entries.len()) {
                                    selected = n;
                                    scroll_top = clamp_scroll(selected, scroll_top, layout.entry_rows);
                                    dirty = true;
                                }
                            }
                            MenuKey::Enter => {
                                if selected < entries.len() {
                                    match mode {
                                        EntriesMode::Boot => boot_idx = Some(selected),
                                        EntriesMode::SetDefault => {
                                            cfg.default_entry = Some(cstr_lossy(
                                                entries[selected].label.as_ref(),
                                            ));
                                            save_config(&mut fs, cfg);
                                        }
                                    }
                                } else {
                                    // The trailing "Type a custom path..." row.
                                    disarmed = true;
                                    let current = cfg.default_entry.clone().unwrap_or_default();
                                    if let Some(value) = edit_text(
                                        input,
                                        out,
                                        &layout,
                                        &mut serial,
                                        &EditSpec {
                                            title: cstr16!(" Custom Default Entry Path "),
                                            initial: &current,
                                            validate: |_| true,
                                            hint: r"ESP path, e.g. \EFI\leon\kernel.efi",
                                        },
                                    ) {
                                        cfg.default_entry = opt_str(value);
                                        save_config(&mut fs, cfg);
                                    }
                                }
                                screen = Screen::Main;
                                selected = 0;
                                scroll_top = 0;
                                dirty = true;
                                break;
                            }
                            MenuKey::Esc => {
                                screen = Screen::Main;
                                selected = 0;
                                scroll_top = 0;
                                dirty = true;
                                break;
                            }
                            _ => {}
                        },
                        Screen::SecureBoot => {
                            if key == MenuKey::Esc {
                                screen = Screen::Main;
                                selected = 0;
                                scroll_top = 0;
                                dirty = true;
                                break;
                            }
                        }
                    }
                }

                if let Some(idx) = boot_idx {
                    let _ = out.set_color(Color::White, Color::Black);
                    let _ = out.enable_cursor(true);
                    return idx;
                }
                if matches!(screen, Screen::Main)
                    && !disarmed
                    && elapsed_ms >= deadline_ms
                {
                    let _ = out.set_color(Color::White, Color::Black);
                    let _ = out.enable_cursor(true);
                    return default_index(cfg, entries);
                }
                boot::stall(Duration::from_millis(100));
                if matches!(screen, Screen::Main) && !disarmed {
                    elapsed_ms = elapsed_ms.saturating_add(100);
                }
            }
        })
    })
}

/// Number of rows on the entries screen: one per entry, plus the trailing
/// "type a custom path" row when choosing the default.
fn entries_row_count(entries: &[Entry], mode: EntriesMode) -> usize {
    if mode == EntriesMode::SetDefault {
        entries.len() + 1
    } else {
        entries.len()
    }
}

/// The text of one entries-screen row.
fn entry_row_text(entries: &[Entry], idx: usize, mode: EntriesMode) -> String {
    if idx < entries.len() {
        cstr_lossy(entries[idx].label.as_ref())
    } else {
        let _ = mode;
        "Type a custom path...".to_string()
    }
}

/// The Secure Boot info screen body.
fn secure_boot_lines(warn: Option<&str>) -> Vec<String> {
    let mut lines = Vec::new();
    match warn {
        Some(w) => lines.push(w.to_string()),
        None => lines.push("Secure Boot is off — no image verification.".to_string()),
    }
    lines.push(String::new());
    lines.push("Unsigned entries are rejected by the firmware when".to_string());
    lines.push("Secure Boot is on (ACCESS_DENIED / SECURITY_VIOLATION).".to_string());
    lines.push(String::new());
    lines.push("To sign your staged tree:".to_string());
    lines.push("  scripts/sign.sh setup, then make sign".to_string());
    lines
}

/// Persists config edits to `boot.toml`. Best-effort: no filesystem, or a
/// read-only volume, just skips the write.
fn save_config(fs: &mut Option<&mut FileSystem>, cfg: &BootConfig) {
    if let Some(fs) = fs.as_mut() {
        config::write(fs, cfg);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Draw helpers
// ─────────────────────────────────────────────────────────────────────────

/// Everything a screen needs to draw, bundled so the draw path stays small.
struct View<'a> {
    cfg: &'a BootConfig,
    entries: &'a [Entry],
    selected: usize,
    scroll_top: usize,
    remaining: u32,
    timeout: u32,
    disarmed: bool,
    warn: Option<&'a str>,
}

fn draw_screen(out: &mut Output, l: &Layout, screen: Screen, v: &View<'_>) {
    match screen {
        Screen::Main => {
            let rows = main_rows(v.cfg);
            draw_frame(out, l, cstr16!(" Leon Boot Menuconfig "), v.warn);
            for i in 0..l.entry_rows {
                let idx = v.scroll_top + i;
                if idx < rows.len() {
                    draw_row(out, l, l.first_entry_row + i, &rows[idx].1, idx == v.selected);
                } else {
                    clear_row(out, l, l.first_entry_row + i);
                }
            }
            draw_scroll_markers(out, l, v.scroll_top, rows.len());
            draw_status(
                out,
                l,
                "↑/↓ move  Enter select  Esc pause",
                Some(&countdown_text(v.remaining, v.timeout, v.disarmed)),
            );
        }
        Screen::Entries(mode) => {
            let title = match mode {
                EntriesMode::Boot => cstr16!(" Discovered Boot Entries "),
                EntriesMode::SetDefault => cstr16!(" Default Boot Entry "),
            };
            let row_count = entries_row_count(v.entries, mode);
            draw_frame(out, l, title, v.warn);
            for i in 0..l.entry_rows {
                let idx = v.scroll_top + i;
                if idx < row_count {
                    let text = entry_row_text(v.entries, idx, mode);
                    draw_row(out, l, l.first_entry_row + i, &text, idx == v.selected);
                } else {
                    clear_row(out, l, l.first_entry_row + i);
                }
            }
            draw_scroll_markers(out, l, v.scroll_top, row_count);
            let help = match mode {
                EntriesMode::Boot => "↑/↓ move  Enter boot  Esc back",
                EntriesMode::SetDefault => "↑/↓ move  Enter set default  Esc back",
            };
            draw_status(out, l, help, None);
        }
        Screen::SecureBoot => {
            draw_frame(out, l, cstr16!(" Secure Boot Status "), v.warn);
            let lines = secure_boot_lines(v.warn);
            for i in 0..l.entry_rows {
                if i < lines.len() {
                    draw_row(out, l, l.first_entry_row + i, &lines[i], false);
                } else {
                    clear_row(out, l, l.first_entry_row + i);
                }
            }
            draw_status(out, l, "Esc: back", None);
        }
    }
}

/// Borders, title bar and Secure Boot warning shared by every screen.
fn draw_frame(out: &mut Output, l: &Layout, title: &CStr16, warn: Option<&str>) {
    hline(out, l, l.top, TL, HLINE, TR);
    draw_title(out, l, title);
    hline(out, l, l.header_row, HDIV, HLINE, UDIV);
    hline(out, l, l.status_row - 1, HDIV, HLINE, UDIV);
    if let (Some(row), Some(warn)) = (l.warn_row, warn) {
        draw_warning(out, l, row, warn);
    }
    hline(out, l, l.bottom, BL, HLINE, BR);
}

/// The title bar, centered in the frame.
fn draw_title(out: &mut Output, l: &Layout, title: &CStr16) {
    let _ = out.set_color(Color::White, Color::Black);
    let mut buf = CString16::new();
    buf.push(VLINE);
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
    let _ = out.set_color(Color::White, Color::Black);
}

/// One menu row. The selected row is marked with `►` — the highlight the
/// firmware console supports without attribute games; long rows are
/// truncated with `...` so the row always spans the full frame width.
fn draw_row(out: &mut Output, l: &Layout, row: usize, text: &str, selected: bool) {
    if row >= l.rows {
        return;
    }
    let _ = out.set_color(Color::White, Color::Black);
    let mut buf = CString16::new();
    buf.push(VLINE);
    buf.push(SPACE);
    buf.push(if selected { MARK } else { SPACE });
    buf.push(SPACE);
    let max = l.content_w.saturating_sub(3);
    let chars: Vec<char> = text.chars().collect();
    let mut used = 0;
    if chars.len() > max {
        for &c in &chars[..max.saturating_sub(3)] {
            buf.push(Char16::try_from(c).unwrap_or(SPACE));
            used += 1;
        }
        for c in "...".chars() {
            buf.push(Char16::try_from(c).unwrap_or(SPACE));
            used += 1;
        }
    } else {
        for &c in &chars {
            buf.push(Char16::try_from(c).unwrap_or(SPACE));
            used += 1;
        }
    }
    while used < max {
        buf.push(SPACE);
        used += 1;
    }
    buf.push(VLINE);
    write_at(out, l, l.frame_left, row, buf.as_ref());
    let _ = out.set_color(Color::White, Color::Black);
}

/// Erases a row (used when a screen has fewer rows than the frame).
fn clear_row(out: &mut Output, l: &Layout, row: usize) {
    if row >= l.rows {
        return;
    }
    let _ = out.set_color(Color::White, Color::Black);
    let mut buf = CString16::new();
    buf.push(VLINE);
    for _ in 0..l.content_w {
        buf.push(SPACE);
    }
    buf.push(VLINE);
    write_at(out, l, l.frame_left, row, buf.as_ref());
}

/// The status bar: help text on the left, optional countdown on the right.
fn draw_status(out: &mut Output, l: &Layout, left: &str, right: Option<&str>) {
    let _ = out.set_color(Color::White, Color::Black);
    let left_c = to_cstring(left);
    let pad = l.content_w.saturating_sub(left_c.num_chars());
    let mut seg = CString16::new();
    seg.push(VLINE);
    seg.push(SPACE);
    seg.push_str(left_c.as_ref());
    for _ in 0..pad {
        seg.push(SPACE);
    }
    write_at(out, l, l.frame_left, l.status_row, seg.as_ref());
    if let Some(r) = right {
        let right_c = to_cstring(r);
        let right_col = (l.frame_left + 1 + l.content_w).saturating_sub(right_c.num_chars());
        write_at(out, l, right_col, l.status_row, right_c.as_ref());
    }
    write_glyph(out, l, l.frame_left + l.frame_w - 1, l.status_row, VLINE);
    let _ = out.set_color(Color::White, Color::Black);
}

/// The countdown: `boot in Ns` plus a shrinking progress bar, or `paused`
/// once `Esc` disarmed it.
fn countdown_text(remaining: u32, timeout: u32, paused: bool) -> String {
    let mut right = String::new();
    if paused {
        right.push_str("paused");
    } else {
        right.push_str("boot in ");
        right.push_str(&remaining.to_string());
        right.push_str("s ");
        let filled = if timeout == 0 {
            0
        } else {
            (u64::from(remaining) * BAR_WIDTH as u64 / u64::from(timeout)) as usize
        };
        for _ in 0..filled.min(BAR_WIDTH) {
            right.push('█');
        }
        for _ in filled.min(BAR_WIDTH)..BAR_WIDTH {
            right.push('░');
        }
    }
    right
}

/// The Secure Boot warning row, between the status line and the bottom border.
fn draw_warning(out: &mut Output, l: &Layout, row: usize, warn: &str) {
    let _ = out.set_color(Color::White, Color::Black);
    let mut buf = CString16::new();
    buf.push(VLINE);
    buf.push(SPACE);
    let max = l.content_w.saturating_sub(2);
    let chars: Vec<char> = warn.chars().collect();
    let mut used = 0;
    for &c in chars.iter().take(max) {
        buf.push(Char16::try_from(c).unwrap_or(SPACE));
        used += 1;
    }
    while used < max {
        buf.push(SPACE);
        used += 1;
    }
    buf.push(VLINE);
    write_at(out, l, l.frame_left, row, buf.as_ref());
    let _ = out.set_color(Color::White, Color::Black);
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

/// Scroll-up/down markers at the right edge of the row area.
fn draw_scroll_markers(out: &mut Output, l: &Layout, scroll_top: usize, len: usize) {
    if scroll_top > 0 {
        write_glyph(out, l, l.frame_left + l.content_w, l.header_row, SCROLL_UP);
    }
    if scroll_top + l.entry_rows < len {
        write_glyph(
            out,
            l,
            l.frame_left + l.content_w,
            l.status_row - 1,
            SCROLL_DOWN,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Value editors (the `lbm` dialogs, on the text console)
// ─────────────────────────────────────────────────────────────────────────

/// Parameters of one value editor, mirroring `lbm`'s `EditView` dialogs.
struct EditSpec<'a> {
    title: &'a CStr16,
    initial: &'a str,
    validate: fn(&str) -> bool,
    hint: &'a str,
}

/// Modal text editor: type a value, Enter confirms, Esc cancels. Returns the
/// confirmed value when valid, or `None` on cancel. While invalid, the error
/// row is shown and Enter is ignored.
fn edit_text(
    input: &mut Input,
    out: &mut Output,
    l: &Layout,
    serial: &mut Option<Console>,
    spec: &EditSpec<'_>,
) -> Option<String> {
    let mut buf: Vec<char> = spec.initial.chars().collect();
    let max_chars = l.content_w.saturating_sub(4);
    let mut error: Option<&str> = None;
    loop {
        draw_edit(out, l, spec.title, &buf, spec.hint, error);
        let mut keys = Vec::new();
        while let Ok(Some(key)) = input.read_key() {
            if let Some(mk) = key_from_conin(key) {
                keys.push(mk);
            }
        }
        if let Some(console) = serial.as_mut() {
            keys.extend(console.poll());
        }
        let mut done = None;
        for key in keys {
            match key {
                MenuKey::Esc => {
                    done = Some(None);
                    break;
                }
                MenuKey::Enter => {
                    let s: String = buf.iter().collect();
                    if (spec.validate)(&s) {
                        done = Some(Some(s));
                        break;
                    }
                    error = Some("Invalid value — Esc to cancel");
                }
                MenuKey::Backspace => {
                    buf.pop();
                    error = None;
                }
                MenuKey::Printable(c) if buf.len() < max_chars => {
                    buf.push(c);
                    error = None;
                }
                _ => {}
            }
        }
        if let Some(d) = done {
            return d;
        }
        boot::stall(Duration::from_millis(50));
    }
}

/// Draws the editor frame: title, the value with a cursor block, a hint row,
/// an optional error row, and the key help status line.
fn draw_edit(
    out: &mut Output,
    l: &Layout,
    title: &CStr16,
    buf: &[char],
    hint: &str,
    error: Option<&str>,
) {
    draw_frame(out, l, title, None);
    let mut line = String::new();
    for &c in buf {
        line.push(c);
    }
    line.push('_');
    draw_row(out, l, l.first_entry_row, &line, true);
    draw_row(out, l, l.first_entry_row + 1, hint, false);
    if let Some(err) = error {
        draw_row(out, l, l.first_entry_row + 2, err, false);
    }
    draw_status(out, l, "Type value  Enter: OK  Esc: cancel  Backspace: delete", None);
}

/// The numeric `timeout` validator.
fn is_u32(s: &str) -> bool {
    s.parse::<u32>().is_ok()
}

/// A trimmed string, or `None` when empty (unsetting the key).
fn opt_str(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// `1`-`9`/`0` jump to an entry by position, like `lbm`'s list picker.
fn digit_value(c: char) -> Option<usize> {
    match c {
        '1'..='9' => Some((c as u8 - b'1') as usize),
        '0' => Some(9),
        _ => None,
    }
}

/// Seconds until the deadline, rounded up (the menu shows `timeout`, then 0).
fn remaining_seconds(elapsed_ms: u32, deadline_ms: u32) -> u32 {
    deadline_ms.saturating_sub(elapsed_ms).div_ceil(1000)
}

/// Lossy UTF-16 to UTF-8 for drawing, logging and JSON output only.
fn to_cstring(s: &str) -> CString16 {
    CString16::try_from(s).unwrap_or_else(|_| CString16::new())
}

/// Maps a UEFI keyboard-console key to the menu's logical keys. Enter/CR/LF
/// and Backspace become their logical keys, printable characters pass through
/// for the editors, and everything else is ignored.
fn key_from_conin(key: Key) -> Option<MenuKey> {
    match key {
        Key::Special(ScanCode::UP) => Some(MenuKey::Up),
        Key::Special(ScanCode::DOWN) => Some(MenuKey::Down),
        Key::Special(ScanCode::HOME) => Some(MenuKey::Home),
        Key::Special(ScanCode::END) => Some(MenuKey::End),
        Key::Special(ScanCode::PAGE_UP) => Some(MenuKey::PageUp),
        Key::Special(ScanCode::PAGE_DOWN) => Some(MenuKey::PageDown),
        Key::Special(ScanCode::ESCAPE) => Some(MenuKey::Esc),
        Key::Special(ScanCode::DELETE) => Some(MenuKey::Backspace),
        Key::Printable(c) => {
            let code: u16 = c.into();
            match code {
                0x0D | 0x0A => Some(MenuKey::Enter),
                0x08 => Some(MenuKey::Backspace),
                code => char::from_u32(code as u32).map(MenuKey::Printable),
            }
        }
        _ => None,
    }
}
