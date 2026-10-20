# Dynamix

Hello, whatever soul who dove into the depths of GitHub to come look at this project.
For your information, this project (`Dynamix`) is a kernel. Or at least intends to be. currently
it's just a bunch of code that does shenanigans on bare metal. 

If you came here to _actually_ find a decent kernel, your better off somewhere else. That's
(if that happens) is several years into the future. I'm not really sure
why you haven't used Linux yet, but that's what you should be doing. 

To those of you who are still here, welcome to the club! It currently only consists of me, me and me again
(Unless I have stalkers). I don't know why you're here, but as long as you are, which won't be very long, we might
as well learn how this thing works

## Executing the Program

### Perquisites

To properly run this, you'll need a few things, such as
1. QEMU / An actual device (if you're insane)
2. Rust 2024 or newer
3. OVMF (if you're targeting x86_64)
4. Patience

### Actually Doing it

Here I'm going to assume you're gonna run it on QEMU, 

1. Clone the git onto your computer. 
2. If your running it for x86_64, make a folder called `ovmf` at project root. copy the code and vars file into it
3. Now, simply do `cargo run --features board-qemu-virt`. if everything is set up properly, it should work.

Now, if you're going to do it on actual hardware, I'd recommend you do it on a x64 laptop with UEFI. That way everything has a 
decent chance of working properly. if ur insistent on doing ot on ARM, well you can .... just that you'll have to write the 
drivers for that as well. You can find out about that [here](docs/Drivers.md)

## Actually understanding what the hell is going on inside the code

Now, here you're a bit on your own. there is a lot of things that go on inside these folders, but the least I can do is
give you a brief overview

The code is split into 2 crates, `kernel` and `hal`. If you want to change what the program can do with its existing resources, 
you go to `kernel`, but if you want to change / give the kernel more resources, you go to `hal`. currently, since the 
program only really has a simple shell and framebuffer, the `hal` crate is the bigger one. 

The sequence is like this  - there is an entrypoint in the code for each architecture it supports (`aarch64-unknown-none`,
`armv7-none-eabi`, `x86_64-unknown-uefi`), but they mainly can be categorized into 2 types - FDT entry, and ACPI entry. 
Each entry must do a few things - turn on the mmu, enable some required hardware such as the one for FP, etc..., and 
most importantly, construct and pass to the kernel a `BootInfo` struct, which contains information like a Device tree,
reserved memory regions, etc... The kernel can then take that and do what it wants, i.e. make a shell. (ik this isn't
a kernel is supposed to do, but trust me, i've been working on the `hal` for a bit of time, so the kernel is lacking)

Beyond this, you're gonna have to look at the source code yourself. Good Luck!
