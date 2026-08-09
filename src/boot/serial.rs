//! Serial (UART) input for the boot manager.
//!
//! The boot-manager menu is mirrored onto the platform's debug UART by the
//! firmware's `ConOut` console (OVMF does this automatically; on real
//! hardware it depends on how the platform routes its consoles). That mirror
//! is one half of a terminal session. This module is the other half: it opens
//! the first `SerialIo` device and decodes the raw bytes a terminal sends
//! back — arrow-key escape sequences, Enter, Esc, digits — into the same
//! [`MenuKey`]s the UEFI keyboard console produces.
//!
//! The result is a menuconfig-style boot manager: `make qemu`, minicom,
//! picocom or screen on the debug port drives the real boot menu from the
//! keyboard, exactly like ncurses.
//!
//! Everything here is best-effort. No serial device, a driver that rejects a
//! short read timeout, or bytes we do not recognize simply mean the UEFI
//! keyboard console keeps working; the boot never depends on the UART.

use alloc::vec::Vec;

use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol};
use uefi::proto::console::serial::Serial;

/// Poll timeout in 100 ns units (the UEFI `SerialIo` unit). The menu loop
/// ticks at 100 ms; 100 ms keeps the countdown smooth while still
/// capturing whole multi-byte escape sequences. Some firmware (OVMF) clamps
/// the minimum timeout, so we use 100 ms rather than a smaller value.
const POLL_TIMEOUT_UNITS: u32 = 1_000_000;

const ESC: u8 = 0x1b;

/// A logical key the boot-manager menu understands, produced by either the
/// UEFI keyboard console or the terminal bytes on the serial console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    /// Disarm the auto-boot countdown.
    Esc,
    /// Delete the last character (UEFI Backspace, or BS/DEL on a terminal).
    Backspace,
    /// A text character, for the config value editors.
    Printable(char),
}

/// The UART console, when present. Reads are configured with a short timeout
/// so the menu loop can poll it without blocking its 100 ms tick.
pub struct Console {
    serial: ScopedProtocol<Serial>,
    decoder: KeyDecoder,
}

impl Console {
    /// Opens the first serial I/O device on the platform.
    ///
    /// Returns `None` when there is no serial device, or when its driver
    /// refuses the short read timeout that polling needs.
    pub fn open() -> Option<Self> {
        let handle = boot::get_handle_for_protocol::<Serial>().ok()?;
        // Non-exclusive: OVMF's ConOut console splitter also holds the
        // SerialIo protocol; opening with Exclusive revokes its mirror.
        let params = OpenProtocolParams {
            handle,
            agent: boot::image_handle(),
            controller: None,
        };
        let mut serial = unsafe {
            boot::open_protocol::<Serial>(params, OpenProtocolAttributes::GetProtocol).ok()?
        };
        // Best-effort: try to set a short read timeout so the menu loop can
        // poll without blocking its 100 ms tick. If the driver rejects the
        // change we fall back to the default (1 s) — slower but functional.
        let mut mode = *serial.io_mode();
        mode.timeout = POLL_TIMEOUT_UNITS;
        let _ = serial.set_attributes(&mode);
        Some(Self {
            serial,
            decoder: KeyDecoder::new(),
        })
    }

    /// Reads and decodes whatever bytes the terminal has sent since the last
    /// poll, returning the decoded menu keys (usually at most one).
    pub fn poll(&mut self) -> Vec<MenuKey> {
        let mut bytes = Vec::with_capacity(16);
        let mut buf = [0u8; 16];
        while let Ok(()) = self.serial.read(&mut buf) {
            bytes.extend_from_slice(&buf);
        }
        self.decoder.feed(&bytes, !bytes.is_empty())
    }
}

/// Turns raw terminal bytes into [`MenuKey`]s.
struct KeyDecoder {
    buf: Vec<u8>,
}

impl KeyDecoder {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feeds a chunk of bytes read from the UART and decodes as many complete
    /// keys as possible. A trailing `ESC` is held back while `more` is true
    /// (it may be the first byte of an arrow-key sequence); once no more
    /// bytes arrive it is resolved as the Esc key.
    fn feed(&mut self, bytes: &[u8], more: bool) -> Vec<MenuKey> {
        self.buf.extend_from_slice(bytes);
        let mut keys = Vec::new();
        while let Some(&b0) = self.buf.first() {
            match b0 {
                ESC => {
                    if self.buf.len() < 2 {
                        if more {
                            break;
                        }
                        self.key(&mut keys, MenuKey::Esc, 1);
                        continue;
                    }
                    match self.buf[1] {
                        // CSI: ESC [ <final>  (arrows/Home/End as H/F, keys 1~/4~/5~/6~)
                        b'[' => {
                            let Some(final_byte) = self.buf.get(2).copied() else {
                                break;
                            };
                            match final_byte {
                                b'A' => self.key(&mut keys, MenuKey::Up, 3),
                                b'B' => self.key(&mut keys, MenuKey::Down, 3),
                                b'H' => self.key(&mut keys, MenuKey::Home, 3),
                                b'F' => self.key(&mut keys, MenuKey::End, 3),
                                b'C' | b'D' => self.skip(3), // Right/Left: unused
                                b'1' | b'3' | b'4' | b'5' | b'6' => {
                                    if self.buf.get(3) != Some(&b'~') {
                                        break; // incomplete tilde sequence
                                    }
                                    match final_byte {
                                        b'1' => self.key(&mut keys, MenuKey::Home, 4),
                                        b'4' => self.key(&mut keys, MenuKey::End, 4),
                                        b'5' => self.key(&mut keys, MenuKey::PageUp, 4),
                                        b'6' => self.key(&mut keys, MenuKey::PageDown, 4),
                                        _ => self.skip(4), // Delete: unused
                                    }
                                }
                                _ => self.skip(2), // unknown CSI: drop "ESC ["
                            }
                        }
                        // SS3: ESC O <final> (what some terminals send for arrows)
                        b'O' => {
                            let Some(final_byte) = self.buf.get(2).copied() else {
                                break;
                            };
                            match final_byte {
                                b'A' => self.key(&mut keys, MenuKey::Up, 3),
                                b'B' => self.key(&mut keys, MenuKey::Down, 3),
                                b'C' | b'D' => self.skip(3),
                                _ => self.skip(2),
                            }
                        }
                        // ESC followed by any other byte (Alt+key): treat as Esc.
                        _ => self.key(&mut keys, MenuKey::Esc, 1),
                    }
                }
                b'\r' | b'\n' => self.key(&mut keys, MenuKey::Enter, 1),
                b'\x08' | b'\x7f' => self.key(&mut keys, MenuKey::Backspace, 1),
                // Vim-style navigation keys keep working on serial even when
                // the terminal has no arrow keys.
                b'j' | b'J' => self.key(&mut keys, MenuKey::Up, 1),
                b'k' | b'K' => self.key(&mut keys, MenuKey::Down, 1),
                // Anything else printable is passed through for the value
                // editors (digits, letters, punctuation).
                b' '..=b'~' => self.key(&mut keys, MenuKey::Printable(b0 as char), 1),
                _ => self.skip(1), // unprintable or unbound
            }
        }
        keys
    }

    fn key(&mut self, keys: &mut Vec<MenuKey>, key: MenuKey, n: usize) {
        keys.push(key);
        self.skip(n);
    }

    fn skip(&mut self, n: usize) {
        let n = n.min(self.buf.len());
        if n > 0 {
            self.buf.drain(..n);
        }
    }
}
