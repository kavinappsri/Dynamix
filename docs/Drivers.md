# How To Write Drivers

There's no point in me beating around the bush if you're in this far. I'll just get straight to the point. 

Here, were just gonna write a simple Serial driver. hopefully that teaches you enough system so that you can look at the 
code and figure the rest out yourself. 

Keep in mind that this file may be outdated, So your best information source
should be the source code. For the record, when I'm typing this, its October 20, 2026. 

## The 3 things

There's mainly 3 things you have to do when writing a Serial driver

1. Implement the `Serial` trait
2. Define the `init` function
3. Call the `register_driver` macro

So lets start writing a driver. when you write one, you'd want to make it sit inside `hal/src/drivers`. Anywhere inside
that folder is fine, the only condition is it has to be accessible by the outside crate, so make sure you write your `mod.rs`
properly

Let's start by making a simple struct for our driver. Let's take our target device to be a simple memory mapped UART

```rust
pub struct MySerialDriver {
    base_register: usize,
}

impl MySerialDriver {
    pub fn new(base: usize) -> Self {
        Self {
            base_register: base,
        }
    }
}
```

The crate gives us a handy struct for memory mapped i/o, which we can use

```rust
use crate::services::mmio::Mmio;

pub struct MySerialDriver {
    base: Mmio,
}

impl MySerialDriver {
    pub unsafe fn new(base: usize) -> Self {
        Self {
            base: unsafe{ Mmio::new(base) },
        }
    }
}
```

Let's say our UART has a write line at offset `0x00` and a write offset at `0x08`

```rust
use crate::services::mmio::Mmio;

pub struct MySerialDriver {
    base: Mmio,
}

impl MySerialDriver {
    const WRITE: usize = 0x00;
    const READ: usize = 0x08;
    
    pub unsafe fn new(base: usize) -> Self {
        Self {
            base: unsafe{ Mmio::new(base) },
        }
    }
}
```
Now, we can implement the `Serial` Trait
```rust
use crate::services::mmio::Mmio;
use crate::{Serial};

pub struct MySerialDriver {
    base: Mmio,
}

impl MySerialDriver {
    const WRITE: usize = 0x00;
    const READ: usize = 0x08;
    
    pub unsafe fn new(base: usize) -> Self {
        Self {
            base: unsafe{ Mmio::new(base) },
        }
    }
}

impl Serial for MySerialDriver {
    fn write_byte(&self, byte: u8) {
        self.base.write32(Self::WRITE, u32::from(byte));
    }
    fn read_byte(&self) -> u8 {
        self.base.read32(Self::READ) as u8
    }
}
```
Now, we must create our init function. the purpose of this is to have a globally accessible place for our driver using 
a custom cell called a `StaticCell`. The function signature must take in `DeviceResource` 
and a `DeviceQuery` and return a `Result<'static dyn Serial, DriverProbeError>`

```rust
use crate::services::mmio::Mmio;
use crate::{Serial};
use crate::services::device_query::DeviceResource;
use crate::services::device_query::DeviceQuery;
use crate::driver_traits::driver::{Driver, DriverProbeError};
use crate::services::sync::StaticCell;

pub struct MySerialDriver {
    base: Mmio,
}

impl MySerialDriver {
    const WRITE: usize = 0x00;
    const READ: usize = 0x08;
    
    pub unsafe fn new(base: usize) -> Self {
        Self {
            base: unsafe{ Mmio::new(base) },
        }
    }
}

impl Serial for MySerialDriver {
    fn write_byte(&self, byte: u8) {
        self.base.write32(Self::WRITE, u32::from(byte));
    }
    fn read_byte(&self) -> u8 {
        self.base.read32(Self::READ) as u8
    }
}

fn init(resource: DeviceResource, _query: &dyn DeviceQuery) -> Result<&'static dyn Serial, DriverProbeError> {
    let base = resource.mmio_base().ok_or(DriverProbeError::WrongResource)?;
    static INSTANCE: StaticCell<MySerialDriver> = StaticCell::uninit();
    Ok(INSTANCE.get_or_init(|| {
        unsafe {MySerialDriver::new(base)}
    }))
}
```

Now, we simply call the macro

```rust
use crate::services::mmio::Mmio;
use crate::{Serial};
use crate::services::device_query::DeviceResource;
use crate::services::device_query::DeviceQuery;
use crate::driver_traits::driver::{Driver, DriverProbeError};
use crate::services::sync::StaticCell;
use crate::register_driver;

pub struct MySerialDriver {
    base: Mmio,
}

impl MySerialDriver {
    const WRITE: usize = 0x00;
    const READ: usize = 0x08;
    
    pub unsafe fn new(base: usize) -> Self {
        Self {
            base: unsafe{ Mmio::new(base) },
        }
    }
}

impl Serial for MySerialDriver {
    fn write_byte(&self, byte: u8) {
        self.base.write32(Self::WRITE, u32::from(byte));
    }
    fn read_byte(&self) -> u8 {
        self.base.read32(Self::READ) as u8
    }
}

fn init(resource: DeviceResource, _query: &dyn DeviceQuery) -> Result<&'static dyn Serial, DriverProbeError> {
    let base = resource.mmio_base().ok_or(DriverProbeError::WrongResource)?;
    static INSTANCE: StaticCell<MySerialDriver> = StaticCell::uninit();
    Ok(INSTANCE.get_or_init(|| {
        unsafe {MySerialDriver::new(base)}
    }))
}

register_driver!(MY_UART, "my_uart_fdt_compatible_string", init, Serial);
```

Now that was a lot, and it went by rather fast. Now, that's probably because I used a bunch of traits and structs without stopping 
to really explain what they are. So i'll finish this off by Giving you links to the source code itself, so you can look at it!

[Driver](/hal/src/driver_traits/driver.rs)

[DeviceResource and DeviceQuery](/hal/src/services/device_query.rs)

[Mmio](/hal/src/services/mmio.rs)

[Port](/hal/src/x86_64/port.rs)

[Serial](/hal/src/driver_traits/serial.rs)

[Example Drivers](/hal/src/drivers/mod.rs)


