//! implementation of `lbc tui`: the keyboard-driven boot-manager
//! menu.
//!
//! Renders the boot entries from a [`Context`](super::tui::Context) in a
//! rounded, full-screen menu — one row per entry, a `▶` marker on the
//! selection, a header with the count + framebuffer geometry, a filter/search
//! line, a details pane, a boot-preview pane, and a help overlay. Pure
//! terminal graphics (monochrome: the default greys and an inverted selection
//! bar); no bitmaps.

use std::io;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::{Frame, Terminal};

use super::tui::Context;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Keys: `↑↓/jk` move, `PgUp/PgDn` page, `Home/End` jump, `1-9/0` jump to an
/// entry by position, `/` search (live filter), `n` next match / `N`,`p`
/// previous match, `d` details, `h` help, `r` refresh, `s`/space/Enter
/// boot-preview, `Esc` dismiss (help → preview → filter → quit), `q` quit.
const FOOTER_HINT: &str = "↑↓/jk move · PgUp/PgDn · Home/End · 1-9 jump · / search · n/N next/prev · d details · \
     h/? help · r refresh · s/space preview · Enter boot · Esc dismiss · q/Ctrl+C quit";

const HELP_LINES: &[&str] = &[
    "LEON Boot Manager — keys",
    "",
    "  ↑ / ↓ or k / j      move selection",
    "  PgUp / PgDn          page up / down",
    "  Home / End           first / last entry",
    "  1-9 / 0              jump to entry 1-10",
    "  / <text>             filter entries (substring match)",
    "  Ctrl+U               clear search",
    "  n / N / p            next / previous match",
    "  d                    toggle the details pane",
    "  h / ?                this help overlay",
    "  Ctrl+C               quit",
    "  r                    re-read geometry, entries, boot config",
    "  s / Space / Enter    preview the selected entry (boot preview)",
    "  Esc                  dismiss help / preview / filter, then quit",
    "  q                    quit",
];

/// A boot-preview pane (what `s`/space/Enter shows).
#[derive(Default)]
struct Preview {
    open: bool,
}

/// The menu application. Drawn once per event; state is kept here so every
/// redraw is deterministic.
pub struct App {
    context: Context,
    /// Indices into `context.entries` currently visible (after the filter).
    filtered: Vec<usize>,
    /// Position within `filtered` that the `▶` marker points at.
    selected: usize,
    /// Top visible row within `filtered`.
    scroll_offset: usize,
    /// Cached number of list rows currently visible.
    page_height: usize,
    /// The `/` search query. Empty means no filter. `editing` is true while
    /// the user is typing (the prompt line is shown).
    filter: String,
    editing: bool,
    show_details: bool,
    show_help: bool,
    preview: Preview,
    /// One-shot status message shown in the header (e.g. after a refresh).
    status: Option<String>,
    quit: bool,
}

impl App {
    pub fn new(context: Context) -> Self {
        let mut app = Self {
            context,
            filtered: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            page_height: 0,
            filter: String::new(),
            editing: false,
            show_details: false,
            show_help: false,
            preview: Preview::default(),
            status: None,
            quit: false,
        };
        app.rebuild_filter();
        app
    }

    /// Runs the menu until the user quits.
    pub fn run(mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor()?;

        let result = self.loop_events(&mut terminal);
        let _ = terminal.show_cursor();
        let _ = terminal.clear();
        let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        result
    }

    fn loop_events(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        while !self.quit {
            terminal.draw(|frame| self.draw(frame))?;
            match event::read()? {
                Event::Key(key) => self.on_key(key),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        Ok(())
    }

    fn on_key(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        if self.editing {
            self.on_key_editing(key);
        } else {
            self.on_key_browsing(key);
        }
    }

    fn on_key_editing(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => {
                if c.is_control() {
                    return;
                }
                self.filter.push(c);
                self.rebuild_filter();
            }
            KeyCode::Backspace | KeyCode::Delete => {
                self.filter.pop();
                self.rebuild_filter();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.clear();
                self.rebuild_filter();
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
            }
            KeyCode::Enter => {
                self.editing = false;
                self.rebuild_filter();
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.editing = false;
                self.rebuild_filter();
            }
            _ => {}
        }
    }

    fn on_key_browsing(&mut self, key: KeyEvent) {
        self.status = None;
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true
            }
            // Esc dismisses whatever is open: help, then preview, then a
            // committed filter, then the app itself.
            KeyCode::Esc => {
                if self.show_help {
                    self.show_help = false;
                } else if self.preview.open {
                    self.preview.open = false;
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild_filter();
                    self.selected = 0;
                } else {
                    self.quit = true;
                }
            }
            KeyCode::Char('/') => {
                self.filter.clear();
                self.editing = true;
                self.show_help = false;
                self.preview.open = false;
                self.rebuild_filter();
            }
            KeyCode::Char('n') => self.move_selection(1),
            KeyCode::Char('N') | KeyCode::Char('p') => self.move_selection(-1),
            KeyCode::Char('d') => self.show_details = !self.show_details,
            KeyCode::Char('h') | KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char('r') => {
                self.context = Context::load();
                self.filter.clear();
                self.editing = false;
                self.show_help = false;
                self.preview.open = false;
                self.selected = 0;
                self.scroll_offset = 0;
                self.rebuild_filter();
                self.status = Some(format!(
                    "refreshed: {} entries, {} shown",
                    self.context.entries.len(),
                    self.filtered.len()
                ));
            }
            KeyCode::Char('s') | KeyCode::Char(' ') | KeyCode::Enter => {
                self.preview.open = !self.preview.open;
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let nth = if c == '0' {
                    9
                } else {
                    (c as u8 - b'1') as usize
                };
                if nth < self.filtered.len() {
                    self.selected = nth;
                    self.ensure_visible();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection_page(-1),
            KeyCode::PageDown => self.move_selection_page(1),
            KeyCode::Home => self.move_selection_to_start(),
            KeyCode::End => self.move_selection_to_end(),
            _ => {}
        }
    }

    /// Recomputes the visible entry list from the filter and clamps the
    /// selection into range. Matches on the entry label (as the classic
    /// npyscreen menu did — paths all share the `/efi/` prefix, so matching
    /// them too would make `/` searches like "efi" match everything).
    fn rebuild_filter(&mut self) {
        let query = self.filter.to_lowercase();
        self.filtered = self
            .context
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| query.is_empty() || e.label.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.ensure_visible();
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let mut next = self.selected as isize + delta;
        next = next.clamp(0, len - 1);
        self.selected = next as usize;
        self.ensure_visible();
    }

    fn move_selection_page(&mut self, direction: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let page = self.page_height.max(1) as isize;
        let len = self.filtered.len() as isize;
        let next = (self.selected as isize + direction * page).clamp(0, len - 1);
        self.selected = next as usize;
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        if self.page_height == 0 || self.filtered.is_empty() {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.page_height {
            self.scroll_offset = self.selected + 1 - self.page_height;
        }
    }

    fn move_selection_to_start(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = 0;
            self.scroll_offset = 0;
        }
    }

    fn move_selection_to_end(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = self.filtered.len() - 1;
            if self.page_height > 0 && self.selected + 1 > self.page_height {
                self.scroll_offset = self.selected + 1 - self.page_height;
            }
        }
    }

    fn selected_entry(&self) -> Option<&super::tui::Entry> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.context.entries.get(i))
    }

    // -- drawing ----------------------------------------------------------

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.buffer_mut().area;
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(if self.editing { 1 } else { 0 }),
            Constraint::Min(0),
            Constraint::Length(if self.show_details { 6 } else { 0 }),
            Constraint::Length(1),
        ])
        .split(area);

        self.draw_header(frame, chunks[0]);
        if self.editing {
            self.draw_search(frame, chunks[1]);
        }
        self.draw_list(frame, chunks[2]);
        if self.show_details {
            self.draw_details(frame, chunks[3]);
        }
        self.draw_footer(frame, chunks[4]);

        if self.show_help {
            self.draw_help(frame, area);
        }
        if self.preview.open && !self.show_help {
            self.draw_preview(frame, area);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let total = self.context.entries.len();
        let shown = self.filtered.len();
        let geo = if self.context.width > 0 {
            format!(
                "fb {}x{} stride {} {}",
                self.context.width, self.context.height, self.context.stride, self.context.format
            )
        } else {
            "fb: not available".to_string()
        };
        let mut spans = vec![
            Span::raw(" LEON Boot Manager — "),
            Span::styled(
                format!("{shown}/{total} entries"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  ·  {geo}")),
        ];
        if !self.filter.is_empty() {
            spans.push(Span::styled(
                format!("  ·  filter: “{}”", self.filter),
                Style::default().fg(Color::DarkGray),
            ));
        }
        if let Some(status) = &self.status {
            spans.push(Span::styled(
                format!("  ·  {status}"),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_search(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!(" / {}▏", self.filter),
                Style::default().add_modifier(Modifier::REVERSED),
            )),
            area,
        );
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" LEON Boot Manager ");
        let inner = block.inner(area);

        self.page_height = inner.height as usize;
        self.ensure_visible();
        let start = self.scroll_offset.min(self.filtered.len());

        let mut lines: Vec<Line> = Vec::with_capacity(inner.height as usize + 1);
        for (visible_row, entry_idx) in self.filtered.iter().skip(start).enumerate() {
            if visible_row >= inner.height as usize {
                break;
            }
            let entry = &self.context.entries[*entry_idx];
            lines.push(self.entry_line(start + visible_row, entry));
        }

        if lines.is_empty() {
            lines.push(Line::styled(
                if self.filter.is_empty() {
                    " no boot entries found (run `lbc entries list`)".to_string()
                } else {
                    format!(" no entries match “{}”", self.filter)
                },
                Style::default().fg(Color::DarkGray),
            ));
        }

        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn entry_line(&self, row: usize, entry: &super::tui::Entry) -> Line<'static> {
        let selected = row == self.selected;
        let marker = if selected { "▶" } else { " " };
        let number = format!("{:>2}", row + 1);
        let prefix = format!(" {marker} {number} ");
        let style = if selected {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
        };
        let label = if entry.label.is_empty() {
            entry.path.clone()
        } else {
            entry.label.clone()
        };
        let label_style = style;
        let path_style = Style::default().fg(Color::DarkGray);
        let mut line = Line::default();
        line.push_span(Span::styled(prefix, style));
        line.push_span(Span::styled(label, label_style));
        line.push_span(Span::styled("  ", style));
        line.push_span(Span::styled(
            entry.path.clone(),
            if selected { style } else { path_style },
        ));
        line
    }

    fn draw_details(&self, frame: &mut Frame, area: Rect) {
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Details ");
        let mut lines = vec![Line::styled(
            "no entry selected".to_string(),
            Style::default().fg(Color::DarkGray),
        )];
        if let Some(entry) = self.selected_entry() {
            let cfg = [
                ("timeout", self.context.timeout.as_str()),
                ("default_entry", self.context.default.as_str()),
                ("splash", self.context.splash.as_str()),
            ];
            lines = vec![
                Line::styled(format!(" label:   {}", entry.label), Style::default()),
                Line::styled(format!(" path:    {}", entry.path), Style::default()),
                Line::styled(
                    format!(" source:  {}", entry.source),
                    Style::default().fg(Color::DarkGray),
                ),
                Line::styled(
                    format!(
                        " boot:    timeout {} · default {} · splash {}",
                        cfg[0].1, cfg[1].1, cfg[2].1
                    ),
                    Style::default().fg(Color::DarkGray),
                ),
            ];
        }
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Paragraph::new(Line::styled(
                FOOTER_HINT.to_string(),
                Style::default().fg(Color::DarkGray),
            )),
            area,
        );
    }

    fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Help — press h to close ");
        let lines: Vec<Line> = HELP_LINES
            .iter()
            .map(|l| Line::styled((*l).to_string(), Style::default()))
            .collect();
        let max_width = area.width.saturating_sub(8).max(20);
        let width = max_width.min(60);
        let height = (HELP_LINES.len() + 2) as u16;
        frame.render_widget(
            Paragraph::new(lines).block(block),
            centered(area, width, height.min(area.height.saturating_sub(4))),
        );
    }

    fn draw_preview(&self, frame: &mut Frame, area: Rect) {
        let lines: Vec<Line> = match self.selected_entry() {
            Some(entry) => vec![
                Line::styled(
                    format!("  label:  {}", entry.label),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::styled(format!("  path:   {}", entry.path), Style::default()),
                Line::styled(
                    "  (preview — s/space/Enter toggles, Esc closes)".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ],
            None => vec![Line::styled(
                "  no entry selected".to_string(),
                Style::default(),
            )],
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Boot preview ");
        let width = area.width.saturating_sub(8).min(60).max(20);
        let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(4));
        frame.render_widget(
            Paragraph::new(lines).block(block),
            centered(area, width, height),
        );
    }
}

/// Returns a rect of `w`×`h` cells centered in `area` (clamped to fit).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn ctx() -> Context {
        use super::super::tui::Entry;
        Context {
            entries: vec![
                Entry {
                    label: "Arch Linux (efi)".into(),
                    path: "/efi/arch/vmlinuz".into(),
                    source: "test".into(),
                },
                Entry {
                    label: "Linux 6.6.30-lts".into(),
                    path: "/efi/linux/vmlinuz".into(),
                    source: "test".into(),
                },
                Entry {
                    label: "Windows Boot Manager".into(),
                    path: "/efi/microsoft/bootmgfw.efi".into(),
                    source: "test".into(),
                },
                Entry {
                    label: "Debian (efi)".into(),
                    path: "/efi/debian/vmlinuz".into(),
                    source: "test".into(),
                },
                Entry {
                    label: "Fedora Workstation".into(),
                    path: "/efi/fedora/vmlinuz".into(),
                    source: "test".into(),
                },
                Entry {
                    label: "Memtest86+".into(),
                    path: "/efi/memtest/memtest.bin".into(),
                    source: "test".into(),
                },
            ],
            ..Default::default()
        }
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        });
    }

    #[test]
    fn filter_narrows_and_commits() {
        let mut app = App::new(ctx());
        assert_eq!(app.filtered.len(), 6);
        press(&mut app, KeyCode::Char('/'));
        assert!(app.editing);
        for c in "efi".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.filter, "efi");
        assert_eq!(app.filtered, vec![0, 3]);
        press(&mut app, KeyCode::Enter);
        assert!(!app.editing);
        assert_eq!(app.filtered, vec![0, 3]);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.filter, "");
        assert_eq!(app.filtered.len(), 6);
    }

    #[test]
    fn digits_navigation_and_non_wrapping_bounds() {
        let mut app = App::new(ctx());
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.selected, 2);
        press(&mut app, KeyCode::Char('0'));
        // '0' maps to entry 10, which is beyond the 6 entries → no-op.
        assert_eq!(app.selected, 2);
        press(&mut app, KeyCode::End);
        assert_eq!(app.selected, 5);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected, 5);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected, 4);
    }

    #[test]
    fn details_and_preview_toggle() {
        let mut app = App::new(ctx());
        assert!(!app.show_details);
        press(&mut app, KeyCode::Char('d'));
        assert!(app.show_details);
        press(&mut app, KeyCode::Char('d'));
        assert!(!app.show_details);
        press(&mut app, KeyCode::Char(' '));
        assert!(app.preview.open);
        press(&mut app, KeyCode::Esc);
        assert!(!app.preview.open);
        assert!(!app.quit);
    }

    #[test]
    fn question_mark_toggles_help() {
        let mut app = App::new(ctx());
        assert!(!app.show_help);
        press(&mut app, KeyCode::Char('?'));
        assert!(app.show_help);
        press(&mut app, KeyCode::Char('?'));
        assert!(!app.show_help);
    }

    #[test]
    fn page_up_page_down_moves_selection_by_page() {
        let mut app = App::new(ctx());
        app.filtered = (0..20).collect();
        app.page_height = 5;
        app.selected = 0;
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected, 5);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected, 10);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.selected, 5);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn ensure_visible_scrolls_selection_into_view() {
        let mut app = App::new(ctx());
        app.filtered = (0..20).collect();
        app.page_height = 5;
        app.selected = 0;
        app.scroll_offset = 0;
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected, 5);
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = App::new(ctx());
        app.on_key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        });
        assert!(app.quit);
    }

    #[test]
    fn ctrl_u_clears_search() {
        let mut app = App::new(ctx());
        press(&mut app, KeyCode::Char('/'));
        for c in "arch".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.filter, "arch");
        app.on_key(KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        });
        assert_eq!(app.filter, "");
        assert!(app.editing);
    }

    #[test]
    fn delete_backspace_work_in_search() {
        let mut app = App::new(ctx());
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('b'));
        assert_eq!(app.filter, "ab");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.filter, "a");
        press(&mut app, KeyCode::Delete);
        assert_eq!(app.filter, "");
    }

    #[test]
    fn esc_dismisses_then_quits() {
        let mut app = App::new(ctx());
        press(&mut app, KeyCode::Esc);
        assert!(app.quit);
        let mut app = App::new(ctx());
        press(&mut app, KeyCode::Char('/'));
        for c in "win".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.filtered, vec![2]);
        press(&mut app, KeyCode::Esc);
        assert!(!app.quit);
        assert_eq!(app.filter, "");
        press(&mut app, KeyCode::Esc);
        assert!(app.quit);
    }

    #[test]
    fn refresh_resets_ui_state() {
        let mut app = App::new(ctx());
        app.show_help = true;
        app.preview.open = true;
        app.filter = "arch".into();
        app.rebuild_filter();
        app.on_key(KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        });
        assert!(!app.show_help);
        assert!(!app.preview.open);
        assert_eq!(app.filter, "");
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn slash_search_clears_preview_help() {
        let mut app = App::new(ctx());
        app.show_help = true;
        app.preview.open = true;
        press(&mut app, KeyCode::Char('/'));
        assert!(!app.show_help);
        assert!(!app.preview.open);
        assert!(app.editing);
    }
}
