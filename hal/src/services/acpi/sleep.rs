//! Sleep states:

/// An ACPI sleep state.
///
/// Only soft-off exists; S3 and friends need the wake path (FACS waking
/// vector, CPU state save) and get added when something wants them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SleepState {
    /// Soft off.
    S5 = 5,
}