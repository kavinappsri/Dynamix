//! virtio-input: QEMU's `-device virtio-keyboard-device`.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};

use super::{CONFIG, QUEUE_SIZE, Queue, Transport};
use crate::driver;
use crate::driver_traits::driver::{Match, ProbeError};
use crate::driver_traits::interrupt_controller::Irq;
use crate::driver_traits::keyboard::{self, KeyCode, KeyEvent, Keyboard};
use crate::services::device::Device;
use crate::services::interrupts::{self, IrqStatus};
use crate::services::provider;
use crate::services::queue::{EventQueue, QueueStats};

const EVENT_SIZE: usize = 8;
/// Events kept for a reader that is slow to come.
const EVENT_QUEUE_SIZE: usize = 64;
const EV_KEY: u16 = 1;

/// Config space: ask which codes of an event type the device can send.
const CONFIG_SELECT: usize = CONFIG;
const CONFIG_SUBSELECT: usize = CONFIG + 1;
const CONFIG_SIZE: usize = CONFIG + 2;
const CONFIG_BITMAP: usize = CONFIG + 8;
const CONFIG_EV_BITS: u8 = 0x11;
const KEY_A: usize = 30;

struct VirtioKeyboard {
    transport: Transport,
    queue: Queue,
    /// Address of the buffers the device writes its events into.
    buffers: usize,
    irq_driven: AtomicBool,
    events: EventQueue<KeyEvent, EVENT_QUEUE_SIZE>,
}

impl VirtioKeyboard {
    /// Whether there is no interrupt, so nothing is read unless someone asks.
    fn needs_polling(&self) -> bool {
        !self.irq_driven.load(Ordering::Acquire)
    }

    fn poll(&self) {
        self.transport.ack_interrupt();
        self.service();
    }

    /// Pushes every event the device has delivered and gives the buffers back.
    fn service(&self) {
        let mut recycled = false;
        while let Some((id, _len)) = self.queue.pop_used() {
            let at = self.buffers + usize::from(id) * EVENT_SIZE;
            // SAFETY: `at` is one of our event buffers, which the device has
            // just finished writing.
            let (kind, code, value) = unsafe {
                (
                    (at as *const u16).read_volatile(),
                    ((at + 2) as *const u16).read_volatile(),
                    ((at + 4) as *const u32).read_volatile(),
                )
            };
            if kind == EV_KEY {
                self.events.push(KeyEvent { code: KeyCode(code), pressed: value != 0 });
            }
            self.queue.submit(id);
            recycled = true;
        }
        if recycled {
            self.queue.notify(&self.transport);
        }
    }
}

impl Keyboard for VirtioKeyboard {
    fn read_event(&self) -> KeyEvent {
        self.events.wait_pop(self.needs_polling(), || self.poll())
    }

    fn try_read_event(&self) -> Option<KeyEvent> {
        if self.needs_polling() {
            self.poll();
        }
        self.events.pop()
    }

    fn stats(&self) -> QueueStats {
        self.events.stats()
    }
}

fn on_irq(_irq: Irq, ctx: *mut ()) -> IrqStatus {
    // SAFETY: `ctx` is the leaked `VirtioKeyboard` that `probe` registered.
    let keyboard = unsafe { &*ctx.cast::<VirtioKeyboard>() };
    if keyboard.transport.ack_interrupt() == 0 {
        return IrqStatus::NotMine;
    }
    keyboard.service();
    IrqStatus::Handled
}

/// Whether the device can send the letter keys.
fn is_keyboard(transport: &Transport) -> bool {
    let regs = transport.regs();
    regs.write8(CONFIG_SELECT, CONFIG_EV_BITS);
    regs.write8(CONFIG_SUBSELECT, EV_KEY as u8);
    let size = usize::from(regs.read8(CONFIG_SIZE));
    let byte = KEY_A / 8;
    size > byte && regs.read8(CONFIG_BITMAP + byte) & (1 << (KEY_A % 8)) != 0
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    if !interrupts::is_ready() {
        return Err(ProbeError::Defer);
    }
    let transport: Transport = *provider::get::<Transport>(dev.parent().ok_or(ProbeError::Firmware)?)?;

    transport.begin()?;
    if !is_keyboard(&transport) {
        transport.reset();
        return Err(ProbeError::NotPresent);
    }
    let queue = Queue::new(&transport, 0).inspect_err(|_| transport.fail())?;

    let buffers = queue.scratch();
    for id in 0..QUEUE_SIZE {
        queue.set_device_buffer(id, buffers + usize::from(id) * EVENT_SIZE, EVENT_SIZE as u32);
        queue.submit(id);
    }

    let keyboard: &'static VirtioKeyboard =
        Box::leak(Box::new(VirtioKeyboard {
            transport,
            queue,
            buffers,
            irq_driven: AtomicBool::new(false),
            events: EventQueue::new(KeyEvent::NONE),
        }));

    // No usable interrupt is not fatal: the keyboard is polled instead.
    let ctx = core::ptr::from_ref(keyboard).cast_mut().cast::<()>();
    if interrupts::request(dev, 0, on_irq, ctx).is_ok() {
        keyboard.irq_driven.store(true, Ordering::Release);
    }

    transport.finish();
    keyboard.queue.notify(&transport);

    keyboard::register_static(dev, keyboard);
    Ok(())
}

driver! {
    name: "virtio-input",
    matches: [Match::Spawned("virtio,input")],
    probe: probe,
}
