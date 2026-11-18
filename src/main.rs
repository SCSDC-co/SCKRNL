#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

mod console;
mod exceptions;
mod mmu;

use aarch64_cpu::registers::*;
use fdt::Fdt;
use mmu::Region;

global_asm!(include_str!("boot.s"));

const NONE: Region = Region { base: 0, size: 0 };

unsafe extern "C" {
    static __image_start: u8;
    static __image_end: u8;
}

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

fn push_regs(node: fdt::node::FdtNode<'_, '_>, out: &mut [Region], n: &mut usize) {
    let disabled = node
        .property("status")
        .and_then(|p| p.as_str())
        .is_some_and(|s| s != "okay" && s != "ok");
    if disabled {
        return;
    }
    let Some(regs) = node.reg() else { return };
    for r in regs {
        let size = r.size.unwrap_or(0) as u64;
        if size == 0 {
            continue;
        }
        assert!(*n < out.len(), "too many device regions");
        out[*n] = Region {
            base: r.starting_address as u64,
            size,
        };
        *n += 1;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kmain(dtb_ptr: usize) -> ! {
    let el: u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) el) };

    let image = (
        &raw const __image_start as u64,
        &raw const __image_end as u64,
    );

    let fdt = unsafe { Fdt::from_ptr(dtb_ptr as *const u8) }.expect("FDT Error");

    let mut devices = [NONE; 8];
    let mut ndev: usize = 0;

    if let Some(console) = fdt.chosen().stdout() {
        push_regs(console, &mut devices, &mut ndev);
        if let Some(reg) = console.reg().and_then(|mut r| r.next()) {
            console::set_base(reg.starting_address as usize);
        }
    }

    println!("Hello from SCKRNL!");
    println!("running at EL{}", (el >> 2) & 3);
    let ver = smc(0x8400_0000, 0, 0, 0);

    println!("PSCI_VERSION: {:#x}", ver[0]);

    let mut mem = [NONE; 8];
    let mut nmem: usize = 0;

    for m in fdt.memory().regions() {
        let Some(size) = m.size else {
            continue;
        };
        assert!(nmem < mem.len(), "Too many ram regions");
        mem[nmem] = Region {
            base: m.starting_address as u64,
            size: size as u64,
        };
        nmem += 1;
    }

    if let Some(gic) = fdt.find_compatible(&["arm,gic-v3"]) {
        push_regs(gic, &mut devices, &mut ndev);
    }

    let root_table_phys = unsafe { mmu::build_identity_map(&mem, &devices, image) };

    println!("Enabling MMU\nMAIR Before: {}", MAIR_EL1.get(),);
    unsafe {
        mmu::enable(root_table_phys);
    }
    println!("MMU enabled");
    println!("MAIR After: {}", MAIR_EL1.get(),);

    #[cfg(feature = "fault-tests")]
    test_fault();

    #[cfg(feature = "fault-tests")]
    println!("Back to kmain!");

    println!("\nShutting down");

    poweroff()
}

#[cfg(feature = "fault-tests")]
fn test_fault() {
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
