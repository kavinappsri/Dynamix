//! Keyboard input as a byte stream.
//!
//! The HAL's keyboard drivers deliver which physical key went up or down;
//! this is where that becomes text. [`KeyboardStream`] wraps one keyboard, tracks shift, ctrl and
//! caps lock, looks keys up in a US layout, and hands the shell one byte at a
//! time like a UART would: Enter is `\n`, Backspace `0x08`, Ctrl+letter the
//! matching control code. Keys without a byte (arrows, function keys, ...) are
//! swallowed for now.

use hal::driver_traits::keyboard::{KeyCode, KeyEvent, Keyboard};
use hal::services::sync::IrqSpinLock;

use crate::io::InputStream;

/// Highest key code the layout knows; the main block and keypad all sit below.
const KEYS: usize = 128;

/// Builds a code-to-byte table from `(first code, bytes)` rows, one byte per
/// consecutive key. Keys not listed produce nothing.
const fn table(rows: &[(usize, &[u8])]) -> [u8; KEYS] {
    let mut out = [0u8; KEYS];
    let mut row = 0;
    while row < rows.len() {
        let (start, bytes) = rows[row];
        let mut i = 0;
        while i < bytes.len() {
            out[start + i] = bytes[i];
            i += 1;
        }
        row += 1;
    }
    out
}

/// US layout, no modifiers. Keypad digits assume Num Lock is on.
static NORMAL: [u8; KEYS] = table(&[
    (2, b"1234567890-="),
    (14, b"\x08\tqwertyuiop[]\n"),
    (30, b"asdfghjkl;'`"),
    (43, b"\\zxcvbnm,./"),
    (55, b"*"),
    (57, b" "),
    (71, b"789-456+1230."),
    (86, b"<"),
    (96, b"\n"),
    (98, b"/"),
]);

/// US layout with Shift held.
static SHIFTED: [u8; KEYS] = table(&[
    (2, b"!@#$%^&*()_+"),
    (14, b"\x08\tQWERTYUIOP{}\n"),
    (30, b"ASDFGHJKL:\"~"),
    (43, b"|ZXCVBNM<>?"),
    (55, b"*"),
    (57, b" "),
    (71, b"789-456+1230."),
    (86, b">"),
    (96, b"\n"),
    (98, b"/"),
]);

struct State {
    /// One bit per key code below [`KEYS`]: the keys that are down.
    held: u128,
    caps_lock: bool,
}

impl State {
    fn is_down(&self, key: KeyCode) -> bool {
        self.held & (1u128 << key.0) != 0
    }
}

/// One HAL keyboard as an [`InputStream`], the way [`crate::io::SerialStream`]
/// is one serial port. Which keyboard to listen to is the caller's choice.
pub struct KeyboardStream<'a> {
    keyboard: &'a dyn Keyboard,
    state: IrqSpinLock<State>,
}

impl<'a> KeyboardStream<'a> {
    pub fn new(keyboard: &'a dyn Keyboard) -> Self {
        Self { keyboard, state: IrqSpinLock::new(State { held: 0, caps_lock: false }) }
    }

    /// The byte `event` types, if it types one.
    fn translate(&self, event: KeyEvent) -> Option<u8> {
        let code = usize::from(event.code.0);
        if code >= KEYS {
            return None;
        }

        let mut state = self.state.lock();
        let bit = 1u128 << code;
        let was_down = state.held & bit != 0;
        if !event.pressed {
            state.held &= !bit;
            return None;
        }
        state.held |= bit;

        // Auto-repeat re-sends the press; it must not flip the lock again.
        if event.code == KeyCode::CAPS_LOCK {
            if !was_down {
                state.caps_lock = !state.caps_lock;
            }
            return None;
        }

        let shift = state.is_down(KeyCode::LEFT_SHIFT) || state.is_down(KeyCode::RIGHT_SHIFT);
        let ctrl = state.is_down(KeyCode::LEFT_CTRL) || state.is_down(KeyCode::RIGHT_CTRL);
        let caps_lock = state.caps_lock;
        drop(state);

        let plain = NORMAL[code];
        // Caps lock only turns letters around; shift changes everything.
        let upper = if plain.is_ascii_lowercase() { shift != caps_lock } else { shift };
        let byte = if upper { SHIFTED[code] } else { plain };

        match byte {
            0 => None,
            letter if ctrl => letter.is_ascii_alphabetic().then_some(letter & 0x1F),
            byte => Some(byte),
        }
    }
}

impl InputStream for KeyboardStream<'_> {
    fn read_byte(&self) -> u8 {
        loop {
            if let Some(byte) = self.translate(self.keyboard.read_event()) {
                return byte;
            }
        }
    }
}
