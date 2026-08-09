//! lbm — "Leon Boot Menuconfig"
//!
//! A kernel-menuconfig-style TUI for editing Leon's `boot.toml`.
//! Built with `cursive` (crossterm backend, pure Rust — no ncurses),
//! themed strictly black-and-white, globally, on every screen.
//!
//! The config model mirrors `leon_common::boot_config` (and the host-side
//! `lbc`/`lbt` tooling) exactly: the same five keys — `timeout`,
//! `default_entry`, `theme`, `splash`, `entries_file` — the same `Option`
//! semantics (unset keys are omitted on save, matching what `lbc config set`
//! writes), and output that the bootloader's parser always accepts.

use cursive::align::HAlign;
use cursive::theme::{BaseColor, BorderStyle, Color, Palette, PaletteColor, Theme};
use cursive::view::{Nameable, Resizable};
use cursive::views::{Dialog, EditView, LinearLayout, SelectView, TextView};
use cursive::Cursive;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::cell::RefCell;

// ─────────────────────────────────────────────────────────────────────────
// Config model
// ─────────────────────────────────────────────────────────────────────────

/// The boot config, byte-for-byte compatible with what `lbc config set`
/// writes and `leon_common::boot_config` reads: unset keys are omitted.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BootConfig {
    #[serde(default)]
    timeout: Option<u32>,
    #[serde(default)]
    default_entry: Option<String>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    splash: Option<bool>,
    #[serde(default)]
    entries_file: Option<String>,
}

fn load_boot_toml(path: &Path) -> (BootConfig, Option<String>) {
    match fs::read_to_string(path) {
        Ok(raw) => match toml::from_str::<BootConfig>(&raw) {
            Ok(cfg) => (cfg, None),
            Err(e) => (
                BootConfig::default(),
                Some(format!("Couldn't parse {}: {e} — starting from defaults.", path.display())),
            ),
        },
        Err(_) => (BootConfig::default(), None),
    }
}

fn write_boot_toml(path: &Path, cfg: &BootConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // `toml::to_string` (not pretty) matches the format `lbc config set` uses,
    // and skips `None` keys exactly like the host-side `BootConfig` does.
    let out = toml::to_string(cfg).expect("BootConfig always serializes");
    fs::write(path, out)
}

fn default_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/leon/boot.toml")
}

// ─────────────────────────────────────────────────────────────────────────
// Discovered entries (best-effort read of entries.jsonc, if reachable)
// ─────────────────────────────────────────────────────────────────────────

fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2 == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for c2 in chars.by_ref() {
                    if prev == '*' && c2 == '/' {
                        break;
                    }
                    prev = c2;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn describe_entry(v: &serde_json::Value) -> String {
    for key in ["label", "name", "path", "file", "vendor"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            return s.to_string();
        }
    }
    v.to_string()
}

fn load_entries(path: &Path) -> Option<Vec<String>> {
    let raw = fs::read_to_string(path).ok()?;
    let stripped = strip_jsonc_comments(&raw);
    let value: serde_json::Value = serde_json::from_str(&stripped).ok()?;
    // lbl writes `{ "entries": [ { "label": ..., "path": ... } ] }`.
    let arr = match value {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(mut o) => o.remove("entries")?.as_array()?.clone(),
        _ => return None,
    };
    Some(arr.iter().map(describe_entry).collect())
}

fn default_entries_search_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/boot/efi/EFI/leon/entries.jsonc"),
        PathBuf::from("/boot/EFI/leon/entries.jsonc"),
        PathBuf::from("/efi/EFI/leon/entries.jsonc"),
    ]
}

// ─────────────────────────────────────────────────────────────────────────
// App state
// ─────────────────────────────────────────────────────────────────────────

struct AppState {
    config_path: PathBuf,
    config: BootConfig,
    entries: Option<Vec<String>>,
    entries_path: Option<PathBuf>,
    dirty: bool,
}

type SharedState = Rc<RefCell<AppState>>;

// ─────────────────────────────────────────────────────────────────────────
// Theme — strict high-contrast black & white, applied globally
// ─────────────────────────────────────────────────────────────────────────

fn bw_theme() -> Theme {
    let mut theme = Theme {
        shadow: false,
        borders: BorderStyle::Simple,
        ..Theme::default()
    };

    let mut palette = Palette::default();
    palette[PaletteColor::Background] = Color::Dark(BaseColor::Black);
    palette[PaletteColor::View] = Color::Dark(BaseColor::Black);
    palette[PaletteColor::Primary] = Color::Light(BaseColor::White);
    palette[PaletteColor::Secondary] = Color::Light(BaseColor::White);
    palette[PaletteColor::Tertiary] = Color::Light(BaseColor::White);
    palette[PaletteColor::TitlePrimary] = Color::Light(BaseColor::White);
    palette[PaletteColor::TitleSecondary] = Color::Light(BaseColor::White);
    // Selected row: inverted (white bg, black text) — classic menuconfig look.
    palette[PaletteColor::Highlight] = Color::Light(BaseColor::White);
    palette[PaletteColor::HighlightInactive] = Color::Dark(BaseColor::White);
    palette[PaletteColor::HighlightText] = Color::Dark(BaseColor::Black);
    palette[PaletteColor::Shadow] = Color::Dark(BaseColor::Black);
    theme.palette = palette;
    theme
}

// ─────────────────────────────────────────────────────────────────────────
// Menu row formatting
// ─────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    ToggleSplash,
    EditTimeout,
    EditTheme,
    EditEntriesFile,
    EditDefaultEntry,
    ViewEntries,
    ViewSecureBoot,
    Save,
    SaveAndQuit,
    QuitWithoutSaving,
}

fn pad_label(label: &str, value: &str) -> String {
    format!("{label:<34}{value}")
}

fn build_main_menu(state: &SharedState) -> Dialog {
    let s = state.borrow();

    let mut select = SelectView::<MenuAction>::new().h_align(HAlign::Left);

    select.add_item(
        pad_label(
            "[Boot Timeout (seconds)]",
            &format!("({})", s.config.timeout.unwrap_or(5)),
        ),
        MenuAction::EditTimeout,
    );
    select.add_item(
        pad_label(
            "[Show Boot Menu (splash)]",
            if s.config.splash.unwrap_or(true) { "[*]" } else { "[ ]" },
        ),
        MenuAction::ToggleSplash,
    );
    select.add_item(
        pad_label(
            "[Splash Theme]",
            &format!("({})", s.config.theme.as_deref().unwrap_or("default")),
        ),
        MenuAction::EditTheme,
    );
    select.add_item(
        pad_label(
            "[Entries File]",
            s.config
                .entries_file
                .as_deref()
                .unwrap_or(r"\EFI\leon\entries.jsonc"),
        ),
        MenuAction::EditEntriesFile,
    );
    select.add_item(
        pad_label(
            "[Default Boot Entry]",
            &format!("({})", s.config.default_entry.as_deref().unwrap_or("auto")),
        ),
        MenuAction::EditDefaultEntry,
    );
    select.add_item(
        pad_label(
            "[Discovered Boot Entries]",
            "--->",
        ),
        MenuAction::ViewEntries,
    );
    select.add_item(
        pad_label("[Secure Boot Status]", "--->"),
        MenuAction::ViewSecureBoot,
    );

    let dirty_marker = if s.dirty { " (unsaved changes)" } else { "" };
    let config_path = s.config_path.display().to_string();
    drop(s);

    let state_for_submit = Rc::clone(state);
    select.set_on_submit(move |siv, action: &MenuAction| {
        dispatch(siv, &state_for_submit, *action);
    });

    let layout = LinearLayout::vertical()
        .child(TextView::new(format!("Editing: {config_path}{dirty_marker}")))
        .child(TextView::new(""))
        .child(select)
        .child(TextView::new(""))
        .child(TextView::new("Enter: select/toggle   Ctrl+S: save   Ctrl+Q: quit"));

    Dialog::around(layout)
        .title("Leon Boot Menuconfig")
        .button("Save", {
            let state = Rc::clone(state);
            move |siv| dispatch(siv, &state, MenuAction::Save)
        })
        .button("Save & Exit", {
            let state = Rc::clone(state);
            move |siv| dispatch(siv, &state, MenuAction::SaveAndQuit)
        })
        .button("Exit", {
            let state = Rc::clone(state);
            move |siv| dispatch(siv, &state, MenuAction::QuitWithoutSaving)
        })
}

fn rebuild_menu(siv: &mut Cursive, state: &SharedState) {
    siv.pop_layer();
    siv.add_layer(build_main_menu(state));
}

fn info_dialog(siv: &mut Cursive, title: &str, body: String) {
    siv.add_layer(
        Dialog::around(TextView::new(body))
            .title(title)
            .button("Back", |s| {
                s.pop_layer();
            }),
    );
}

fn dispatch(siv: &mut Cursive, state: &SharedState, action: MenuAction) {
    match action {
        MenuAction::ToggleSplash => {
            {
                let mut s = state.borrow_mut();
                s.config.splash = Some(!s.config.splash.unwrap_or(true));
                s.dirty = true;
            }
            rebuild_menu(siv, state);
        }

        MenuAction::EditTimeout => {
            let current = state
                .borrow()
                .config
                .timeout
                .map(|t| t.to_string())
                .unwrap_or_default();
            let state = Rc::clone(state);
            siv.add_layer(
                Dialog::around(
                    EditView::new()
                        .content(current)
                        .with_name("edit_timeout")
                        .fixed_width(20),
                )
                .title("Boot timeout, in seconds")
                .button("OK", {
                    let state = Rc::clone(&state);
                    move |siv| {
                        let value = siv
                            .call_on_name("edit_timeout", |v: &mut EditView| v.get_content())
                            .unwrap();
                        match value.parse::<u32>() {
                            Ok(n) => {
                                {
                                    let mut s = state.borrow_mut();
                                    s.config.timeout = Some(n);
                                    s.dirty = true;
                                }
                                siv.pop_layer();
                                rebuild_menu(siv, &state);
                            }
                            Err(_) => {
                                siv.add_layer(
                                    Dialog::info("Enter a whole number of seconds (e.g. 5).")
                                        .title("Invalid value"),
                                );
                            }
                        }
                    }
                })
                .button("Cancel", |siv| {
                    siv.pop_layer();
                }),
            );
        }

        MenuAction::EditTheme | MenuAction::EditEntriesFile => {
            let (title, name, current) = {
                let s = state.borrow();
                match action {
                    MenuAction::EditTheme => (
                        "Splash theme file",
                        "edit_theme",
                        s.config.theme.clone().unwrap_or_default(),
                    ),
                    _ => (
                        "Entries file path (\\EFI\\leon\\...)",
                        "edit_entries_file",
                        s.config.entries_file.clone().unwrap_or_default(),
                    ),
                }
            };
            let state = Rc::clone(state);
            siv.add_layer(
                Dialog::around(
                    EditView::new()
                        .content(current)
                        .with_name(name)
                        .fixed_width(60),
                )
                .title(title)
                .button("OK", {
                    let state = Rc::clone(&state);
                    move |siv| {
                        let value = siv
                            .call_on_name(name, |v: &mut EditView| v.get_content())
                            .unwrap();
                        {
                            let mut s = state.borrow_mut();
                            if name == "edit_theme" {
                                s.config.theme = Some(value.trim().to_string());
                            } else {
                                s.config.entries_file = Some(value.trim().to_string());
                            }
                            s.dirty = true;
                        }
                        siv.pop_layer();
                        rebuild_menu(siv, &state);
                    }
                })
                .button("Cancel", |siv| {
                    siv.pop_layer();
                }),
            );
        }

        MenuAction::EditDefaultEntry => {
            let (current, entries) = {
                let s = state.borrow();
                (
                    s.config.default_entry.clone().unwrap_or_default(),
                    s.entries.clone(),
                )
            };
            let state = Rc::clone(state);

            let mut layout = LinearLayout::vertical();

            if let Some(entries) = entries.filter(|e| !e.is_empty()) {
                let mut list = SelectView::<String>::new().h_align(HAlign::Left);
                for e in entries {
                    list.add_item(e.clone(), e);
                }
                let state_pick = Rc::clone(&state);
                list.set_on_submit(move |siv, value: &String| {
                    {
                        let mut s = state_pick.borrow_mut();
                        s.config.default_entry = Some(value.clone());
                        s.dirty = true;
                    }
                    siv.pop_layer();
                    rebuild_menu(siv, &state_pick);
                });
                layout.add_child(TextView::new(
                    "Pick a discovered entry, or type a custom path below:",
                ));
                layout.add_child(list);
                layout.add_child(TextView::new(""));
            } else {
                layout.add_child(TextView::new(
                    "No entries.jsonc found — enter the entry path manually\n\
                     (e.g. \\EFI\\leon\\kernel.efi). Leave blank for auto.",
                ));
            }

            layout.add_child(
                EditView::new()
                    .content(current)
                    .with_name("edit_default_entry")
                    .fixed_width(50),
            );

            siv.add_layer(
                Dialog::around(layout)
                    .title("Default Boot Entry")
                    .button("OK", {
                        let state = Rc::clone(&state);
                        move |siv| {
                            let value = siv
                                .call_on_name("edit_default_entry", |v: &mut EditView| {
                                    v.get_content()
                                })
                                .unwrap();
                            {
                                let mut s = state.borrow_mut();
                                s.config.default_entry = if value.trim().is_empty() {
                                    None
                                } else {
                                    Some(value.trim().to_string())
                                };
                                s.dirty = true;
                            }
                            siv.pop_layer();
                            rebuild_menu(siv, &state);
                        }
                    })
                    .button("Cancel", |siv| {
                        siv.pop_layer();
                    }),
            );
        }

        MenuAction::ViewEntries => {
            let s = state.borrow();
            let body = match (&s.entries, &s.entries_path) {
                (Some(entries), Some(path)) if !entries.is_empty() => {
                    let mut body = format!("From {}:\n\n", path.display());
                    for e in entries {
                        body.push_str("  - ");
                        body.push_str(e);
                        body.push('\n');
                    }
                    body
                }
                _ => {
                    "No entries.jsonc was found at any of the usual ESP mount points.\n\n\
                     Leon writes this file to \\EFI\\leon\\entries.jsonc on every boot —\n\
                     mount your ESP and pass its path as the 2nd argument to lbm, e.g.:\n\n\
                     \x20\x20lbm ~/.config/leon/boot.toml /boot/efi/EFI/leon/entries.jsonc"
                        .to_string()
                }
            };
            drop(s);
            info_dialog(siv, "Discovered Boot Entries", body);
        }

        MenuAction::ViewSecureBoot => {
            info_dialog(
                siv,
                "Secure Boot Status",
                "Secure Boot / Setup Mode are read live from UEFI globals by Leon\n\
                 itself at boot time — this host-side tool has no way to query them.\n\n\
                 If Secure Boot is on, Leon shows a warning on its own boot menu and\n\
                 logs it to \\var\\logs\\leon\\log.md. Unsigned entries are rejected by\n\
                 firmware (ACCESS_DENIED / SECURITY_VIOLATION) and logged with a hint\n\
                 to sign them or enroll their key.\n\n\
                 To sign your staged tree: `scripts/sign.sh setup`, then `make sign`\n\
                 after every rebuild. See docs/secure-boot.md for the full walkthrough."
                    .to_string(),
            );
        }

        MenuAction::Save => {
            let (path, cfg) = {
                let s = state.borrow();
                (s.config_path.clone(), s.config.clone())
            };
            match write_boot_toml(&path, &cfg) {
                Ok(()) => {
                    state.borrow_mut().dirty = false;
                    rebuild_menu(siv, state);
                }
                Err(e) => {
                    siv.add_layer(
                        Dialog::info(format!("Couldn't write {}: {e}", path.display()))
                            .title("Save failed"),
                    );
                }
            }
        }

        MenuAction::SaveAndQuit => {
            let (path, cfg) = {
                let s = state.borrow();
                (s.config_path.clone(), s.config.clone())
            };
            match write_boot_toml(&path, &cfg) {
                Ok(()) => siv.quit(),
                Err(e) => {
                    siv.add_layer(
                        Dialog::info(format!("Couldn't write {}: {e}", path.display()))
                            .title("Save failed"),
                    );
                }
            }
        }

        MenuAction::QuitWithoutSaving => {
            if state.borrow().dirty {
                siv.add_layer(
                    Dialog::text("You have unsaved changes. Quit anyway?")
                        .title("Confirm")
                        .button("Quit without saving", |siv| siv.quit())
                        .button("Back", |siv| {
                            siv.pop_layer();
                        }),
                );
            } else {
                siv.quit();
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────

fn main() {
    let mut args = std::env::args().skip(1);
    let config_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let entries_arg = args.next().map(PathBuf::from);

    let (config, parse_warning) = load_boot_toml(&config_path);

    let (entries, entries_path) = if let Some(p) = entries_arg {
        (load_entries(&p), Some(p))
    } else {
        default_entries_search_paths()
            .into_iter()
            .find_map(|p| load_entries(&p).map(|e| (Some(e), Some(p))))
            .unwrap_or((None, None))
    };

    let state: SharedState = Rc::new(RefCell::new(AppState {
        config_path,
        config,
        entries,
        entries_path,
        dirty: false,
    }));

    let mut siv = cursive::crossterm();
    siv.set_theme(bw_theme());

    siv.add_global_callback(cursive::event::Event::CtrlChar('q'), {
        let state = Rc::clone(&state);
        move |siv| dispatch(siv, &state, MenuAction::QuitWithoutSaving)
    });
    siv.add_global_callback(cursive::event::Event::CtrlChar('s'), {
        let state = Rc::clone(&state);
        move |siv| dispatch(siv, &state, MenuAction::Save)
    });

    siv.add_layer(build_main_menu(&state));

    if let Some(msg) = parse_warning {
        info_dialog(&mut siv, "Notice", msg);
    }

    siv.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn cfg() -> BootConfig {
        BootConfig {
            timeout: Some(5),
            default_entry: Some("Cudane Linux".to_string()),
            theme: Some("splash.py".to_string()),
            splash: Some(true),
            entries_file: Some(r"\EFI\leon\entries.jsonc".to_string()),
        }
    }

    #[test]
    fn roundtrips_through_toml() {
        let s = toml::to_string(&cfg()).unwrap();
        let back: BootConfig = toml::from_str(&s).unwrap();
        assert_eq!(back, cfg());
    }

    #[test]
    fn unset_keys_are_omitted_like_lbc() {
        // `lbc config set` omits unset keys; lbm must do the same.
        let s = toml::to_string(&BootConfig::default()).unwrap();
        assert!(s.trim().is_empty());
        let back: BootConfig = toml::from_str(&s).unwrap();
        assert_eq!(back, BootConfig::default());
    }

    #[test]
    fn output_is_parseable_by_the_bootloader() {
        // Every key lbm writes must survive `leon_common::boot_config`, the
        // parser the bootloader runs on every boot.
        let s = toml::to_string(&cfg()).unwrap();
        let parsed = leon_common::boot_config::parse_boot_config(&s).unwrap();
        assert_eq!(parsed.timeout, Some(5));
        assert_eq!(parsed.default_entry.as_deref(), Some("Cudane Linux"));
        assert_eq!(parsed.theme.as_deref(), Some("splash.py"));
        assert_eq!(parsed.splash, Some(true));
        assert_eq!(
            parsed.entries_file.as_deref(),
            Some(r"\EFI\leon\entries.jsonc")
        );
    }

    #[test]
    fn loads_the_bootloaders_entries_file() {
        let dir = std::env::temp_dir().join("lbm-entries-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("entries.jsonc");
        std::fs::write(
            &p,
            "// Auto-generated by lbl on every boot. Do not edit.\n\
             {\n  \"entries\": [\n    { \"label\": \"kernel\", \"path\": \"\\\\EFI\\\\leon\\\\kernel.efi\" }\n  ]\n}\n",
        )
        .unwrap();
        let entries = load_entries(Path::new(&p)).unwrap();
        assert_eq!(entries, vec!["kernel"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
