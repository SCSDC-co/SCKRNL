use core::arch::global_asm;
use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{println, semihost_exit};

global_asm!(include_str!("vectors.s"), handler = sym handle_exception);

#[repr(C)]
pub struct TrapFrame {
    pub x: [u64; 31],
    pub elr: u64,
    pub spsr: u64,
    pub esr: u64,
    pub far: u64,
    _pad: u64, // keeps the size a multiple of 16
}

const _: () = {
    assert!(size_of::<TrapFrame>() == 288);
    assert!(offset_of!(TrapFrame, elr) == 248);
    assert!(offset_of!(TrapFrame, far) == 272);
    assert!(offset_of!(TrapFrame, spsr) == 256);
    assert!(offset_of!(TrapFrame, esr) == 264);
};

static IN_HANDLER: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_exception(frame: &mut TrapFrame, index: u64) {
    let source = index / 4;
    let kind = index % 4;

    #[cfg(feature = "fault-tests")]
    if frame.esr & 0xFFFF == 0xDEAD && frame.x[0] == 0xF055F055F055F055 {
        println!("\nBRK Exception test, returning to kernel...\n");
        frame.elr += 4;
        return;
    }

    if IN_HANDLER.swap(true, Ordering::AcqRel) {
        loop {
            semihost_exit(2)
        }
    }

    println!("\nCPU EXCEPTION");
    println!(
        "Caused by: {} - {}",
        vector_source(source),
        vector_kind(kind)
    );
    println!("\nSPECIAL REGISTER DUMP:");
    println!("SPSR: {:#018x}", frame.spsr);
    println!("ESR:  {:#018x}", frame.esr);
    println!("FAR:  {:#018x}", frame.far);
    println!("ELR:  {:#018x}", frame.elr);
    println!("\nGENERAL REGISTER DUMP:");
    for (reg, contents) in frame.x.chunks(2).enumerate() {
        if contents.len() == 2 {
            println!(
                "x{:<2}: {:#018x}     x{:<2}: {:#018x}",
                reg * 2,
                contents[0],
                reg * 2 + 1,
                contents[1]
            )
        } else {
            println!("x{:<2}: {:#018x}", reg * 2, contents[0]);
        }
    }

    IN_HANDLER.store(false, Ordering::Release);

    semihost_exit(2)
}

fn vector_source(source: u64) -> &'static str {
    match source {
        0 => "Current EL SP0",
        1 => "Current EL SPx",
        2 => "Lower EL AArch64",
        3 => "Lower EL AArch32",
        _ => "Unknown source",
    }
}

fn vector_kind(kind: u64) -> &'static str {
    match kind {
        0 => "Synchronous",
        1 => "IRQ",
        2 => "FIQ",
        _ => "SError",
    }
}
