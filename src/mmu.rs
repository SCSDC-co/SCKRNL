use aarch64_cpu::{asm::barrier, registers::*};
use core::arch::asm;

pub const BLOCK_2M: u64 = 2 << 20;
pub const PAGE: u64 = 4096;

const ATTR_DEVICE: u64 = 0;
const ATTR_NORMAL: u64 = 1;

const VALID: u64 = 1 << 0;
const TABLE: u64 = 0b11;
const AF: u64 = 1 << 10;
const SH_INNER: u64 = 0b11 << 8;
const SH_OUTER: u64 = 0b10 << 8;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;

const NORMAL_RWX: u64 = VALID | (ATTR_NORMAL << 2) | SH_INNER | AF;
const NORMAL_RW: u64 = NORMAL_RWX | PXN | UXN;
const DEVICE: u64 = VALID | (ATTR_DEVICE << 2) | SH_OUTER | AF | PXN | UXN;

#[repr(C, align(4096))]
pub struct Table([u64; 512]);

const MAX_L2: usize = 8;
static mut ROOT: Table = Table([0; 512]);
static mut L2: [Table; MAX_L2] = [const { Table([0; 512]) }; MAX_L2];
static mut L2_USED: usize = 0;

pub struct Region {
    pub base: u64,
    pub size: u64,
}

#[derive(Clone, Copy)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

impl Range {
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

pub struct RangeSet<const N: usize> {
    range: [Range; N],
    len: usize,
}

impl<const N: usize> RangeSet<N> {
    pub const fn new() -> Self {
        Self {
            range: [Range { start: 0, end: 0 }; N],
            len: 0,
        }
    }

    pub fn insert(&mut self, x: Range) {
        if x.is_empty() {
            return;
        }
        assert!(self.len < N, "RangeSet full");
        self.range[self.len] = x;
        self.len += 1;
    }
    pub fn subtract(&mut self, x: Range) {
        if x.is_empty() {
            return;
        }
        let mut i = 0;
        while i < self.len {
            let f = self.range[i];
            if x.end <= f.start || x.start >= f.end {
                i += 1;
                continue;
            }

            let left = Range {
                start: f.start,
                end: x.start,
            };
            let right = Range {
                start: x.end,
                end: f.end,
            };
            match (!left.is_empty(), !right.is_empty()) {
                (true, true) => {
                    self.range[i] = left;
                    self.insert(right);
                    i += 1;
                }
                (true, false) => {
                    self.range[i] = left;
                    i += 1;
                }
                (false, true) => {
                    self.range[i] = right;
                    i += 1;
                }
                (false, false) => {
                    self.len -= 1;
                    self.range[i] = self.range[self.len];
                }
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Range> {
        self.range[..self.len].iter()
    }

    pub fn sort(&mut self) {
        self.range[..self.len].sort_unstable_by_key(|r| r.start);
    }

    pub fn total(&self) -> u64 {
        self.iter().map(|r| r.end - r.start).sum()
    }

    pub fn mem_start(&self) -> Result<u64, ()> {
        if self.len == 0 {
            return Err(());
        }

        Ok(self.range[0].start)
    }

    pub fn mem_end(&self) -> Result<u64, ()> {
        if self.len == 0 {
            return Err(());
        }

        Ok(self.range[self.len - 1].end)
    }
}

pub fn reserve<const N: usize>(free: &mut RangeSet<N>, start: u64, end: u64) {
    if start >= end {
        return;
    }
    free.subtract(Range {
        start: align_down(start, PAGE),
        end: align_up(end, PAGE),
    });
}

pub const fn align_down(x: u64, a: u64) -> u64 {
    x & !(a - 1)
}
pub const fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

unsafe fn l2_for(i1: usize) -> *mut u64 {
    unsafe {
        let entry = (&raw mut ROOT).cast::<u64>().add(i1);
        let cur = entry.read();
        if cur & VALID != 0 {
            assert!(cur & 0b10 != 0, "a 1 GiB block is in the way");
            return (cur & 0x0000_ffff_ffff_f000) as *mut u64;
        }
        let n = L2_USED;
        assert!(n < MAX_L2, "out of L2 tables");
        L2_USED = n + 1;
        let tbl = (&raw mut L2).cast::<Table>().add(n).cast::<u64>();
        entry.write(tbl as u64 | TABLE);
        tbl
    }
}

unsafe fn map_blocks(start: u64, end: u64, attrs: impl Fn(u64) -> u64) {
    assert!(start % BLOCK_2M == 0 && end % BLOCK_2M == 0 && start < end);
    assert!(end <= 1 << 39, "outside the 39-bit TTBR0 range");
    let mut pa = start;
    while pa < end {
        let i1 = ((pa >> 30) & 0x1ff) as usize;
        let i2 = ((pa >> 21) & 0x1ff) as usize;
        unsafe {
            let slot = l2_for(i1).add(i2);
            let desc = pa | attrs(pa);
            let cur = slot.read();
            assert!(cur == 0 || cur == desc, "conflicting mapping at {pa:#x}");
            slot.write(desc);
        }
        pa += BLOCK_2M;
    }
}

pub fn trim_ram(ram: &mut [Region]) {
    for r in ram {
        let start = align_up(r.base, BLOCK_2M);
        let end = align_down(r.base.saturating_add(r.size), BLOCK_2M);
        r.base = start;
        r.size = end.saturating_sub(start);
    }
}

pub unsafe fn build_identity_map(ram: &[Region], devices: &[Region], image: (u64, u64)) -> u64 {
    for r in ram {
        if r.size == 0 {
            continue;
        }
        let start = align_up(r.base, BLOCK_2M);
        let end = align_down(r.base + r.size, BLOCK_2M);
        if start < end {
            unsafe {
                map_blocks(start, end, |pa| {
                    if pa < image.1 && pa + BLOCK_2M > image.0 {
                        NORMAL_RWX
                    } else {
                        NORMAL_RW
                    }
                });
            }
        }
    }
    for d in devices {
        if d.size == 0 {
            continue;
        }
        let start = align_down(d.base, BLOCK_2M);
        let end = align_up(d.base + d.size, BLOCK_2M);
        unsafe { map_blocks(start, end, |_| DEVICE) };
    }
    unsafe { core::arch::asm!("dsb ish", options(nostack)) };
    (&raw const ROOT) as u64
}

pub unsafe fn enable(root_table_phys: u64) {
    let mut parange: u64;
    unsafe {
        asm!("mrs {}, id_aa64mmfr0_el1", out(reg) parange, options(nomem, nostack, preserves_flags));
    }

    parange = parange & 0b1111;

    let ips = match parange {
        0 => TCR_EL1::IPS::Bits_32,
        1 => TCR_EL1::IPS::Bits_36,
        2 => TCR_EL1::IPS::Bits_40,
        3 => TCR_EL1::IPS::Bits_42,
        4 => TCR_EL1::IPS::Bits_44,
        _ => TCR_EL1::IPS::Bits_48,
    };

    unsafe {
        asm!("dsb ish");
    }

    MAIR_EL1.write(
        MAIR_EL1::Attr0_Device::nonGathering_nonReordering_EarlyWriteAck
            + MAIR_EL1::Attr1_Normal_Outer::WriteBack_NonTransient_ReadWriteAlloc
            + MAIR_EL1::Attr1_Normal_Inner::WriteBack_NonTransient_ReadWriteAlloc
            + MAIR_EL1::Attr2_Normal_Outer::NonCacheable
            + MAIR_EL1::Attr2_Normal_Inner::NonCacheable,
    );

    unsafe {
        asm!("dsb ish");
    }

    TCR_EL1.write(
        TCR_EL1::T0SZ.val(25)
            + TCR_EL1::TG0::KiB_4
            + TCR_EL1::SH0::Inner
            + TCR_EL1::ORGN0::WriteBack_ReadAlloc_WriteAlloc_Cacheable
            + TCR_EL1::IRGN0::WriteBack_ReadAlloc_WriteAlloc_Cacheable
            + ips
            + TCR_EL1::EPD1.val(1)
            + TCR_EL1::TG1.val(0b10),
    );

    unsafe {
        asm!("dsb ish");
    }

    TTBR0_EL1.set(root_table_phys);

    barrier::isb(barrier::SY);

    unsafe {
        asm!("tlbi vmalle1");
        asm!("dsb nsh");
    }

    barrier::isb(barrier::SY);

    SCTLR_EL1.modify(SCTLR_EL1::M::Enable);
    barrier::isb(barrier::SY);

    SCTLR_EL1.modify(SCTLR_EL1::C::Cacheable);
    barrier::isb(barrier::SY);

    SCTLR_EL1.modify(SCTLR_EL1::I::Cacheable);
    barrier::isb(barrier::SY);
}
