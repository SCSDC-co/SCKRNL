#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

mod console;
mod exceptions;

use fdt::Fdt;

global_asm!(include_str!("boot.s"));

#[allow(dead_code)]
fn poweroff() -> ! {
    smc(0x8400_0008, 0, 0, 0);
    loop {
        unsafe { asm!("wfe", options(nomem, nostack)) };
    }
}

#[allow(dead_code)]
fn reboot() -> ! {
    smc(0x8400_0009, 0, 0, 0);
    loop {
        unsafe { asm!("wfe", options(nomem, nostack)) };
    }
}

#[allow(dead_code)]
fn semihost_exit(code: u64) -> ! {
    let block: [u64; 2] = [0x20026, code];
    unsafe {
        asm!(
            "hlt #0xf000",
            in("x0") 0x18u64,
            in("x1") block.as_ptr(),
            options(noreturn)
        );
    }
}

fn smc(id: u64, a1: u64, a2: u64, a3: u64) -> [u64; 4] {
    let (mut r0, mut r1, mut r2, mut r3) = (id, a1, a2, a3);
    unsafe {
        asm!("smc #0",
            inout("x0") r0, inout("x1") r1, inout("x2") r2, inout("x3") r3,
            out("x4") _, out("x5") _, out("x6") _, out("x7") _, out("x8") _,
            out("x9") _, out("x10") _, out("x11") _, out("x12") _, out("x13") _,
            out("x14") _, out("x15") _, out("x16") _, out("x17") _,
            options(nostack));
    }
    [r0, r1, r2, r3]
}

#[unsafe(no_mangle)]
pub extern "C" fn kmain(dtb_ptr: usize) -> ! {
    let el: u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) el) };

    println!("Hello from SCKRNL!");
    println!("running at EL{}", (el >> 2) & 3);
    let ver = smc(0x8400_0000, 0, 0, 0);

    println!("PSCI_VERSION: {:#x}", ver[0]);

    let fdt = unsafe { Fdt::from_ptr(dtb_ptr as *const u8) }.expect("FDT Error");

    let root = fdt.find_node("/").expect("Root node not found");

    for node in root.children() {
        println!("Node name: {}", node.name);
    }

    println!("Asking op-tee the uid...");
    let recv = smc(0xBF00_FF01, 0, 0, 0);

    for (i, reg) in recv.iter().enumerate() {
        println!("Response x{i}: {reg:#0x}");
    }

    #[cfg(feature = "fault-tests")]
    crash();

    #[cfg(feature = "fault-tests")]
    println!("Back to kmain!");

    println!("\nShutting down");

    poweroff()
}

#[cfg(feature = "fault-tests")]
fn crash() {
    println!("Exception");
    let reg: u64 = 0xF055F055F055F055;
    unsafe {
        asm!("brk #0xDEAD", in("x0") reg);
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("KERNEL PANIC: {}", info);
    semihost_exit(1)
}
