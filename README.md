# Dynamix

__THIS PROJECT IS BY NO MEANS READY YET, ITS IS A WORK IN PROGRESS__

Dynamix is an OS, currently being made as a hobby project. It's coded in Rust. It currently supports QEMU, both aarch64
and armv7. More platform are intended to be added as their integration and drivers become ready. At this stage the 
project only shows its boot logo and has a shell which allows you to change the display and shut it down

## Crates

This project currently has 2 crates - kernel, and hal

Kernel is the main part of project, it is the part of the OS that performs the higher level logic

Hal (Hardware Abstraction Layer) is a set of low level features that accommodate for differences in platform, assisting 
in setting up the runtime environment for rust and setting up the exception vector table, among other things.

## General Execution Flow

Please look at `main.rs` for reference

The boot assembly (automatically run by importing `hal`) writes the Linux Boot Header, turns on FP/NEON hardware, set 
the stack pointer, clears the ram, sets the FDT pointer in register 0, and branches to `rust_main` (defined in `main.rs`).

In `rust_main`, the exception vector table is set up. The FDT is found and parsed by the kernel, which then passes it to
HAL functions named `probe_serial` and `probe_framebuffer` to find compatible drivers that were compiled with it. The kernel
then sets up `hal::power` by passing in the `method` property of the `psci` node in the FDT to it.

The kernel then draws its boot logo on the screen, and automatically shuts down after 5 seconds. if the code for the shutdown
is removed, the kernel will run on a simple shell via the UART, which allows the user to set the display or shut the kernel
down.

## Building and Running

### Prerequisites

* Rust
* Python
* Poetry
* QEMU (if you are running this on your computer)

### Process

1. Set up the Poetry venv by running the given command at the workspace root

```bash
poetry install
```

2. Convert the boot logo to a .bin by running

```bash
cd Tools
python generate_image.py ../Assets/Dynamix.png ../kernel/src/logo.bin
```

3. Build for your targeted environment, passing in the flags of the drivers you want to compile with it, or the board you
want to compile for (currently only the drivers for qemu-virt board are supported)

```bash
cargo build --target aarch64-unknown-none --feature board-qemu-virt
```

4. To run this on your computer, simply run with cargo (this assumes you have the associated qemu board installed)


```bash
cargo run
```

if that does not work, invoke QEMU manually

```bash
FILE="target/aarch64-unknown-none/release/dynamix_kernel" && rust-objcopy -O binary "$FILE" "$FILE.bin" && qemu-system-aarch64 -machine virt -cpu cortex-a72 -device ramfb -serial stdio -monitor none -d cpu,in_asm -kernel "$FILE.bin
```
