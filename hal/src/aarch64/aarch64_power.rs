//! Provides power related functions for aarch64 systems

use core::arch::asm;
use core::sync::atomic::{AtomicU8, Ordering};

pub enum PsciMethod {
    Hvc = 0,
    Smc = 1,
    Uninitialized = 2,
}

static PSCI_METHOD: AtomicU8 = AtomicU8::new(PsciMethod::Uninitialized as u8);

pub fn init(method: &str) -> bool {
    let value = if method == "hvc" {
        PsciMethod::Hvc as u8
    } else if method == "smc" {
        PsciMethod::Smc as u8
    } else {
        return false;
    };
    
    PSCI_METHOD.store(value, Ordering::Release);
    true
}

pub fn system_off() {
    match PSCI_METHOD.load(Ordering::Acquire) {
        0 => {
            unsafe {
                asm!(
                "hvc #0",
                in("x0") 0x84000008u64,
                options(nostack, nomem)
                );
            }
        }
        1 => {
            unsafe {
                asm!(
                "smc #0",
                in("x0") 0x84000008u64,
                options(nostack, nomem)
                );
            }
        }
        _ => return,
    }
}