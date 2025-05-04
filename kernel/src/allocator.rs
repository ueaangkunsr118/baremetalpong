#[global_allocator]
static ALLOCATOR: DummyAllocator = DummyAllocator;

use alloc::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use core::fmt::Write;

use crate::serial;
pub struct DummyAllocator;

pub static mut HEAP_START: usize = 0x0;
pub static mut OFFSET: usize = 0x0;
pub const HEAP_SIZE: usize = 100 * 1024; // 100 KiB

unsafe impl GlobalAlloc for DummyAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            // Calculate the next aligned address
            let align = layout.align();
            let size = layout.size();
            
            let current = HEAP_START + OFFSET;
            let aligned = (current + align - 1) & !(align - 1);
            let new_offset = (aligned - HEAP_START) + size;
            
            // Check if we have enough space
            if new_offset > HEAP_SIZE {
                return null_mut();
            }
            
            // Update the offset
            OFFSET = new_offset;
            
            aligned as *mut u8
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        writeln!(serial(), "dealloc was called at {_ptr:?}").unwrap();
        // Note: Bump allocator doesn't actually free memory
    }
}

pub fn init_heap(offset: usize) {
    unsafe {
        HEAP_START = offset;
        OFFSET = 0;
    }
}