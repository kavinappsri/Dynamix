//! The firmware text console (`ConOut`), as a [`core::fmt::Write`] sink.
//!
//! Only usable before `ExitBootServices`.

use core::fmt;

use super::types::EfiSimpleTextOutputProtocol;

/// UCS-2 code units written per `OutputString` call (NUL excluded).
const CHUNK: usize = 63;

/// A handle to the firmware console. A null console silently discards output.
pub struct ConOut(*mut EfiSimpleTextOutputProtocol);

impl ConOut {
    /// # Safety
    /// `con_out` must be null or a valid `EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL*`,
    /// and Boot Services must not exit while the value is in use.
    pub unsafe fn new(con_out: *mut EfiSimpleTextOutputProtocol) -> Self {
        Self(con_out)
    }

    fn flush(&mut self, buf: &mut [u16; CHUNK + 1], len: &mut usize) {
        if *len == 0 {
            return;
        }
        buf[*len] = 0;
        // SAFETY: upheld by `new`; `buf[..=len]` is NUL terminated UCS-2.
        unsafe { ((*self.0).output_string)(self.0, buf.as_ptr()) };
        *len = 0;
    }
}

impl fmt::Write for ConOut {
    /// Converts to UCS-2 in fixed chunks (there is no length-based
    /// `OutputString`). `\n` becomes `\r\n`; non-ASCII becomes `?`.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.0.is_null() {
            return Ok(());
        }
        let mut buf = [0u16; CHUNK + 1];
        let mut len = 0;
        for byte in s.bytes() {
            // A newline needs two units; keep room for both.
            if len + 2 > CHUNK {
                self.flush(&mut buf, &mut len);
            }
            match byte {
                b'\n' => {
                    buf[len] = b'\r' as u16;
                    buf[len + 1] = b'\n' as u16;
                    len += 2;
                }
                b if b.is_ascii() => {
                    buf[len] = b as u16;
                    len += 1;
                }
                _ => {
                    buf[len] = b'?' as u16;
                    len += 1;
                }
            }
        }
        self.flush(&mut buf, &mut len);
        Ok(())
    }
}
