use core::fmt::{self, Write};

const UART0: *mut u32 = 0x0900_0000 as *mut u32;

pub struct Uart;

impl Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            unsafe {
                if b == b'\n' {
                    UART0.write_volatile(b'\r' as u32);
                }
                UART0.write_volatile(b as u32);
            }
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use ::core::fmt::Write as _;
        let _ = ::core::write!($crate::console::Uart, $($arg)*);
    }};
}

#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {{
        use ::core::fmt::Write as _;
        let _ = ::core::writeln!($crate::console::Uart, $($arg)*);
    }};
}
