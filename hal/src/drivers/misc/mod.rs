#[cfg(feature = "driver-pl011")]
/// ARM PrimeCell PL011 UART
pub mod pl011;
#[cfg(feature = "driver-dw-8250")]
/// DesignWare 8250 UART Core
pub mod dw_8250;

#[cfg(all(feature = "driver-ns16550", target_arch = "x86_64"))]
/// 16550 UART over x86 port I/O.
pub mod ns16550;