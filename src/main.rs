#![no_std]
#![no_main]

use crate::frame_allocator::FrameAllocator;
use crate::mmu::PAGE;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use fdt::Fdt;
use mmu::Range;
use mmu::Region;
use spinning_top::RawSpinlock;
use talc::TalcLock;
use talc::source::Manual;

mod console;
mod exceptions;
mod frame_allocator;
mod mmu;

extern crate alloc;

#[global_allocator]
static HEAP: TalcLock<RawSpinlock, Manual> = TalcLock::new(Manual);

type FreeList = mmu::RangeSet<32>;

global_asm!(include_str!("boot.s"));

const NONE: Region = Region { base: 0, size: 0 };

const HEAP_FRAMES: usize = 4096;

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

fn init_mmu(fdt: &Fdt, image: (u64, u64), dtb_ptr: u64) -> FreeList {
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

    let mut devices = [NONE; 8];
    let mut ndev: usize = 0;

    if let Some(console) = fdt.chosen().stdout() {
        push_regs(console, &mut devices, &mut ndev);
    }

    if let Some(gic) = fdt.find_compatible(&["arm,gic-v3"]) {
        push_regs(gic, &mut devices, &mut ndev);
    }

    mmu::trim_ram(&mut mem[..nmem]);

    let map = unsafe { mmu::build_identity_map(&mem, &devices, image) };

    let mut ranges = FreeList::new();

    for m in &mem[..nmem] {
        if m.size == 0 {
            continue;
        }

        ranges.insert(Range {
            start: m.base,
            end: m.base.saturating_add(m.size),
        });
    }

    mmu::reserve(&mut ranges, image.0, image.1);

    mmu::reserve(
        &mut ranges,
        dtb_ptr,
        dtb_ptr.saturating_add(fdt.total_size() as u64),
    );

    if let Some(reserved_mem) = fdt.find_node("/reserved-memory") {
        for node in reserved_mem.children() {
            let disabled = node
                .property("status")
                .and_then(|p| p.as_str())
                .is_some_and(|s| s != "okay" && s != "ok");
            if disabled {
                continue;
            }

            let Some(regs) = node.reg() else {
                continue;
            };
            for reg in regs {
                let size = reg.size.unwrap_or(0) as u64;
                if size == 0 {
                    continue;
                }
                mmu::reserve(
                    &mut ranges,
                    reg.starting_address as u64,
                    (reg.starting_address as u64).saturating_add(size),
                );
            }
        }
    }

    for reservation in fdt.memory_reservations() {
        if reservation.size() == 0 {
            continue;
        }

        let start = reservation.address() as u64;

        mmu::reserve(
            &mut ranges,
            start,
            start.saturating_add(reservation.size() as u64),
        );
    }

    let optee_size = smc(0xB2000007, 0, 0, 0);

    if optee_size[0] == 0 {
        mmu::reserve(
            &mut ranges,
            optee_size[1],
            optee_size[1].saturating_add(optee_size[2]),
        );
    }

    for r in ranges.iter() {
        assert!(r.start % mmu::PAGE == 0 && r.end % mmu::PAGE == 0);
    }
    ranges.sort();

    for (i, range) in ranges.iter().enumerate() {
        println!(
            "Range {i}: start: {:#018x} end: {:#018x}",
            range.start, range.end
        );
    }

    println!("Free: {} MiB", ranges.total() >> 20);

    unsafe {
        mmu::enable(map);
    }

    ranges
}

fn init_heap(frames: &mut FrameAllocator) {
    let run = frames
        .alloc_contiguous(HEAP_FRAMES, 512)
        .expect("no contiguous memory for the heap");
    let base = frame_allocator::phys_to_virt(run.addr);
    let size = HEAP_FRAMES * PAGE as usize;
    unsafe { HEAP.lock().claim(base, size).expect("heap claim failed") };
}

#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub extern "C" fn kmain(dtb_ptr: usize) -> ! {
    let el: u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) el) };

    let image = (
        &raw const __image_start as u64,
        &raw const __image_end as u64,
    );

    let fdt = unsafe { Fdt::from_ptr(dtb_ptr as *const u8) }.expect("FDT Error");

    if let Some(console) = fdt.chosen().stdout() {
        if let Some(reg) = console.reg().and_then(|mut r| r.next()) {
            console::set_base(reg.starting_address as usize);
        }
    }

    let free = init_mmu(&fdt, image, dtb_ptr as u64);

    #[allow(unused_mut)]
    let mut falloc = FrameAllocator::new_dynamic(&free);

    assert_eq!(
        falloc.free_frames(),
        free.total() / mmu::PAGE - falloc.meta_pages() as u64
    );

    init_heap(&mut falloc);

    println!("Hello from SCKRNL!");
    println!("running at EL{}", (el >> 2) & 3);

    #[cfg(feature = "fault-tests")]
    test_fault(&mut falloc);

    #[cfg(feature = "fault-tests")]
    println!("Back to kmain!");

    println!("\nShutting down");

    poweroff()
}

//#[cfg(feature = "fault-tests")]
#[allow(static_mut_refs)]
fn test_fault(falloc: &mut FrameAllocator) {
    println!("Testing Exception");
    let reg: u64 = 0xF055F055F055F055;
    unsafe {
        asm!("brk #0xDEAD", in("x0") reg);
    }

    let before = falloc.free_frames();

    println!("Current free frames: {}", before);

    println!("Allocating 512 contiguous frames");
    let cont_pages = match falloc.alloc_contiguous(512, 512) {
        Some(p) => p,
        None => panic!("Could not allocate frames"),
    };

    println!("Free frames after allocation: {}", falloc.free_frames());

    assert!(
        cont_pages.addr % 0x200000 == 0,
        "Address is not 2MiB aligned"
    );

    assert_eq!(
        before,
        falloc.free_frames() + 512,
        "Free frame count mismatch"
    );

    println!("Freeing frames");

    falloc.free_contiguous(cont_pages, 512);

    println!("Free frames after frame free: {}", falloc.free_frames());

    assert_eq!(before, falloc.free_frames(), "Free frame count mismatch");

    println!("Allocating vector to test heap allocation");

    let mut vector: Vec<u64> = Vec::new();

    for i in 0..10 as u64 {
        vector.push(i);
    }

    drop(vector);

    println!("Allocating out-of-memory vector (shouldn't work)");
    match Vec::<u8>::new().try_reserve(HEAP_FRAMES * PAGE as usize * 2) {
        Ok(_) => panic!("Could allocate vector out of memory"),
        Err(_) => {}
    }

    println!("Allocation errored (good)");

    println!("Testing leaks");

    let start = HEAP.lock().counters().available_bytes;

    for _ in 0..20 {
        let mut v = vec![10];
        for i in 0..5 {
            v.push(i);
        }
    }

    let end = HEAP.lock().counters().available_bytes;

    assert_eq!(
        start, end,
        "Leaks found. Start free memory: {start} bytes, end free memory: {end} bytes"
    )
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("KERNEL PANIC: {}", info);
    semihost_exit(1)
}
