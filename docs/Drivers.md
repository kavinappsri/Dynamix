# The Driver System

This doc should be able to give you a brief overview of how the driver system generally works, and how you can write your
own drivers. Do keep in mind that this doc may be outdated, so your best resource should always be the source code. 

## How it works

When the driver system starts (`hal::services::device::init`), The first thing it does is enumerate all the devices present.
The exact way it does this depends on whether it's loaded with the ACPI tables or the FDT, but for either case, it constructs
a `Device` for each device and appends it to a global registry of items. 

The system then 'probes' i.e. starts matching the drivers and calling their `probe` function. the `probe` function essentially
initializes your driver and device. Once that is done, it does a few checks on what is the default device, then it hands 
control back to the kernel.

## Writing drivers

Now, we're going to learn by actually writing a driver. Let's say ... for a UART, which has a write offset as `0x0` and a 
read offset at `0x08`. 

```rust
use crate::services::mmio::Mmio;

pub struct MyUartDriver {
    regs: Mmio,
}

impl MyUartDriver {
    const WRITE: usize = 0x00;
    const READ: usize = 0x08;

    pub const fn new(regs: Mmio) -> Self {
        Self { regs }
    }
}
```

That was quick, wasn't it? What we basically have here is a struct for our driver. A driver must be a struct, or else 
it wouldn't be possible to create multiple instances of it. 

[`Mmio`](/hal/src/services/mmio.rs) is basically a struct for writing/reading values at an offset from a certain base. You
aren't required to use it, you could have had `base: usize` instead of `regs: Mmio`, its just that `Mmio` makes things easier.

Now, UART drivers can be used directly by the kernel, so we have to implement a trait for the kernel to communicate with
the driver. For a uart, this trait it [`Serial`](/hal/src/driver_traits/serial.rs)

```rust
use crate::services::mmio::Mmio;
use crate::driver_traits::serial::Serial;

pub struct MyUartDriver {
    regs: Mmio,
}

impl MyUartDriver {
    const WRITE: usize = 0x00;
    const READ: usize = 0x08;

    pub const fn new(regs: Mmio) -> Self {
        Self { regs }
    }
}

impl Serial for MyUartDriver {
    fn write_byte(&self, byte: u8) {
        self.regs.write32(Self::WRITE, u32::from(byte));
    }
    fn read_byte(&self) -> u8 {
        self.regs.read32(Self::READ) as u8
    }
}
```

Finally, we need to register our driver with the kernel, and the driver system. To do this, we define a `probe`, where we 
register our driver, and then pass the probe and our compatible matches to the kernel via the `driver!` macro.

```rust
use crate::services::mmio::Mmio;
use crate::services::device::Device;
use crate::driver_traits::serial::Serial;
use crate::driver_traits::driver::{Match, ProbeError};

pub struct MyUartDriver {
    regs: Mmio,
}

impl MyUartDriver {
    const WRITE: usize = 0x00;
    const READ: usize = 0x08;

    pub const fn new(regs: Mmio) -> Self {
        Self { regs }
    }
}

impl Serial for MyUartDriver {
    fn write_byte(&self, byte: u8) {
        self.regs.write32(Self::WRITE, u32::from(byte));
    }
    fn read_byte(&self) -> u8 {
        self.regs.read32(Self::READ) as u8
    }
}

fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    // The first register window of the device, mapped as device memory.
    let registers = dev.mem(0)?.mmio();
    serial::register(dev, MyUartDriver::new(registers));
    Ok(())
}

driver! {
    name: "my-uart",
    matches: [Match::Fdt("my,uart"), Match::Fdt("my,uart-v2")],
    probe: probe,
}
```

Here, `Match` is against what you are matching your string, i.e. ACPI device, FDT compatible, etc. `ProbeError` is what 
error, if any, your driver returns. There is a couple you can choose from, like `NotPresent` if your device isn't even there,
`Defer` if your waiting for a dependency driver (`Defer` will cause your probe to be run again after some time), `InitFailed`
for a failed device initialization, etc.

## A few special cases

### Provider drivers

If your writing a driver that will never be directly used by the kernel, only by other drivers (i.e. for some device
only on a specific family of SoC) you are writing a provider driver, you can just simply not implement any trait for your 
struct, register it with `provider::register(...)` in the probe instead.

If your using a provider driver, you have to first get the `Device`, then you have to do `provider::get()` on the device
to get your dependency driver (Note that `get()` returns a `Result<&'static T, ProbeError>`, and you are responsible
for handling that). You have a few different ways of getting the device

* device tree: `dev.fdt_link(&FdtLink::CLOCKS, "name")` follows `clocks` / `clock-names` (also
  `FdtLink::POWER_DOMAINS`, or `FdtLink::new(..)` for any other `<phandle cells...>` list);
* ACPI: `dev.acpi_dependency(i)` is the `i`th entry of the device's `_DEP`;
* a device spawned by another driver (below): `dev.parent()`.

```rust
use crate::services::device::Device;

pub fn probe(dev: &'static Device) -> Result<(), ProbeError> {
    // the clock controller's probe
    provider::register(dev, Rk3126Cru::new(regs));

    // the display driver's probe
    let cru_device = dev.fdt_link(&FdtLink::CLOCKS, "dclk_lcdc")?;   // which device? (firmware says)
    let cru: &Rk3126Cru = provider::get(cru_device)?;                // what does it offer? (by type)
    cru.enable(Rk3126Clock::DclkVop)?;
}
```

### My driver isn't listed!

More often than not, you have a device that isn't listed in FDT/ACPI. Now, for your driver to load, either your consumer
driver or provider driver, can announce a new device with `dev.spawn(name, &[ids], &[resources])`. The new device goes 
through the usual probing, so its driver is an ordinary `driver!` that matches `Match::Spawned("id")`. It can reach whoever
announced it with `dev.parent()`.

## And so now you know

Yay! You've figured out how the driver system works!

But have you? As I said, this file may very well be outdated, so you could have been learning the driver system from a hundred
years ago. There's only one way to find out. Off you go to the source files!

[Driver table and `driver!`](/hal/src/driver_traits/driver.rs)

[Device, resources and probing](/hal/src/services/device/mod.rs)

[Class registries](/hal/src/services/registry.rs)

[Device tree walker](/hal/src/services/dtb.rs)

[Mmio](/hal/src/services/mmio.rs)

[Port](/hal/src/x86_64/port.rs)

[Serial](/hal/src/driver_traits/serial.rs)

[Example Drivers](/hal/src/drivers/mod.rs)

