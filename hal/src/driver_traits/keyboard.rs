//! Keyboards

use alloc::boxed::Box;

use crate::services::device::Device;
use crate::services::queue::QueueStats;
use crate::services::registry::{Class, Entry, Iter};

static KEYBOARDS: Class<dyn Keyboard> = Class::new();

/// A physical key, in Linux's numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyCode(pub u16);

impl KeyCode {
    pub const ESC: KeyCode = KeyCode(1);
    pub const BACKSPACE: KeyCode = KeyCode(14);
    pub const TAB: KeyCode = KeyCode(15);
    pub const ENTER: KeyCode = KeyCode(28);
    pub const LEFT_CTRL: KeyCode = KeyCode(29);
    pub const LEFT_SHIFT: KeyCode = KeyCode(42);
    pub const RIGHT_SHIFT: KeyCode = KeyCode(54);
    pub const KP_ASTERISK: KeyCode = KeyCode(55);
    pub const LEFT_ALT: KeyCode = KeyCode(56);
    pub const SPACE: KeyCode = KeyCode(57);
    pub const CAPS_LOCK: KeyCode = KeyCode(58);
    pub const NUM_LOCK: KeyCode = KeyCode(69);
    pub const KP_ENTER: KeyCode = KeyCode(96);
    pub const RIGHT_CTRL: KeyCode = KeyCode(97);
    pub const KP_SLASH: KeyCode = KeyCode(98);
    pub const RIGHT_ALT: KeyCode = KeyCode(100);
    pub const HOME: KeyCode = KeyCode(102);
    pub const UP: KeyCode = KeyCode(103);
    pub const PAGE_UP: KeyCode = KeyCode(104);
    pub const LEFT: KeyCode = KeyCode(105);
    pub const RIGHT: KeyCode = KeyCode(106);
    pub const END: KeyCode = KeyCode(107);
    pub const DOWN: KeyCode = KeyCode(108);
    pub const PAGE_DOWN: KeyCode = KeyCode(109);
    pub const INSERT: KeyCode = KeyCode(110);
    pub const DELETE: KeyCode = KeyCode(111);
    pub const LEFT_META: KeyCode = KeyCode(125);
    pub const RIGHT_META: KeyCode = KeyCode(126);
}

/// A key going down or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub pressed: bool,
}

impl KeyEvent {
    pub const NONE: KeyEvent = KeyEvent { code: KeyCode(0), pressed: false };
}

/// A keyboard.
pub trait Keyboard: Sync {
    /// Blocks until a key goes down or up.
    fn read_event(&self) -> KeyEvent;

    /// The next event if one is waiting, without blocking.
    fn try_read_event(&self) -> Option<KeyEvent>;

    /// Counters of this keyboard's event queue.
    fn stats(&self) -> QueueStats;
}

/// The keyboard the kernel uses: the first that was bound.
pub fn primary_keyboard() -> Option<&'static dyn Keyboard> {
    KEYBOARDS.primary()
}

/// Every keyboard that was bound, in the order they were bound.
pub fn keyboards() -> Iter<Entry<dyn Keyboard>> {
    KEYBOARDS.iter()
}

/// Called by a driver's probe to hand over the keyboard it started on `device`.
pub fn register(device: &'static Device, keyboard: impl Keyboard + 'static) -> &'static dyn Keyboard {
    let keyboard: &'static dyn Keyboard = Box::leak(Box::new(keyboard));
    KEYBOARDS.register(device, keyboard);
    keyboard
}

/// Like [`register`], for a keyboard that already lives in a leaked box
pub fn register_static(device: &'static Device, keyboard: &'static dyn Keyboard) {
    KEYBOARDS.register(device, keyboard);
}
