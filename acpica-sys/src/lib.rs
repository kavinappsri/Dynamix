#![no_std]
#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    improper_ctypes,
    unnecessary_transmutes,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]

//! Raw FFI to [ACPICA](https://github.com/acpica/acpica), the reference ACPI
//! implementation (table parsing, AML interpreter, namespace, events, power
//! management).
//!
//! This crate is only the *binding*: the vendored C sources are compiled by
//! `build.rs` and the Rust declarations come from `bindgen`. Everything in it
//! is `unsafe` and uses ACPICA's own names (`AcpiInitializeSubsystem`,
//! `ACPI_OBJECT`, ...). The safe, Rust-shaped layer lives in `hal`.
//!
//! ACPICA calls *into* the kernel through the OS services layer (`AcpiOs*`);
//! that layer is `hal::services::acpica_osl`, and those functions are
//! deliberately not declared here.
//!
//! Only built for x86_64; on other architectures this crate is empty.

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// `AE_*` status codes (generated from acexcep.h; see build.rs).
include!(concat!(env!("OUT_DIR"), "/status_codes.rs"));

/// `ACPI_SUCCESS`: ACPICA's `AE_OK` is zero.
pub const fn acpi_success(status: u32) -> bool {
    status == 0
}

/// `ACPI_FAILURE`.
pub const fn acpi_failure(status: u32) -> bool {
    status != 0
}
