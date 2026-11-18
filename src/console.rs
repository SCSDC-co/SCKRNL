use core::fmt::{self, Write};
use core::sync::atomic::{AtomicUsize, Ordering};

// Used until the DTB has been parsed
const DEFAULT_UART_BASE: usize = 0x0900_0000;
static UART_BASE: AtomicUsize = AtomicUsize::new(DEFAULT_UART_BASE);

pub fn set_base(base: usize) {
    UART_BASE.store(base, Ordering::Relaxed);
}

pub struct Uart;

unsafe fn putc(base: *mut u32, c: u8) {
    unsafe {
        let fr = base.byte_add(0x18);
        while fr.read_volatile() & (1 << 5) != 0 {}
        base.write_volatile(c as u32);
    }
}

impl Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let base = UART_BASE.load(Ordering::Relaxed) as *mut u32;
        for b in s.bytes() {
            unsafe {
                if b == b'\n' {
                    putc(base, b'\r');
                }
                putc(base, b);
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
