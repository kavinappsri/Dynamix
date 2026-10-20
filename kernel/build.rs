//! Re-links the kernel when a linker script changes. The scripts are passed to
//! the linker through `.cargo/config.toml` rustflags, which cargo does not
//! track on its own.

fn main() {
    for script in ["asm/aarch64/aarch64_linker.ld", "asm/armv7/armv7_linker.ld"] {
        println!("cargo::rerun-if-changed={script}");
    }
    println!("cargo::rerun-if-changed=build.rs");
}
