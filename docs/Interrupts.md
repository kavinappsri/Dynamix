# How Interrupts Work

I'll skip the small talk. This file explains how the interrupt system works, and how
you'd use it from a driver. If that's not what you're looking for, then you're probably in the wrong place

Keep in mind that this file may be outdated, So your best information source should be the source code. For the record,
when I'm typing this, its November 11, 2026.


## The things

There's are a few things you have to know to use interrupts, mainly
* Know which interrupt your device uses (an `Irq`)
* Write a handler function 
* Register it, and turn it on

Let's start with the first one.

## Irq numbers

An `Irq` is just a number

```rust
use crate::driver_traits::interrupt_controller::Irq;

static IRQ = Irq(33);
```

What the number means depends on the machine. On ARM, it's the id the GIC gives the interrupt (0-15 are software
interrupts, 16-31 are per-core ones like the timer, 32 and up are normal devices). On x86_64 it's the GSI
(global system interrupt) number, which is whatever the I/O APIC pin the device is wired to is called.
You can get those numbers like so:

- **FDT:** the `interrupts` property of your device's node. For the GIC it's 3 numbers: type (0 = normal device,
  1 = per-core), the interrupt number, and some flags. The id the GIC uses is `number + 32` for type 0, and
  `number + 16` for type 1.
- **x86_64 legacy (ISA) devices:** use `interrupts::isa_irq(line)`. Firmware is allowed to rewire these (the PIT's
  IRQ0 usually ends up on GSI 2), so never assume line N is GSI N. It returns the `Irq` *and* how it's signaled
- **x86_64 PCI devices:** from the PCI config space / ACPI. And I am so sorry, but we don't have PCI yet. So umm ... 
  you're in the dark for now 😅

## The handler

A handler is a plain function that takes the `Irq` that fired, and a pointer you gave it when you registered. It
returns whether the interrupt was yours

```rust
use crate::services::interrupts::{Irq, IrqStatus};

fn my_uart_handler(_irq: Irq, ctx: *mut ()) -> IrqStatus {
    // SAFETY: we registered a pointer to a `MySerialDriver` that lives forever.
    let uart = unsafe { &*(ctx as *const MySerialDriver) };

    if !uart.has_pending_byte() {
        return IrqStatus::NotMine;
    }
    let byte = uart.read_byte();
    // ... put it somewhere ...
    IrqStatus::Handled
}
```

The `*mut ()` is there so you can hand the handler whatever you want (in a driver, usually a pointer to the driver
itself). The handler just gets the pointer back untouched, so it's up to you to make sure it stays alive for as long as
the handler is registered.

`IrqStatus::NotMine` is for shared lines. If you're the only device on your line, just always return `IrqStatus::Handled`.

A few general rules of thumb to keep in mind when writing a handler, 

- Try to be quick. Read the data, clear the interrupt, get out.
- Don't block or wait on anything.
- Don't allocate. The heap _is_ interrupt safe, but allocating in a handler is probably not a great idea.
- Only take `IrqSpinLock`s (more on that below).
- Don't call `unregister` from a handler on its own line, it'll wait for itself forever.

## Registering

The easy way is `request`. It sets how the line is signaled, registers your handler, and turns the line on

```rust
use crate::services::interrupts::{self, Irq, Polarity, Trigger, IrqError};

pub fn my_random_fn_so_that_i_dont_get_bombed_by_syntax_errors() -> Result<(),IrqError> {
    let handle = interrupts::request(
        irq,
        Trigger::Level,
        Polarity::ActiveHigh,
        my_uart_handler,
        uart as *const MySerialDriver as *mut (),
    )?;
}
```

`Trigger` and `Polarity` describe how your device signals. Look at your device's docs (or the FDT flags / ACPI
tables). Level-triggered means the line stays asserted until you clear it in the device, edge-triggered means it gives
a single pulse.

A few things to know

- The GIC can only do active-high lines, `configure` returns `IrqError::Unsupported` for active-low on ARM.
- Software interrupts (see below) can't be configured. Use `register` and `enable` for those instead of `request`.
- `request` returns an `IrqHandle`. You might want to hold on to that if you're ever going to need to unregister your
  handler

If you need more control, `request` is just these three calls in a row

```rust
pub fn my_random_fn_so_that_i_dont_get_bombed_by_syntax_errors() -> Result<(),IrqError> {
    interrupts::configure(irq, Trigger::Level, Polarity::ActiveHigh)?;
    let handle = interrupts::register(irq, my_uart_handler, ctx)?;
    interrupts::enable(irq)?;
}
```

`register` alone doesn't turn the line on, so you can set everything up and only enable it when you're ready.
`disable(irq)` masks it again.

To remove a handler

```rust
pub fn my_random_fn_so_that_i_dont_get_bombed_by_syntax_errors() -> Result<(),IrqError> {
    interrupts::unregister(handle)?;
}
```

When that returns, your handler isn't running on any core and will never be called again. So it's now safe to free
whatever the context pointer was pointing to.

## Locking

A normal `SpinLock` doesn't turn interrupts off. If the code you interrupted was holding a lock,
and your handler tries to take the same lock, this will deadlock (groundbreaking, right?).

So there's 2 locks

- `SpinLock`: doesn't touch interrupts.
- `IrqSpinLock`: turns interrupts off on this core while it's held, and puts them back the way they were after.

The PMM, the heap and the MMU all use `IrqSpinLock` already, which is why you can safely allocate frames with
interrupts on. If you just need interrupts off for a moment without a lock

```rust
use crate::services::cpu;

pub fn my_random_fn_so_that_i_dont_get_bombed_by_syntax_errors() {
    let state = cpu::irq_save();   // turns them off, remembers what they were
    // ... critical section ...
    cpu::irq_restore(state); // puts them back the way they were
} 
```

Using `irq_save`/`irq_restore` instead of `irq_disable`/`irq_enable` means it still works if it's nested inside
another critical section.

## Turning it all on

`interrupts::init(hardware)` finds the interrupt controller and sets it up. The kernel calls it once during boot,
after the MMU is up. You don't call it from a driver, so you're probably never going to have to worry about it.

It does **not** unmask interrupts on the CPU. Until somebody calls `cpu::irq_enable()`, nothing will actually reach
your handler, even if the controller is ready. Right now the kernel leaves them off, except for the duration of the
self-test.

## Software interrupts

Every controller can also trigger an interrupt on purpose, on the core that asks for it (SGIs on the GIC, a
self-IPI on x86_64).

```rust
pub fn my_random_fn_so_that_i_dont_get_bombed_by_syntax_errors() -> Result<(),IrqError> {
    let irq = interrupts::software_irq(0).expect("no software interrupts");
    let handle = interrupts::register(irq, my_handler, ctx)?;
    interrupts::enable(irq)?;
    cpu::irq_enable();
    interrupts::raise_self(irq)?; // my_handler runs right about now
}
```

That's more or less exactly what `kernel/src/irq_test.rs` does. If you ever think you broke interrupts, flip
`DEV_IRQ_SELFTEST` in `main.rs` and look at what it says.

## Something went wrong :(

- `IrqError::NoController`: `init` didn't find a controller. On ARM, there's no GIC driver for that device tree (this currently
  only supports GICv2). On x86_64, there's no MADT, or no I/O APIC in it.
- `interrupts::unhandled_count()`: how many interrupts fired on a line with no handler.
- `interrupts::spurious_count()`: how many times the controller said "nothing is pending" when the CPU got an
  interrupt.

## Writing a controller driver

If you're adding support for a new interrupt controller, you implement the `InterruptController` trait instead of
`Serial`. It's the same 3 steps as the driver doc, `init` function and everything, except you register it with
the `InterruptController` kind

```rust
register_driver!(MY_GIC, "my,gic-compatible-string", init, InterruptController);
```

The trait is small. It can enable/disable/configure a line, acknowledge the interrupt that's being delivered, and
signal the end of it. Read `GicV2` to see a full example. Controllers that don't show up in a device tree, like the
APIC on x86_64, are set up directly by `interrupts::init` instead.

### TODO for this system

These are just notes for myself, you don't need to look at them

- No timer interrupts. 
- Every interrupt is sent to the boot core. `set_affinity` works on the GIC, not on x86_64.
- No MSI / MSI-X, after PCIe.
- GICv3 isn't supported, only GICv2.

That's about it. You're gonna have to look at the code for the rest, here are some links to help

[Interrupt service](/hal/src/services/interrupts.rs)

[InterruptController](/hal/src/driver_traits/interrupt_controller.rs)

[IrqSpinLock](/hal/src/services/sync.rs)

[CPU interrupt control](/hal/src/services/cpu.rs)

[GICv2 driver](/hal/src/drivers/arm/gic_v2.rs)

[x86_64 APIC](/hal/src/x86_64/x64_apic.rs)

[Self-test](/kernel/src/selftest/irq_test.rs)