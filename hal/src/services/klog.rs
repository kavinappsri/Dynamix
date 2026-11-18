//! Dynamix log
//!
//! # Record Format
//!
//! ```text
//! offset  size  field
//!      0     2  text length
//!      2     1  source  (see [`Source`])
//!      3     1  level   (see [`Level`])
//!      4     1  record version (currently 1)
//!      5     3  zero
//!      8     8  sequence number
//!     16     8  timestamp, nanoseconds since the clock started
//! ```
//!
//! All numbers are little-endian.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::driver_traits::serial::active_serial;
use crate::services::sync::{IrqSpinLock, IrqSpinLockGuard};
use crate::services::timer;

/// Bytes of storage for records.
pub const CAPACITY: usize = 64 * 1024;

/// Longest line in one record..
pub const LINE_CAPACITY: usize = 256;

const HEADER_LEN: usize = 24;
const RECORD_VERSION: u8 = 1;

/// Who wrote a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Source {
    /// The kernel itself.
    Kernel = 0,
    /// ACPICA's output.
    Acpi = 1,
    /// The embedded controller driver.
    Ec = 2,
}

impl Source {
    const COUNT: usize = 3;

    fn from_raw(raw: u8) -> Source {
        match raw {
            1 => Source::Acpi,
            2 => Source::Ec,
            _ => Source::Kernel,
        }
    }

    /// Short name, as `dmesg` prints it.
    pub fn name(self) -> &'static str {
        match self {
            Source::Kernel => "kernel",
            Source::Acpi => "acpi",
            Source::Ec => "ec",
        }
    }
}

/// How much a record matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    fn from_raw(raw: u8) -> Level {
        match raw {
            0 => Level::Error,
            1 => Level::Warn,
            3 => Level::Debug,
            _ => Level::Info,
        }
    }
}

/// One record, copied out of the ring. The text went into the caller's buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub seq: u64,
    pub time_ns: u64,
    pub source: Source,
    pub level: Level,
    /// Bytes of text copied (cut to the buffer's size if the record is longer).
    pub len: usize,
}

/// What [`read_from`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    /// The record asked for, or the oldest one still there. `None` when
    /// there is nothing at or after the sequence number.
    pub record: Option<Record>,
    /// What to ask for next.
    pub next_seq: u64,
    /// How many records between the asked-for sequence number and the first
    /// one returned had already been overwritten.
    pub lost: u64,
}

/// A snapshot of the ring's bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub capacity: usize,
    /// Bytes in use.
    pub used: usize,
    /// Sequence number of the oldest record still stored.
    pub first_seq: u64,
    /// Sequence number the next record will get.
    pub next_seq: u64,
    /// Records overwritten since boot.
    pub dropped: u64,
}

/// A line being put together from fragments.
struct Line {
    len: usize,
    level: Level,
    text: [u8; LINE_CAPACITY],
}

impl Line {
    const fn new() -> Self {
        Self { len: 0, level: Level::Info, text: [0; LINE_CAPACITY] }
    }
}

struct Log<const N: usize> {
    data: [u8; N],
    /// Positions count bytes ever written, not offsets: the byte at position
    /// `p` lives at `data[p % CAPACITY]`. `head` is the oldest record's start
    /// and `end` is one past the newest.
    head: u64,
    end: u64,
    first_seq: u64,
    next_seq: u64,
    dropped: u64,
    /// Where the last reader found a record, so reading in order does not
    /// re-walk the ring from the start each time.
    hint_seq: u64,
    hint_pos: u64,
    lines: [Line; Source::COUNT],
}

impl<const N: usize> Log<N> {
    const fn new() -> Self {
        Self {
            data: [0; N],
            head: 0,
            end: 0,
            first_seq: 0,
            next_seq: 0,
            dropped: 0,
            hint_seq: 0,
            hint_pos: 0,
            lines: [Line::new(), Line::new(), Line::new()],
        }
    }

    fn at(position: u64) -> usize {
        (position % N as u64) as usize
    }

    fn used(&self) -> usize {
        (self.end - self.head) as usize
    }

    fn copy_in(&mut self, position: u64, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.data[Self::at(position + offset as u64)] = *byte;
        }
    }

    fn copy_out(&self, position: u64, bytes: &mut [u8]) {
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = self.data[Self::at(position + offset as u64)];
        }
    }

    fn read_u64(&self, position: u64) -> u64 {
        let mut bytes = [0u8; 8];
        self.copy_out(position, &mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn text_len(&self, position: u64) -> usize {
        let mut bytes = [0u8; 2];
        self.copy_out(position, &mut bytes);
        usize::from(u16::from_le_bytes(bytes))
    }

    /// Drops the oldest record to make room. The one place records get lost,
    /// and so the place a sink would take them first.
    fn evict_oldest(&mut self) -> bool {
        if self.head == self.end {
            return false;
        }
        self.head += (HEADER_LEN + self.text_len(self.head)) as u64;
        self.first_seq += 1;
        self.dropped += 1;
        true
    }

    fn append(&mut self, source: Source, level: Level, text: &[u8]) {
        let size = HEADER_LEN + text.len();
        while N - self.used() < size {
            if !self.evict_oldest() {
                return;
            }
        }

        let mut header = [0u8; HEADER_LEN];
        header[0..2].copy_from_slice(&(text.len() as u16).to_le_bytes());
        header[2] = source as u8;
        header[3] = level as u8;
        header[4] = RECORD_VERSION;
        header[8..16].copy_from_slice(&self.next_seq.to_le_bytes());
        header[16..24].copy_from_slice(&timer::now_ns().to_le_bytes());

        let start = self.end;
        self.copy_in(start, &header);
        self.copy_in(start + HEADER_LEN as u64, text);
        self.end += size as u64;
        self.next_seq += 1;
    }

    /// Turns the source's pending line into a record.
    fn commit(&mut self, source: Source) {
        let index = source as usize;
        let len = self.lines[index].len;
        if len == 0 {
            return;
        }
        let level = self.lines[index].level;
        let mut text = [0u8; LINE_CAPACITY];
        text[..len].copy_from_slice(&self.lines[index].text[..len]);
        self.lines[index].len = 0;

        self.append(source, level, &text[..len]);
        echo(source, &text[..len]);
    }

    fn push(&mut self, source: Source, level: Level, bytes: &[u8]) {
        let index = source as usize;
        for &byte in bytes {
            match byte {
                b'\n' => self.commit(source),
                b'\r' => {}
                _ => {
                    if self.lines[index].len == 0 {
                        self.lines[index].level = level;
                    }
                    if self.lines[index].len == LINE_CAPACITY {
                        self.commit(source);
                        self.lines[index].level = level;
                    }
                    let at = self.lines[index].len;
                    self.lines[index].text[at] = byte;
                    self.lines[index].len = at + 1;
                }
            }
        }
    }

    fn stats(&self) -> Stats {
        Stats {
            capacity: N,
            used: self.used(),
            first_seq: self.first_seq,
            next_seq: self.next_seq,
            dropped: self.dropped,
        }
    }

    fn read_from(&mut self, seq: u64, text: &mut [u8]) -> Read {
        let mut wanted = seq;
        let mut lost = 0;
        if wanted < self.first_seq {
            lost = self.first_seq - wanted;
            wanted = self.first_seq;
        }
        if wanted >= self.next_seq {
            return Read { record: None, next_seq: self.next_seq.max(seq), lost };
        }

        // Start from where the last read ended if that is still in the ring.
        let (mut at_seq, mut position) = if self.hint_seq >= self.first_seq && self.hint_seq <= wanted {
            (self.hint_seq, self.hint_pos)
        } else {
            (self.first_seq, self.head)
        };
        while at_seq < wanted {
            position += (HEADER_LEN + self.text_len(position)) as u64;
            at_seq += 1;
        }
        self.hint_seq = wanted;
        self.hint_pos = position;

        let full_len = self.text_len(position);
        let mut header = [0u8; HEADER_LEN];
        self.copy_out(position, &mut header);
        let copied = full_len.min(text.len());
        self.copy_out(position + HEADER_LEN as u64, &mut text[..copied]);

        Read {
            record: Some(Record {
                seq: self.read_u64(position + 8),
                time_ns: self.read_u64(position + 16),
                source: Source::from_raw(header[2]),
                level: Level::from_raw(header[3]),
                len: copied,
            }),
            next_seq: wanted + 1,
            lost,
        }
    }
}

static LOG: IrqSpinLock<Log<CAPACITY>> = IrqSpinLock::new(Log::new());

/// Which sources also print to the serial port as they log.
static ECHO: [AtomicBool; Source::COUNT] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];

/// Makes a source also print every finished line to the serial port (once
/// there is one). Off for all sources by default.
pub fn set_echo(source: Source, on: bool) {
    ECHO[source as usize].store(on, Ordering::Relaxed);
}

fn echo(source: Source, text: &[u8]) {
    if !ECHO[source as usize].load(Ordering::Relaxed) {
        return;
    }
    if let Some(uart) = active_serial() {
        for byte in source.name().bytes().chain(": ".bytes()).chain(text.iter().copied()).chain("\r\n".bytes()) {
            uart.write_byte(byte);
        }
    }
}

/// Appends a piece of a line. A record is created when a `\n` comes by (or
/// when the line reaches [`LINE_CAPACITY`]). Meant for sources that print in
/// fragments; for anything else use [`log`].
pub fn write_fragment(source: Source, level: Level, bytes: &[u8]) {
    LOG.lock().push(source, level, bytes);
}

/// Logs one line.
pub fn log(source: Source, level: Level, args: fmt::Arguments<'_>) {
    struct Fragments(Source, Level);

    impl fmt::Write for Fragments {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            write_fragment(self.0, self.1, text.as_bytes());
            Ok(())
        }
    }

    let _ = fmt::write(&mut Fragments(source, level), args);
    write_fragment(source, level, b"\n");
}

/// The ring's bookkeeping right now.
pub fn stats() -> Stats {
    LOG.lock().stats()
}

/// Copies out the record with sequence number `seq` (or the oldest one after
/// it, if `seq` was already overwritten). See the module docs.
pub fn read_from(seq: u64, text: &mut [u8]) -> Read {
    LOG.lock().read_from(seq, text)
}

/// Writes one record the way `dmesg` shows it:
/// `[   12.345678] acpi: text`.
fn print_record(out: &mut dyn fmt::Write, record: &Record, text: &[u8]) -> fmt::Result {
    let micros = record.time_ns / 1_000;
    write!(out, "[{:>5}.{:06}] {}: ", micros / 1_000_000, micros % 1_000_000, record.source.name())?;
    for &byte in text {
        // Firmware text is ASCII; anything else would only garble the console.
        out.write_char(if byte == b'\t' || (0x20..0x7F).contains(&byte) { byte as char } else { '?' })?;
    }
    out.write_char('\n')
}

/// What [`dump`] ended with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dump {
    /// Where to continue from next time.
    pub next_seq: u64,
    /// Records that were overwritten before they could be printed.
    pub lost: u64,
}

/// Prints every record from `from_seq` on, oldest first. Pass 0 for
/// everything still stored. The lock is not held while printing.
pub fn dump(out: &mut dyn fmt::Write, from_seq: u64) -> Dump {
    let mut text = [0u8; LINE_CAPACITY];
    let mut cursor = from_seq;
    let mut lost = 0;
    loop {
        let read = read_from(cursor, &mut text);
        lost += read.lost;
        cursor = read.next_seq;
        let Some(record) = read.record else { break };
        if print_record(out, &record, &text[..record.len]).is_err() {
            break;
        }
    }
    Dump { next_seq: cursor, lost }
}

/// Prints the last `count` records without ever waiting for the lock
pub fn dump_tail_nowait(out: &mut dyn fmt::Write, count: u64) {
    let mut text = [0u8; LINE_CAPACITY];
    let mut cursor = match LOG.try_lock() {
        Some(log) => log.next_seq.saturating_sub(count).max(log.first_seq),
        None => return,
    };
    loop {
        let read = {
            let Some(mut log): Option<IrqSpinLockGuard<'_, Log<CAPACITY>>> = LOG.try_lock() else { return };
            log.read_from(cursor, &mut text)
        };
        cursor = read.next_seq;
        let Some(record) = read.record else { break };
        if print_record(out, &record, &text[..record.len]).is_err() {
            break;
        }
    }
}
