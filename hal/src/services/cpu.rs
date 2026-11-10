//! Architecture-neutral CPU control.

pub fn hlt_frvr() -> ! {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: masking interrupts has no memory effects, and nothing here
    // depends on them being enabled.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    loop {
        // SAFETY: both instructions only idle the core until the next event.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }

        #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
        unsafe {
            core::arch::asm!("wfe", options(nomem, nostack));
        }
    }
}

/// Saved interrupt-enable state; see [`irq_save`] / [`irq_restore`].
#[derive(Clone, Copy, Debug)]
pub struct IrqState(usize);

/// Masks interrupts on the current core and returns what the state was
/// before
#[inline]
pub fn irq_save() -> IrqState {
    #[cfg(target_arch = "x86_64")]
    {
        let flags: usize;
        // SAFETY: push/pop are balanced; `cli` only masks interrupts. Memory is
        // deliberately not declared untouched so this acts as a compiler barrier.
        unsafe { core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags) };
        IrqState(flags)
    }
    #[cfg(target_arch = "aarch64")]
    {
        let daif: usize;
        // SAFETY: reads DAIF and sets the I bit; no other side effects.
        unsafe { core::arch::asm!("mrs {}, daif", "msr daifset, #2", out(reg) daif, options(nostack)) };
        IrqState(daif)
    }
    #[cfg(target_arch = "arm")]
    {
        let cpsr: usize;
        // SAFETY: reads CPSR and sets the I bit; no other side effects.
        unsafe { core::arch::asm!("mrs {}, cpsr", "cpsid i", out(reg) cpsr, options(nostack)) };
        IrqState(cpsr)
    }
}

/// Restores the state returned by [`irq_save`] on the same core.
#[inline]
pub fn irq_restore(state: IrqState) {
    #[cfg(target_arch = "x86_64")]
    if state.0 & (1 << 9) != 0 {
        // SAFETY: interrupts were enabled when the matching `irq_save` ran.
        unsafe { core::arch::asm!("sti") };
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: writes back exactly the DAIF value read by `irq_save`.
    unsafe {
        core::arch::asm!("msr daif, {}", in(reg) state.0, options(nostack))
    };
    #[cfg(target_arch = "arm")]
    // SAFETY: writes back only the CPSR control field read by `irq_save`.
    unsafe {
        core::arch::asm!("msr cpsr_c, {}", in(reg) state.0, options(nostack))
    };
}

/// Unmasks interrupts on the current core.
#[inline]
pub fn irq_enable() {
    // SAFETY: only changes the interrupt mask.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("sti");
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("msr daifclr, #2", options(nostack));
        #[cfg(target_arch = "arm")]
        core::arch::asm!("cpsie i", options(nostack));
    }
}

/// Masks interrupts on the current core.
#[inline]
pub fn irq_disable() {
    // SAFETY: only changes the interrupt mask.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("cli");
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("msr daifset, #2", options(nostack));
        #[cfg(target_arch = "arm")]
        core::arch::asm!("cpsid i", options(nostack));
    }
}

/// Whether interrupts are currently unmasked on this core.
#[inline]
pub fn irqs_enabled() -> bool {
    let state = irq_save();
    irq_restore(state);
    #[cfg(target_arch = "x86_64")]
    return state.0 & (1 << 9) != 0;
    #[cfg(target_arch = "aarch64")]
    return state.0 & (1 << 7) == 0;
    #[cfg(target_arch = "arm")]
    return state.0 & (1 << 7) == 0;
}

/// Idles the core until the next interrupt (or event) arrives.
#[inline]
pub fn wait_for_interrupt() {
    // SAFETY: idles the core only.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("hlt", options(nomem, nostack));
        #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
        core::arch::asm!("wfi", options(nomem, nostack));
    }
}

