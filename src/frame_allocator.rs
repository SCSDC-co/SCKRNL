use core::sync::atomic::{AtomicBool, Ordering};

use crate::{FreeList, mmu::PAGE};

static CREATED: AtomicBool = AtomicBool::new(false);

fn claim_once() {
    assert!(
        !CREATED.swap(true, Ordering::SeqCst),
        "allocator created twice"
    );
}

// Right now physical addresses match virtual ones
pub fn phys_to_virt(phys: u64) -> *mut u8 {
    phys as *mut u8
}

pub struct PhysAddr {
    pub addr: u64,
    pub page: u64,
}

pub struct FrameAllocator {
    bitmap: &'static mut [u64],
    start: u64,
    total: usize,
    hint: usize,
    meta_pages: usize,
}

#[allow(dead_code)]
impl FrameAllocator {
    fn get_bit(&self, pos: usize) -> bool {
        (self.bitmap[pos / 64] >> (pos % 64)) & 1 != 0
    }

    fn set_range(&mut self, start: usize, end: usize, used: bool) {
        assert!(end <= self.total);
        let mut pos = start;
        while pos < end {
            let word = pos / 64;
            let lo = pos % 64;
            let hi = (end - word * 64).min(64); // bits [lo, hi) of this word
            let mask = (u64::MAX >> (64 - (hi - lo))) << lo;
            if used {
                self.bitmap[word] |= mask;
            } else {
                self.bitmap[word] &= !mask;
            }
            pos = (word + 1) * 64;
        }
    }

    fn set_bit(&mut self, pos: usize, used: bool) {
        self.set_range(pos, pos + 1, used);
    }

    fn find_next(&self, from: usize, to: usize, want_set: bool) -> Option<usize> {
        let to = to.min(self.total);
        let mut pos = from;
        while pos < to {
            let w = self.bitmap[pos / 64];
            let w = if want_set { w } else { !w };
            let shifted = w >> (pos % 64);
            if shifted != 0 {
                let found = pos + shifted.trailing_zeros() as usize;
                return if found < to { Some(found) } else { None };
            }
            pos = (pos / 64 + 1) * 64;
        }
        None
    }

    fn span(mem: &FreeList) -> (u64, usize) {
        let start = mem
            .mem_start()
            .unwrap_or_else(|_| panic!("no memory ranges"));
        let end = mem.mem_end().unwrap_or_else(|_| panic!("no memory ranges"));
        assert!(start % PAGE == 0 && end % PAGE == 0);
        (start, ((end - start) / PAGE) as usize)
    }

    fn release(&mut self, mem: &FreeList) {
        for r in mem.iter() {
            let first = ((r.start - self.start) / PAGE) as usize;
            let last = ((r.end - self.start) / PAGE) as usize;
            self.set_range(first, last, false);
        }
        self.hint = self.find_next(0, self.total, false).unwrap_or(self.total);
    }

    pub fn new(mem: &FreeList, bitmap: &'static mut [u64]) -> FrameAllocator {
        claim_once();
        let (start, total) = Self::span(mem);
        assert!(
            bitmap.len() * 64 >= total,
            "bitmap too small for {total} frames"
        );
        bitmap.fill(u64::MAX); // everything used, including the tail bits
        let mut a = FrameAllocator {
            bitmap,
            start,
            total,
            hint: 0,
            meta_pages: 0,
        };
        a.release(mem);
        a
    }

    pub fn new_dynamic(mem: &FreeList) -> FrameAllocator {
        claim_once();
        let (start, total) = Self::span(mem);
        let words = total.div_ceil(64);
        let meta_pages = (words * 8).div_ceil(PAGE as usize);

        let place = mem
            .iter()
            .find(|r| (r.end - r.start) / PAGE >= meta_pages as u64)
            .unwrap_or_else(|| panic!("no free range can hold the frame bitmap"))
            .start;

        let bitmap =
            unsafe { core::slice::from_raw_parts_mut(phys_to_virt(place) as *mut u64, words) };
        bitmap.fill(u64::MAX);

        let mut a = FrameAllocator {
            bitmap,
            start,
            total,
            hint: 0,
            meta_pages,
        };
        a.release(mem);
        let first = ((place - start) / PAGE) as usize;
        a.set_range(first, first + meta_pages, true);
        a.hint = a.find_next(0, a.total, false).unwrap_or(a.total);
        a
    }

    pub fn alloc_contiguous(&mut self, count: usize, align: usize) -> Option<PhysAddr> {
        assert!(count > 0 && align.is_power_of_two());
        let base = (self.start / PAGE) as usize;
        let mut i = self.hint;
        loop {
            let free = self.find_next(i, self.total, false)?;
            let first = (free + base).next_multiple_of(align) - base;
            if first + count > self.total {
                return None;
            }
            match self.find_next(first, first + count, true) {
                None => {
                    self.set_range(first, first + count, true);
                    if first == free {
                        self.hint = first + count;
                    }
                    return Some(PhysAddr {
                        addr: self.start + first as u64 * PAGE,
                        page: first as u64,
                    });
                }
                Some(used) => i = used + 1,
            }
        }
    }

    pub fn alloc_frame(&mut self) -> Option<PhysAddr> {
        self.alloc_contiguous(1, 1)
    }

    pub fn free_contiguous(&mut self, first: PhysAddr, count: usize) {
        let p = first.page as usize;
        assert!(
            count > 0 && p + count <= self.total,
            "free outside the managed range"
        );
        assert!(
            self.find_next(p, p + count, false).is_none(),
            "double free in frames {p}..{}",
            p + count
        );
        self.set_range(p, p + count, false);
        self.hint = self.hint.min(p);
    }

    pub fn free_frame(&mut self, f: PhysAddr) {
        self.free_contiguous(f, 1);
    }

    pub fn zero_frame(&mut self, f: &PhysAddr) -> Result<(), ()> {
        let p = f.page as usize;
        if p >= self.total || !self.get_bit(p) {
            return Err(()); // not an allocated frame
        }
        unsafe { core::ptr::write_bytes(phys_to_virt(f.addr), 0, PAGE as usize) };
        Ok(())
    }

    pub fn free_frames(&self) -> u64 {
        self.bitmap.iter().map(|w| w.count_zeros() as u64).sum()
    }

    pub fn meta_pages(&self) -> usize {
        self.meta_pages
    }

    pub fn address_to_page(&self, address: u64) -> u64 {
        (address - self.start) / PAGE
    }

    pub fn page_to_address(&self, page: u64) -> u64 {
        self.start + page * PAGE
    }
}
