# DynamixOS

DynamixOS is an experimental, freestanding Rust kernel and OS.
QEMU's `virt` machine is the currently supported execution platform and is
used for early bring-up. It demonstrates the path from a bootloader-supplied
device tree to UART output, `fw_cfg` discovery, and a RAM-backed framebuffer.
The code is deliberately small and structured so that platform and driver
support can grow without redefining the kernel.

## Architecture

The workspace separates machine-independent driver interfaces from kernel
policy:

```text
boot.s -> rust_main(dtb pointer)
             |
             +-> kernel::dtb       parses the bootloader's FDT
             +-> hal::probe_serial discovers a registered UART driver
             +-> hal::FwCfg/RamFb  configures QEMU display output
             +-> kernel modules    console, logo, timer, power, exceptions
```

`kernel` owns boot sequencing, device-tree parsing, exception reporting, and
the interactive serial console. `hal` is a `no_std` library that contains the
MMIO wrapper, polling serial trait, linker-section driver registry, ARM PL011
driver, and platform-specific device drivers. The linker script in
[`kernel/asm/aarch64/aarch64_linker.ld`](kernel/asm/aarch64/aarch64_linker.ld) places the image at `0x4020_0000` and
keeps registered serial drivers in a dedicated section.

## Workspace crates

| Crate | Purpose |
| --- | --- |
| `hal` | `no_std` hardware abstraction library. Feature `qemu-virt` (the current default platform configuration) enables QEMU framebuffer support; `pl011` enables PL011 registration. |
| `kernel` | `no_std`, `no_main` AArch64 kernel binary named `dynamix_kernel`. It consumes `hal` and provides the boot entry point. |

The Python utilities in [`Tools`](Tools) support image generation and inspection
of boot-related artifacts; they are managed separately with Poetry.

## Prerequisites

- A recent Rust toolchain with the `aarch64-unknown-none` target.
- For the currently supported QEMU `virt` setup: an AArch64 QEMU system
  emulator (`qemu-system-aarch64`).
- A boot flow for the selected platform that loads this image at the address
  described by the linker script and supplies a valid Flattened Device Tree
  pointer in `x0`.

Install the Rust target if needed:

```sh
rustup target add aarch64-unknown-none
```

For the optional image tools, install Poetry and run `poetry install` at the
repository root.

## Build

Build the whole Rust workspace for its target architecture:

```sh
cargo build --workspace --target aarch64-unknown-none
```

Build just the kernel release image:

```sh
cargo build -p kernel --release --target aarch64-unknown-none
```

The resulting executable is
`target/aarch64-unknown-none/release/dynamix_kernel`. Integrate that ELF with
your selected platform's boot flow. For the current QEMU `virt` setup, this
means a matching QEMU invocation; the repository does not yet provide a
complete launch script or boot-image packager.

## Using the crates

The kernel uses `hal` as a path dependency. The central integration point is
the `DeviceTree` trait: the kernel's DTB parser implements it, allowing
`hal::probe_serial` to select a serial driver registered by compatible string.

```rust,ignore
use hal::{probe_serial, DeviceTree};

fn start_console(tree: &impl DeviceTree) {
    let uart = probe_serial(tree).expect("a supported UART is required");
    uart.write_str("DynamixOS is running\n");
}
```

For QEMU graphics, create a `Framebuffer` over exclusively owned XRGB8888
memory, configure it with `RamFb::configure`, then use `fill` or
`blit_xrgb8888`. These APIs interact with MMIO/DMA and are intended for the
freestanding kernel environment, not a hosted application.

## Development notes

- All hardware addresses must be mapped before constructing HAL devices.
- The current QEMU `virt` configuration expects a PL011 UART and the
  `qemu,fw-cfg-mmio` device to be described by the DTB. Other platforms need
  the appropriate enabled drivers and boot integration.
- Build profiles use `panic = "abort"`; exception and panic reporting write to
  the early PL011 UART.
- Generate API documentation with
  `cargo doc --workspace --target aarch64-unknown-none --no-deps`.

## Status

This is an early bring-up kernel, not a general-purpose operating system. QEMU
`virt` is currently the only supported runtime platform; the architecture is
intended to accommodate additional platforms as their drivers and boot
integration become ready.
