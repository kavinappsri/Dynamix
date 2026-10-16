//! Minimal polled COM1 writer for fault reporting.
//!
//! This is NOT part of the driver subsystem. Can be deleted once driver subsystem works

use core::fmt;

use crate::x86_64::port::{inb, outb};

const COM1: u16 = 0x3F8;
const LINE_STATUS_TX_EMPTY: u8 = 0x20;
/// Upper bound on status polls per byte, so a machine without a COM1 cannot hang the fault path.
const POLL_LIMIT: u32 = 100_000;

/// Programs COM1 for 115200 8N1 with FIFOs, interrupts off.
pub fn init() {
    // SAFETY: legacy COM1 ports; nothing else in the kernel owns them yet.
    unsafe {
        outb(COM1 + 1, 0x00); // interrupts off
        outb(COM1 + 3, 0x80); // DLAB on
        outb(COM1, 0x01); // divisor 1 -> 115200 baud
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8 bits, no parity, one stop bit
        outb(COM1 + 2, 0xC7); // enable and clear FIFOs
        outb(COM1 + 4, 0x0B); // DTR, RTS, OUT2
    }
}

fn write_byte(byte: u8) {
    // SAFETY: as in `init`.
    unsafe {
        for _ in 0..POLL_LIMIT {
            if inb(COM1 + 5) & LINE_STATUS_TX_EMPTY != 0 {
                break;
            }
        }
        outb(COM1, byte);
    }
}

/// `core::fmt` sink over COM1. Newlines are sent as CR LF.
pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}
