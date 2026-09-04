#[cfg(feature = "driver-pl011")]
/// ARM PrimeCell PL011 UART support.
pub mod pl011;
#[cfg(feature = "driver-dw-8250")]
/// DesignWare 8250 UART Core Support
pub mod dw_8250;