pub mod gic_v2;
// The generic timer and PSCI are part of the ARM architecture itself, not of
// any board, so there are no features for them.
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
pub mod generic_timer;
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
pub mod psci;