//! Full end-to-end keymap test + preview for the boot-manager TUI.
//!
//! Boots the real `lbt tui` (the pure-Rust ratatui menu) in a pty, drives
//! every keybinding, asserts the menu responds, and prints every frame.
//!
//! The menu renders through ratatui/crossterm, which paints cells in place
//! rather than repainting whole frames, so the pty output is reconstructed
//! into a logical terminal grid and every assertion reads that grid.
//!
//! Requires a Linux host. Run from the workspace root:
//!
//! ```text
//! cargo test -p lbt --test tui_full -- --nocapture
//! ```
//!
//! `--nocapture` is what makes this a preview: every frame of the TUI is
//! printed as the keys are driven.

#![cfg(target_os = "linux")]

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const COLS: usize = 120;
const ROWS: usize = 40;
const BOOT_MARKER: &str = "LEON Boot Manager";

// Deterministic test data: 6 entries, of which "efi" matches exactly two
// ("Arch Linux (efi)" and "Debian (efi)") — the `/` filter matches labels
// only, so the `/efi/` path prefix must not make everything match.
const ENTRIES: &str = "Arch Linux (efi)\t/efi/arch/vmlinuz-linux\n\
Linux 6.6.30-lts\t/efi/linux/vmlinuz\n\
Windows Boot Manager\t/efi/microsoft/bootmgfw.efi\n\
Debian (efi)\t/efi/debian/vmlinuz\n\
Fedora Workstation\t/efi/fedora/vmlinuz\n\
Memtest86+\t/efi/memtest/memtest.bin";

// ── pty plumbing ────────────────────────────────────────────────────────────

fn dup(fd: RawFd) -> OwnedFd {
    let d = unsafe { libc::dup(fd) };
    assert!(d >= 0, "dup failed: {}", io::Error::last_os_error());
    unsafe { OwnedFd::from_raw_fd(d) }
}

fn open_pty() -> (OwnedFd, OwnedFd) {
    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    assert!(master >= 0, "posix_openpt: {}", io::Error::last_os_error());
    assert_eq!(
        unsafe { libc::grantpt(master) },
        0,
        "grantpt: {}",
        io::Error::last_os_error()
    );
    assert_eq!(
        unsafe { libc::unlockpt(master) },
        0,
        "unlockpt: {}",
        io::Error::last_os_error()
    );

    let mut name = [0 as libc::c_char; 256];
    assert_eq!(
        unsafe { libc::ptsname_r(master, name.as_mut_ptr(), name.len()) },
        0,
        "ptsname_r: {}",
        io::Error::last_os_error()
    );
    let name = unsafe { CStr::from_ptr(name.as_ptr()) };
    let slave = unsafe { libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    assert!(slave >= 0, "open slave: {}", io::Error::last_os_error());

    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(slave, &mut t) }, 0);
    t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    t.c_cc[libc::VMIN] = 1;
    t.c_cc[libc::VTIME] = 0;
    assert_eq!(
        unsafe { libc::tcsetattr(slave, libc::TCSANOW, &t) },
        0,
        "tcsetattr: {}",
        io::Error::last_os_error()
    );

    let win = libc::winsize {
        ws_row: ROWS as u16,
        ws_col: COLS as u16,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe { libc::ioctl(slave, libc::TIOCSWINSZ as _, &win) };

    (unsafe { OwnedFd::from_raw_fd(master) }, unsafe {
        OwnedFd::from_raw_fd(slave)
    })
}

fn set_nonblocking(fd: RawFd) {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(flags >= 0, "F_GETFL: {}", io::Error::last_os_error());
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0,
        "F_SETFL: {}",
        io::Error::last_os_error()
    );
}

fn read_some(fd: RawFd, timeout: Duration) -> Option<Vec<u8>> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(u32::MAX as u128) as i32;
    let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
    if rc <= 0 {
        return None;
    }
    let mut out = Vec::new();
    loop {
        let mut tmp = [0u8; 65536];
        let n = unsafe { libc::read(fd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len()) };
        if n > 0 {
            out.extend_from_slice(&tmp[..n as usize]);
        } else if n < 0 {
            match io::Error::last_os_error().raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => break,
                _ => break,
            }
        } else {
            break; // EOF
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

// ── terminal grid reconstruction ───────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum PState {
    Text,
    Esc,
    Csi,
    Ss3,
}

/// Rebuilds the logical screen from the raw pty byte stream. ratatui paints
/// cells with cursor-addressed writes (`CSI row;col H`) interleaved with
/// SGR/CSI decorations, so we track the cursor and apply only printable
/// characters.
struct Screen {
    cells: Vec<Vec<char>>,
    cx: usize,
    cy: usize,
    state: PState,
    params: Vec<u8>,
    utf8: Vec<u8>,
}

impl Screen {
    fn new() -> Self {
        Screen {
            cells: vec![vec![' '; COLS]; ROWS],
            cx: 0,
            cy: 0,
            state: PState::Text,
            params: Vec::new(),
            utf8: Vec::new(),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.feed_byte(b);
        }
    }

    fn feed_byte(&mut self, b: u8) {
        match self.state {
            PState::Text => {
                if b == 0x1b {
                    self.state = PState::Esc;
                } else if b < 0x20 || b == 0x7f {
                    self.apply_control(b);
                } else {
                    self.collect_char(b);
                }
            }
            PState::Esc => match b {
                b'[' => {
                    self.params.clear();
                    self.state = PState::Csi;
                }
                b'O' => self.state = PState::Ss3,
                _ => self.state = PState::Text,
            },
            PState::Csi => {
                if b == 0x1b {
                    self.params.clear();
                    self.state = PState::Esc;
                } else if (0x40..=0x7e).contains(&b) {
                    self.finish_csi(b);
                    self.state = PState::Text;
                } else {
                    self.params.push(b);
                }
            }
            PState::Ss3 => {
                if b == 0x1b {
                    self.state = PState::Esc;
                } else {
                    self.state = PState::Text;
                }
            }
        }
    }

    fn collect_char(&mut self, b: u8) {
        self.utf8.push(b);
        match std::str::from_utf8(&self.utf8) {
            Ok(s) => {
                if let Some(ch) = s.chars().next() {
                    self.put(ch);
                }
                self.utf8.clear();
            }
            Err(_) => { /* wait for more continuation bytes */ }
        }
    }

    fn apply_control(&mut self, b: u8) {
        match b {
            b'\r' => self.cx = 0,
            b'\n' => self.cy = (self.cy + 1).min(ROWS - 1),
            _ => {}
        }
    }

    fn finish_csi(&mut self, finalb: u8) {
        if finalb != b'H' && finalb != b'f' {
            return;
        }
        let text = String::from_utf8_lossy(&self.params);
        let mut parts = text.split(';');
        let row: usize = parts
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1)
            .max(1);
        let col: usize = parts
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1)
            .max(1);
        self.cy = (row - 1).min(ROWS - 1);
        self.cx = (col - 1).min(COLS - 1);
    }

    fn put(&mut self, ch: char) {
        if self.cy < ROWS && self.cx < COLS {
            self.cells[self.cy][self.cx] = ch;
            self.cx += 1;
        }
    }

    fn frame(&self) -> String {
        self.cells
            .iter()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── the TUI driver ──────────────────────────────────────────────────────────

struct Tui {
    child: Option<Child>,
    fd: RawFd,
    buf: Vec<u8>,
    screen: Screen,
}

impl Drop for Tui {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Tui {
    fn boot() -> Tui {
        let (master, slave) = open_pty();
        // The child's stdin must be the pty slave so that `isatty(0)` is true
        // (crossterm's raw-mode `tty_fd()` picks fd 0 only then; otherwise it
        // reopens /dev/tty, which is not our pty).
        let stdin = dup(slave.as_raw_fd());
        let stdout = dup(slave.as_raw_fd());
        let stderr = dup(slave.as_raw_fd());
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lbt"));
        cmd.arg("tui")
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        // Deterministic test data + a UTF-8-capable, cursor-addressing term.
        cmd.env("LEON_BOOT_ENTRIES", ENTRIES);
        cmd.env("LEON_FB_WIDTH", "1280");
        cmd.env("LEON_FB_HEIGHT", "1024");
        cmd.env("LEON_FB_STRIDE", "5120");
        cmd.env("LEON_FB_FORMAT", "Bgrx");
        cmd.env("TERM", "xterm-256color");
        cmd.env("LANG", "C.UTF-8");
        cmd.env("LC_ALL", "C.UTF-8");
        cmd.env("HOME", std::env::temp_dir().join("lbt-tui-test-home"));
        let child = cmd.spawn().expect("spawn lbt tui");
        drop(slave); // keep the pty alive only via the child's stdio

        let fd = master.into_raw_fd();
        set_nonblocking(fd);
        let mut tui = Tui {
            child: Some(child),
            fd,
            buf: Vec::new(),
            screen: Screen::new(),
        };
        tui.wait_boot();
        tui
    }

    fn drain(&mut self, timeout: Duration) {
        if let Some(d) = read_some(self.fd, timeout) {
            self.buf.extend_from_slice(&d);
            self.screen.feed(&d);
        }
    }

    fn wait_boot(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            self.drain(Duration::from_millis(400));
            if self.screen.frame().contains(BOOT_MARKER) {
                self.drain(Duration::from_millis(500));
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let tail = &self.buf[self.buf.len().saturating_sub(400)..];
        panic!(
            "TUI never rendered '{BOOT_MARKER}' (buffer {} bytes, tail: {:?})",
            self.buf.len(),
            String::from_utf8_lossy(tail)
        );
    }

    fn send(&mut self, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            let n = unsafe {
                libc::write(
                    self.fd,
                    bytes[i..].as_ptr() as *const libc::c_void,
                    bytes.len() - i,
                )
            };
            if n > 0 {
                i += n as usize;
            } else if n < 0 {
                let err = io::Error::last_os_error();
                match err.raw_os_error() {
                    Some(libc::EAGAIN) | Some(libc::EINTR) => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    _ => panic!("pty write: {err}"),
                }
            } else {
                panic!("pty write returned 0");
            }
        }
    }

    fn wait_quiet(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut quiet = 0u32;
        while Instant::now() < deadline {
            let before = self.buf.len();
            self.drain(Duration::from_millis(200));
            if self.buf.len() == before {
                quiet += 1;
                if quiet >= 2 {
                    return;
                }
            } else {
                quiet = 0;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn key(&mut self, bytes: &[u8]) {
        self.send(bytes);
        self.wait_quiet();
    }

    fn frame(&self) -> String {
        self.screen.frame()
    }

    fn sel(&self) -> Option<u32> {
        sel_label(&self.frame())
    }

    fn preview(&self, tag: &str) {
        let lines: Vec<String> = self
            .screen
            .cells
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        println!("── {tag} ──");
        for l in lines.iter().take(14) {
            let cut: String = l.chars().take(COLS).collect();
            println!("  {cut}");
        }
        let status = lines
            .last()
            .map(|l| l.trim().to_string())
            .unwrap_or_default();
        println!("── status: {status}");
    }

    fn quit(mut self) -> ExitStatus {
        self.send(b"q");
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut child = match self.child.take() {
            Some(c) => c,
            None => return ExitStatus::default(),
        };
        loop {
            if let Some(st) = child.try_wait().unwrap() {
                return st;
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        child.wait().unwrap()
    }
}

// ── frame parsing ───────────────────────────────────────────────────────────

/// Reads the selected entry's displayed number (1-based) from the `▶` marker.
/// The row format is ` {marker} {number} `, e.g. ` ▶  3 `.
fn sel_label(frame: &str) -> Option<u32> {
    let bytes = frame.as_bytes();
    let marker = "\u{25b6}".as_bytes(); // "▶" (U+25B6)
    let mut i = 0;
    while i + marker.len() <= bytes.len() {
        if &bytes[i..i + marker.len()] == marker {
            let mut j = i + marker.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                return frame[start..j].parse().ok();
            }
        }
        i += 1;
    }
    None
}

/// Parses " N/M entries" (the header line) as `(visible, total)`.
fn visible_total(frame: &str) -> Option<(u32, u32)> {
    let bytes = frame.as_bytes();
    let needle = b"entries";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let mut j = i;
            while j > 0 && bytes[j - 1].is_ascii_whitespace() {
                j -= 1;
            }
            let total_end = j;
            while j > 0 && bytes[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j < total_end && j > 0 && bytes[j - 1] == b'/' {
                let mid = j - 1;
                let mut start = mid;
                while start > 0 && bytes[start - 1].is_ascii_digit() {
                    start -= 1;
                }
                if start < mid {
                    let visible: u32 = frame[start..mid].parse().ok()?;
                    let total: u32 = frame[j..total_end].parse().ok()?;
                    return Some((visible, total));
                }
            }
        }
        i += 1;
    }
    None
}

fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}

macro_rules! check {
    ($failures:expr, $tag:expr, $cond:expr, $detail:expr) => {{
        let ok = $cond;
        println!(
            "{}  {}  {}",
            if ok { "PASS" } else { "FAIL" },
            $tag,
            $detail
        );
        if !ok {
            $failures.push($tag.to_string());
        }
    }};
}

// ── tests ───────────────────────────────────────────────────────────────────

#[test]
fn boot_and_quit() {
    let tui = Tui::boot();
    println!(
        "\n━━━ LEON Boot Manager — preview ({}x{}) ━━━\n",
        COLS, ROWS
    );
    tui.preview("boot");
    let st = tui.quit();
    println!("\nquit rc: {st}");
    assert!(st.success(), "expected clean quit, got {st}");
}

#[test]
fn full_keymap() {
    let mut tui = Tui::boot();
    let mut failures: Vec<String> = Vec::new();
    println!(
        "\n━━━ LEON Boot Manager — full keymap preview ({}x{}) ━━━\n",
        COLS, ROWS
    );

    tui.preview("boot");
    let (visible, total) = visible_total(&tui.frame()).unwrap_or((6, 6));
    check!(
        failures,
        "initial-label-1",
        tui.sel() == Some(1),
        format!("label={:?}", tui.sel())
    );

    tui.key(b"j");
    check!(
        failures,
        "down-j",
        tui.sel() == Some(2),
        format!("label={:?}", tui.sel())
    );
    tui.preview("j");
    tui.key(b"k");
    check!(
        failures,
        "up-k",
        tui.sel() == Some(1),
        format!("label={:?}", tui.sel())
    );

    tui.key(b"\x1b[6~"); // PgDn
    check!(
        failures,
        "pgdn-moves",
        tui.sel().is_some() && tui.sel() != Some(1),
        format!("label={:?}", tui.sel())
    );
    tui.key(b"\x1b[5~"); // PgUp
    check!(
        failures,
        "pgup-back-to-1",
        tui.sel() == Some(1),
        format!("label={:?}", tui.sel())
    );

    tui.key(b"\x1b[H"); // Home
    check!(
        failures,
        "home->1",
        tui.sel() == Some(1),
        format!("label={:?}", tui.sel())
    );
    tui.key(b"\x1b[F"); // End
    check!(
        failures,
        "end->last",
        tui.sel() == Some(total),
        format!("label={:?}, total={total}", tui.sel())
    );
    if total >= 3 {
        tui.key(b"3");
        check!(
            failures,
            "num-3",
            tui.sel() == Some(3),
            format!("label={:?}", tui.sel())
        );
    }
    tui.key(b"1");
    check!(
        failures,
        "num-1",
        tui.sel() == Some(1),
        format!("label={:?}", tui.sel())
    );

    tui.key(b"\x1b[B"); // down arrow
    check!(
        failures,
        "down-arrow",
        tui.sel() == Some(2),
        format!("label={:?}", tui.sel())
    );
    tui.key(b"\x1b[A"); // up arrow
    check!(
        failures,
        "up-arrow",
        tui.sel() == Some(1),
        format!("label={:?}", tui.sel())
    );

    // search / filter
    tui.key(b"/");
    let f = tui.frame();
    check!(
        failures,
        "search-focus",
        f.contains(" / "),
        format!("…{}", tail(&f, 90))
    );
    tui.key(b"efi");
    let f = tui.frame();
    check!(
        failures,
        "search-typing",
        f.contains("/ efi"),
        format!("…{}", tail(&f, 90))
    );
    tui.key(b"\r");
    let f = tui.frame();
    check!(
        failures,
        "search-commit-filter",
        f.contains("filter:") && !f.contains("/ efi"),
        format!("…{}", tail(&f, 90))
    );
    check!(
        failures,
        "search-reduces-visible",
        visible_total(&f) == Some((2, 6)),
        format!("…{}", tail(&f, 90))
    );

    // n / N move within the (label-only) filtered list
    let before = tui.sel();
    tui.key(b"n");
    let after_n = tui.sel();
    if visible == total {
        check!(
            failures,
            "search-n-cycles",
            after_n.is_some() && after_n != before,
            format!("before={before:?} after={after_n:?}")
        );
    } else {
        check!(
            failures,
            "search-n-valid",
            after_n.is_some() && after_n <= Some(total),
            format!("before={before:?} after={after_n:?} (visible={visible})")
        );
    }
    tui.key(b"N");
    let after_shift_n = tui.sel();
    if visible == total {
        check!(
            failures,
            "search-N-cycles",
            after_shift_n.is_some() && after_shift_n != after_n,
            format!("before={after_n:?} after={after_shift_n:?}")
        );
    } else {
        check!(
            failures,
            "search-N-valid",
            after_shift_n.is_some() && after_shift_n <= Some(total),
            format!("before={after_n:?} after={after_shift_n:?} (visible={visible})")
        );
    }

    tui.key(b"\x1b"); // Esc clears the committed filter
    let f = tui.frame();
    check!(
        failures,
        "esc-clears-filter",
        !f.contains("filter:"),
        format!("…{}", tail(&f, 90))
    );

    // help (toggle on, toggle off)
    tui.key(b"h");
    let f = tui.frame();
    check!(
        failures,
        "help-shows-keymap",
        f.contains("PgUp / PgDn") && f.contains("move selection"),
        format!("…{}", tail(&f, 100))
    );
    tui.preview("h (help)");
    tui.key(b"h");
    let f = tui.frame();
    check!(
        failures,
        "help-closes",
        !f.contains("PgUp / PgDn"),
        format!("…{}", tail(&f, 90))
    );

    // detail panel toggle (starts off)
    tui.key(b"d");
    let f = tui.frame();
    check!(
        failures,
        "details-on",
        f.contains("path:") && f.contains("source:"),
        format!("…{}", tail(&f, 100))
    );
    tui.preview("d (details on)");
    tui.key(b"d");
    let f = tui.frame();
    check!(
        failures,
        "details-off",
        !f.contains("path:"),
        format!("…{}", tail(&f, 90))
    );

    // boot preview
    tui.key(b" ");
    let f = tui.frame();
    check!(
        failures,
        "space-boot-preview",
        f.contains("Boot preview"),
        format!("…{}", tail(&f, 100))
    );
    tui.preview("space (boot preview)");
    tui.key(b" ");
    let f = tui.frame();
    check!(
        failures,
        "preview-closed",
        !f.contains("Boot preview"),
        format!("…{}", tail(&f, 90))
    );

    // refresh
    tui.key(b"r");
    let f = tui.frame();
    check!(
        failures,
        "refresh-status",
        f.contains("refreshed"),
        format!("…{}", tail(&f, 100))
    );
    tui.preview("r (refresh)");

    // quit
    let st = tui.quit();
    check!(
        failures,
        "clean-exit-rc0",
        st.success(),
        format!("rc={st:?}")
    );

    if failures.is_empty() {
        println!("\nALL PASS");
    } else {
        println!("\nFAILURES: {failures:?}");
        assert!(failures.is_empty(), "TUI keymap failures: {failures:?}");
    }
}
