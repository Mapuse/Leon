//! Raw serial console for use after `ExitBootServices`.
//!
//! The stub has no firmware services past the handoff, so this talks to the
//! UART directly: COM1 on x86_64 (PC UART 16550) and the PL011 on aarch64
//! (the standard `virt` machine UART at `0x0900_0000`). It is deliberately
//! tiny — byte out with a ready-check, plus a `fmt::Write` shim so the kernel
//! can log status with `format_args!`-style formatting.

use core::fmt;

#[cfg(target_arch = "x86_64")]
mod inner {
    /// COM1 base port.
    const COM1: u16 = 0x3F8;
    /// Line Status Register: bit 5 (THRE) set means the transmitter is ready.
    const LSR: u16 = COM1 + 5;

    /// Writes one byte to COM1, waiting for the transmitter to drain first.
    pub fn putc(c: u8) {
        unsafe {
            loop {
                let mut status: u8;
                core::arch::asm!(
                    "in al, dx",
                    out("al") status,
                    in("dx") LSR,
                    options(nomem, nostack)
                );
                if status & 0x20 != 0 {
                    break;
                }
            }
            core::arch::asm!(
                "out dx, al",
                in("dx") COM1,
                in("al") c,
                options(nomem, nostack)
            );
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod inner {
    /// PL011 base on the QEMU `virt` machine.
    const PL011: *mut u32 = 0x0900_0000 as *mut u32;

    /// Writes one byte to the PL011, waiting for the TX FIFO to drain first.
    pub fn putc(c: u8) {
        unsafe {
            let fr = PL011.add(0x18 / 4);
            while core::ptr::read_volatile(fr) & (1 << 5) != 0 {}
            core::ptr::write_volatile(PL011, c as u32);
        }
    }
}

/// A `core::fmt::Write` target backed by the raw serial port.
pub struct Serial;

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            inner::putc(b);
        }
        Ok(())
    }
}

/// Formats a message to the serial console, with a trailing newline.
pub fn log(args: fmt::Arguments<'_>) {
    let _ = fmt::write(&mut Serial, args);
    inner::putc(b'\n');
}
