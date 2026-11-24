//! The PC's keyboard controller (i8042): the keyboard port.
//!
//! This driver is planned to be split into its i8042 and keyboard drivers in the near future. for now
//! its one.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::interrupt_controller::Irq;
use crate::driver_traits::keyboard::{self, KeyCode, KeyEvent, Keyboard};
use crate::services::device::Device;
use crate::services::interrupts::{self, IrqError, IrqHandle, IrqStatus};
use crate::services::queue::{EventQueue, QueueStats};
use crate::services::sync::IrqSpinLock;
use crate::x86_64::port::{inb, outb};

const DEFAULT_DATA_PORT: u16 = 0x60;
const DEFAULT_COMMAND_PORT: u16 = 0x64;

/// Status register bits.
const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;
const STATUS_ERROR: u8 = (1 << 6) | (1 << 7); // timeout, parity

/// Controller commands.
const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_DISABLE_KEYBOARD: u8 = 0xAD;
const CMD_ENABLE_KEYBOARD: u8 = 0xAE;

/// Configuration byte bits.
const CONFIG_KEYBOARD_IRQ: u8 = 1 << 0;
const CONFIG_KEYBOARD_CLOCK_OFF: u8 = 1 << 4;
const CONFIG_TRANSLATION: u8 = 1 << 6;

/// Keyboard command: start sending scancodes.
const KEYBOARD_ENABLE_SCANNING: u8 = 0xF4;

/// Status polls before giving up on the controller
const POLL_LIMIT: u32 = 50_000;
/// Most bytes one drain reads, so a stuck status bit cannot hang an interrupt.
const DRAIN_LIMIT: usize = 64;
/// Events kept for a reader that is slow to come.
const EVENT_QUEUE_SIZE: usize = 64;

/// Where a multi-byte scancode sequence is.
#[derive(Clone, Copy)]
enum Prefix {
    None,
    /// After `0xE0`: the next byte names an extended key.
    Extended,
    /// Inside the Pause key's sequence: this many bytes still to ignore.
    Pause(u8),
}

struct I8042 {
    data: u16,
    command: u16,
    irq_driven: AtomicBool,
    prefix: IrqSpinLock<Prefix>,
    events: EventQueue<KeyEvent, EVENT_QUEUE_SIZE>,
}

impl I8042 {
    fn new(data: u16, command: u16) -> Self {
        Self { data, command, irq_driven: AtomicBool::new(false), prefix: IrqSpinLock::new(Prefix::None), events: EventQueue::new(KeyEvent::NONE) }
    }

    fn status(&self) -> u8 {
        // SAFETY: the port is the controller's, as the firmware described it.
        unsafe { inb(self.command) }
    }

    fn read_data(&self) -> u8 {
        // SAFETY: as above.
        unsafe { inb(self.data) }
    }

    fn wait_input_clear(&self) -> Result<(), ProbeError> {
        for _ in 0..POLL_LIMIT {
            if self.status() & STATUS_INPUT_FULL == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(ProbeError::NotPresent)
    }

    fn write_command(&self, command: u8) -> Result<(), ProbeError> {
        self.wait_input_clear()?;
        // SAFETY: as above.
        unsafe { outb(self.command, command) };
        Ok(())
    }

    fn write_data(&self, byte: u8) -> Result<(), ProbeError> {
        self.wait_input_clear()?;
        // SAFETY: as above.
        unsafe { outb(self.data, byte) };
        Ok(())
    }

    fn read_response(&self) -> Result<u8, ProbeError> {
        for _ in 0..POLL_LIMIT {
            if self.status() & STATUS_OUTPUT_FULL != 0 {
                return Ok(self.read_data());
            }
            core::hint::spin_loop();
        }
        Err(ProbeError::NotPresent)
    }

    /// Throws away whatever the controller has buffered.
    fn flush(&self) {
        for _ in 0..DRAIN_LIMIT {
            if self.status() & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            self.read_data();
        }
    }

    /// Puts the controller in the state the driver decodes: keyboard port on,
    /// translation to set 1, and its interrupt on if `interrupts`.
    fn configure(&self, interrupts: bool) -> Result<(), ProbeError> {
        self.write_command(CMD_DISABLE_KEYBOARD)?;
        self.flush();

        self.write_command(CMD_READ_CONFIG)?;
        let mut config = self.read_response()?;
        config |= CONFIG_TRANSLATION;
        config &= !CONFIG_KEYBOARD_CLOCK_OFF;
        if interrupts {
            config |= CONFIG_KEYBOARD_IRQ;
        } else {
            config &= !CONFIG_KEYBOARD_IRQ;
        }
        self.write_command(CMD_WRITE_CONFIG)?;
        self.write_data(config)?;

        self.write_command(CMD_ENABLE_KEYBOARD)?;

        // Firmware normally has the keyboard scanning already; make sure. The
        // acknowledgement is not worth failing over.
        if self.write_data(KEYBOARD_ENABLE_SCANNING).is_ok() {
            let _ = self.read_response();
        }
        Ok(())
    }

    /// Reads every byte the controller has and decodes the keyboard's. Returns
    /// how many bytes there were.
    fn drain(&self) -> usize {
        let mut count = 0;
        for _ in 0..DRAIN_LIMIT {
            let status = self.status();
            if status & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            let byte = self.read_data();
            count += 1;
            // The mouse shares the controller, and a byte with a bad parity or
            // timeout flag is garbage.
            if status & (STATUS_AUX_DATA | STATUS_ERROR) != 0 {
                continue;
            }
            if let Some(event) = self.decode(byte) {
                self.events.push(event);
            }
        }
        count
    }

    /// Feeds one scancode set 1 byte to the decoder.
    fn decode(&self, byte: u8) -> Option<KeyEvent> {
        let mut prefix = self.prefix.lock();
        let code = match *prefix {
            Prefix::Pause(remaining) => {
                *prefix = if remaining > 1 { Prefix::Pause(remaining - 1) } else { Prefix::None };
                return None;
            }
            Prefix::Extended => {
                *prefix = Prefix::None;
                extended(byte & 0x7F)?
            }
            Prefix::None => match byte {
                0xE0 => {
                    *prefix = Prefix::Extended;
                    return None;
                }
                // Pause sends `E1 1D 45` then the same again with the release bit set.
                0xE1 => {
                    *prefix = Prefix::Pause(2);
                    return None;
                }
                // Set 1's main block matches Linux's key codes. Past 0x58 are
                // keys nobody types on, and the controller's own replies (acknowledge
                // 0xFA, resend 0xFE, errors 0x00 and 0xFF).
                _ => match byte & 0x7F {
                    0 | 0x59..=0x7F => return None,
                    code => KeyCode(u16::from(code)),
                },
            },
        };
        Some(KeyEvent { code, pressed: byte & 0x80 == 0 })
    }
}

/// The keys behind an `0xE0` prefix. `E0 2A` and `E0 36` are the "fake shifts"
/// some keyboards wrap around Print Screen and the arrows, and are dropped.
fn extended(code: u8) -> Option<KeyCode> {
    Some(match code {
        0x1C => KeyCode::KP_ENTER,
        0x1D => KeyCode::RIGHT_CTRL,
        0x35 => KeyCode::KP_SLASH,
        0x38 => KeyCode::RIGHT_ALT,
        0x47 => KeyCode::HOME,
        0x48 => KeyCode::UP,
        0x49 => KeyCode::PAGE_UP,
        0x4B => KeyCode::LEFT,
        0x4D => KeyCode::RIGHT,
        0x4F => KeyCode::END,
        0x50 => KeyCode::DOWN,
        0x51 => KeyCode::PAGE_DOWN,
        0x52 => KeyCode::INSERT,
        0x53 => KeyCode::DELETE,
        0x5B => KeyCode::LEFT_META,
        0x5C => KeyCode::RIGHT_META,
        _ => return None,
    })
}

impl I8042 {
    /// Whether there is no interrupt, so nothing is read unless someone asks.
    fn needs_polling(&self) -> bool {
        !self.irq_driven.load(Ordering::Acquire)
    }
}

impl Keyboard for I8042 {
    fn read_event(&self) -> KeyEvent {
        self.events.wait_pop(self.needs_polling(), || {
            self.drain();
        })
    }

    fn try_read_event(&self) -> Option<KeyEvent> {
        if self.needs_polling() {
            self.drain();
        }
        self.events.pop()
    }

    fn stats(&self) -> QueueStats {
        self.events.stats()
    }
}

fn on_irq(_irq: Irq, ctx: *mut ()) -> IrqStatus {
    // SAFETY: `ctx` is the leaked `I8042` that `probe` passed to `request_irq`.
    let keyboard = unsafe { &*ctx.cast::<I8042>() };
    if keyboard.drain() > 0 { IrqStatus::Handled } else { IrqStatus::NotMine }
}

/// Hooks IRQ 1 to `keyboard`.
fn hook_irq(dev: &Device, keyboard: &'static I8042) -> Result<IrqHandle, ProbeError> {
    // ACPI's legacy IRQ descriptors name ISA lines. What an ISA line is
    // actually wired to (after the MADT's overrides) is the APIC's to say.
    let line = dev
        .irq(0)
        .ok()
        .and_then(|spec| u8::try_from(spec.irq.number()).ok())
        .filter(|line| *line < 16)
        .unwrap_or(1);
    let wiring = interrupts::isa_irq(line).ok_or(ProbeError::NotPresent)?;

    let ctx = core::ptr::from_ref(keyboard).cast_mut().cast::<()>();
    interrupts::request_irq(wiring.irq, wiring.trigger, wiring.polarity, on_irq, ctx).map_err(|error| match error {
        IrqError::NoController => ProbeError::Defer,
        _ => ProbeError::InitFailed,
    })
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    // The interrupt service comes up once a controller has bound; wait for it
    // rather than settle for polling.
    if !interrupts::is_ready() {
        return Err(ProbeError::Defer);
    }

    let data = dev.io(0).map_or(DEFAULT_DATA_PORT, |ports| ports.base());
    let command = dev.io(1).map_or(DEFAULT_COMMAND_PORT, |ports| ports.base());

    let controller = I8042::new(data, command);
    // A floating bus reads as all ones.
    if controller.status() == 0xFF {
        return Err(ProbeError::NotPresent);
    }
    let keyboard: &'static I8042 = Box::leak(Box::new(controller));

    // No usable interrupt is not fatal: the keyboard is polled instead.
    let handle = match hook_irq(dev, keyboard) {
        Ok(handle) => Some(handle),
        Err(ProbeError::Defer) => return Err(ProbeError::Defer),
        Err(_) => None,
    };

    if let Err(error) = keyboard.configure(handle.is_some()) {
        if let Some(handle) = handle {
            let _ = interrupts::unregister(handle);
        }
        return Err(error);
    }
    keyboard.irq_driven.store(handle.is_some(), Ordering::Release);

    keyboard::register_static(dev, keyboard);
    Ok(())
}

// The standard PC keyboard ids; `PNP0303` is the one nearly every machine uses.
driver! {
    name: "i8042-keyboard",
    matches: [
        Match::Acpi("PNP0303"),
        Match::Acpi("PNP030B"),
        Match::Acpi("PNP0301"),
        Match::Acpi("PNP0300"),
    ],
    probe: probe,
}
