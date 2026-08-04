//! Full end-to-end keymap test + preview for the boot-manager TUI.
//!
//! Boots the real `lbt tui` (the embedded `leon_menu.py`) in a pty, drives
//! every keybinding, asserts the menu responds, and prints every frame.
//!
//! Requires the `python` feature and a Linux host. Run from `lbt/`:
//!
//! ```text
//! cargo test -p lbt --features python --test tui_full -- --nocapture
//! ```
//!
//! On this machine (musl target + a pyo3 config for the static CPython) use:
//!
//! ```text
//! PYO3_CONFIG_FILE=/tmp/opencode/pyo3-musl-config.toml \
//! CARGO_TARGET_DIR=/home/m/Leon/target \
//! cargo test --locked --target x86_64-unknown-linux-musl -p lbt \
//!   --features python --test tui_full -- --nocapture
//! ```
//!
//! `--nocapture` is what makes this a preview: every frame of the TUI is
//! printed as the keys are driven. The embedded CPython needs its stdlib;
//! this is auto-detected at `$XDG_CACHE_HOME/leon/python-build` (override
//! with `LEON_PYTHON_LIB`).

#![cfg(all(feature = "python", target_os = "linux"))]

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const COLS: u16 = 120;
const ROWS: u16 = 40;
const BOOT_MARKER: &str = "LEON Boot Manager";

// ── pty plumbing ────────────────────────────────────────────────────────────

fn dup(fd: RawFd) -> OwnedFd {
    let d = unsafe { libc::dup(fd) };
    assert!(d >= 0, "dup failed: {}", io::Error::last_os_error());
    unsafe { OwnedFd::from_raw_fd(d) }
}

fn open_pty() -> (OwnedFd, OwnedFd) {
    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    assert!(master >= 0, "posix_openpt: {}", io::Error::last_os_error());
    assert_eq!(unsafe { libc::grantpt(master) }, 0, "grantpt: {}", io::Error::last_os_error());
    assert_eq!(unsafe { libc::unlockpt(master) }, 0, "unlockpt: {}", io::Error::last_os_error());

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
    assert_eq!(unsafe { libc::tcsetattr(slave, libc::TCSANOW, &t) }, 0,
        "tcsetattr: {}", io::Error::last_os_error());

    let win = libc::winsize { ws_row: ROWS, ws_col: COLS, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe { libc::ioctl(slave, libc::TIOCSWINSZ as _, &win) };

    (unsafe { OwnedFd::from_raw_fd(master) }, unsafe { OwnedFd::from_raw_fd(slave) })
}

fn set_nonblocking(fd: RawFd) {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(flags >= 0, "F_GETFL: {}", io::Error::last_os_error());
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0,
        "F_SETFL: {}", io::Error::last_os_error());
}

fn read_some(fd: RawFd, timeout: Duration) -> Option<Vec<u8>> {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
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

fn python_lib() -> Option<String> {
    if let Ok(p) = std::env::var("LEON_PYTHON_LIB")
        && !p.is_empty()
    {
        return Some(p);
    }
    let cache = std::env::var("XDG_CACHE_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.cache")))
        .unwrap_or_default();
    let base = format!("{cache}/leon/python-build");
    let lib = format!("{base}/Lib");
    let plat = format!("{base}/build/lib.linux-x86_64-3.14");
    if Path::new(&lib).is_dir() {
        Some(format!("{lib}:{plat}"))
    } else {
        None
    }
}

// ── the TUI driver ──────────────────────────────────────────────────────────

struct Tui {
    child: Option<Child>,
    fd: RawFd,
    buf: Vec<u8>,
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
        let stdin = dup(slave.as_raw_fd());
        let stdout = dup(slave.as_raw_fd());
        let stderr = dup(slave.as_raw_fd());
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lbt"));
        cmd.arg("tui")
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        // The TUI repaints on an idle timer by default ("always displaying");
        // disable it so the harness's quiet-window detection stays deterministic.
        cmd.env("LEON_TUI_IDLE_MS", "0");
        if let Some(py) = python_lib() {
            cmd.env("PYTHONPATH", py);
        }
        let child = cmd.spawn().expect("spawn lbt tui");
        drop(slave); // keep the pty alive only via the child's stdio

        let fd = master.into_raw_fd();
        set_nonblocking(fd);
        let mut tui = Tui { child: Some(child), fd, buf: Vec::new() };
        tui.wait_boot();
        tui
    }

    fn drain(&mut self, timeout: Duration) {
        if let Some(d) = read_some(self.fd, timeout) {
            self.buf.extend_from_slice(&d);
        }
    }

    fn wait_boot(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            self.drain(Duration::from_millis(400));
            if self.buf.windows(BOOT_MARKER.len()).any(|w| w == BOOT_MARKER.as_bytes()) {
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
                libc::write(self.fd, bytes[i..].as_ptr() as *const libc::c_void, bytes.len() - i)
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
        latest_frame(&self.buf)
    }

    fn sel(&self) -> Option<u32> {
        sel_label(&self.frame())
    }

    fn preview(&self, tag: &str) {
        let f = self.frame();
        let lines: Vec<&str> = f.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
        println!("── {tag} ──");
        for l in lines.iter().take(14) {
            let cut: String = l.chars().take(COLS as usize).collect();
            println!("  {cut}");
        }
        let status = lines.last().map(|l| l.trim().to_string()).unwrap_or_default();
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

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn latest_frame(buf: &[u8]) -> String {
    let text = String::from_utf8_lossy(buf);
    // split on the CSI "cursor home" that starts every repaint, then strip colors
    let last = text.rsplit("\u{1b}[H").next().unwrap_or("");
    strip_ansi(last)
}

fn sel_label(frame: &str) -> Option<u32> {
    let bytes = frame.as_bytes();
    let marker = "\u{25b6} ".as_bytes(); // "▶ " (U+25B6)
    let mut i = 0;
    while i + marker.len() <= bytes.len() {
        if &bytes[i..i + marker.len()] == marker {
            let mut j = i + marker.len();
            let start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b'.' {
                return frame[start..j].parse().ok();
            }
        }
        i += 1;
    }
    None
}

/// Parses " N/M entries" (the header sub-line) as `(visible, total)`.
fn visible_total(frame: &str) -> Option<(u32, u32)> {
    let bytes = frame.as_bytes();
    let needle = b"entries";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let mut j = i;
            while j > 0 && bytes[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j < i && j > 0 && bytes[j - 1] == b'/' {
                let mut k = j - 1;
                while k > 0 && bytes[k - 1].is_ascii_digit() {
                    k -= 1;
                }
                if k < j - 1 {
                    let visible: u32 = frame[k..j - 1].parse().ok()?;
                    let total: u32 = frame[j..i].parse().ok()?;
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
        println!("{}  {}  {}", if ok { "PASS" } else { "FAIL" }, $tag, $detail);
        if !ok {
            $failures.push($tag.to_string());
        }
    }};
}

// ── tests ───────────────────────────────────────────────────────────────────

#[test]
fn boot_and_quit() {
    let tui = Tui::boot();
    println!("\n━━━ LEON Boot Manager — preview ({}x{}) ━━━\n", COLS, ROWS);
    tui.preview("boot");
    let st = tui.quit();
    println!("\nquit rc: {st}");
    assert!(st.success(), "expected clean quit, got {st}");
}

#[test]
fn full_keymap() {
    let mut tui = Tui::boot();
    let mut failures: Vec<String> = Vec::new();
    println!("\n━━━ LEON Boot Manager — full keymap preview ({}x{}) ━━━\n", COLS, ROWS);

    tui.preview("boot");
    let (visible, total) = visible_total(&tui.frame()).unwrap_or((6, 6));
    check!(failures, "initial-label-1", tui.sel() == Some(1), format!("label={:?}", tui.sel()));

    tui.key(b"j");
    check!(failures, "down-j", tui.sel() == Some(2), format!("label={:?}", tui.sel()));
    tui.preview("j");
    tui.key(b"k");
    check!(failures, "up-k", tui.sel() == Some(1), format!("label={:?}", tui.sel()));

    tui.key(b"\x1b[6~"); // PgDn
    check!(failures, "pgdn-moves", tui.sel().is_some() && tui.sel() != Some(1),
        format!("label={:?}", tui.sel()));
    tui.key(b"\x1b[5~"); // PgUp
    check!(failures, "pgup-back-to-1", tui.sel() == Some(1), format!("label={:?}", tui.sel()));

    tui.key(b"\x1b[H"); // Home
    check!(failures, "home->1", tui.sel() == Some(1), format!("label={:?}", tui.sel()));
    tui.key(b"\x1b[F"); // End
    check!(failures, "end->last", tui.sel() == Some(total),
        format!("label={:?}, total={total}", tui.sel()));
    if total >= 3 {
        tui.key(b"3");
        check!(failures, "num-3", tui.sel() == Some(3), format!("label={:?}", tui.sel()));
    }
    tui.key(b"1");
    check!(failures, "num-1", tui.sel() == Some(1), format!("label={:?}", tui.sel()));

    tui.key(b"\x1b[B"); // down arrow
    check!(failures, "down-arrow", tui.sel() == Some(2), format!("label={:?}", tui.sel()));
    tui.key(b"\x1b[A"); // up arrow
    check!(failures, "up-arrow", tui.sel() == Some(1), format!("label={:?}", tui.sel()));

    // search / filter
    tui.key(b"/");
    let f = tui.frame();
    check!(failures, "search-focus", f.contains("enter: filter"),
        format!("…{}", tail(&f, 90)));
    tui.key(b"efi");
    let f = tui.frame();
    check!(failures, "search-typing", f.contains("/ efi"), format!("…{}", tail(&f, 90)));
    tui.key(b"\r");
    let f = tui.frame();
    check!(failures, "search-commit-filter", f.contains("filter: /efi"),
        format!("…{}", tail(&f, 90)));

    // n / N cycle matches (only asserted to move when every entry matches)
    let before = tui.sel();
    tui.key(b"n");
    let after_n = tui.sel();
    if visible == total {
        check!(failures, "search-n-cycles", after_n.is_some() && after_n != before,
            format!("before={before:?} after={after_n:?}"));
    } else {
        check!(failures, "search-n-valid", after_n.is_some() && after_n <= Some(total),
            format!("before={before:?} after={after_n:?} (visible={visible})"));
    }
    tui.key(b"N");
    let after_shift_n = tui.sel();
    if visible == total {
        check!(failures, "search-N-cycles", after_shift_n.is_some() && after_shift_n != after_n,
            format!("before={after_n:?} after={after_shift_n:?}"));
    } else {
        check!(failures, "search-N-valid", after_shift_n.is_some() && after_shift_n <= Some(total),
            format!("before={after_n:?} after={after_shift_n:?} (visible={visible})"));
    }

    tui.key(b"\x1b"); // Esc clears the committed filter
    let f = tui.frame();
    check!(failures, "esc-clears-filter", !f.contains("filter: /efi"),
        format!("…{}", tail(&f, 90)));

    // help
    tui.key(b"h");
    let f = tui.frame();
    check!(failures, "help-shows-keymap", f.contains("j/k") && f.contains("PgUp/PgDn"),
        format!("…{}", tail(&f, 100)));
    tui.preview("h (help)");
    tui.key(b"\x1b"); // Esc closes help
    let f = tui.frame();
    check!(failures, "esc-closes-help", !f.contains("PgUp/PgDn"),
        format!("…{}", tail(&f, 90)));

    // detail panel toggle
    tui.key(b"d");
    let f = tui.frame();
    check!(failures, "details-off", !f.contains("path:"), format!("…{}", tail(&f, 90)));
    tui.key(b"d");
    let f = tui.frame();
    check!(failures, "details-on", f.contains("path:") && f.contains("vendor:"),
        format!("…{}", tail(&f, 100)));

    // boot preview
    tui.key(b" ");
    let f = tui.frame();
    check!(failures, "space-boot-preview", f.contains("WOULD BOOT"),
        format!("…{}", tail(&f, 100)));
    tui.preview("space (boot preview)");

    // refresh
    tui.key(b"r");
    let f = tui.frame();
    check!(failures, "refresh-status", f.contains("refreshed"),
        format!("…{}", tail(&f, 100)));
    tui.preview("r (refresh)");

    // quit
    let st = tui.quit();
    check!(failures, "clean-exit-rc0", st.success(), format!("rc={st:?}"));

    if failures.is_empty() {
        println!("\nALL PASS");
    } else {
        println!("\nFAILURES: {failures:?}");
        assert!(failures.is_empty(), "TUI keymap failures: {failures:?}");
    }
}
