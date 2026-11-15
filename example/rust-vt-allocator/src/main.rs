use libghostty_vt::{alloc::Allocator, error::Error, Terminal};
use std::{alloc, cell::Cell, ptr::NonNull};

// The allocator's state. This example only counts, so it can check at
// the end that the terminal freed everything it allocated.
struct Stats {
    allocs: Cell<usize>,
    live_bytes: Cell<usize>,
}

// ANCHOR: allocator-vtable
unsafe impl alloc::Allocator for Stats {
    fn allocate(&self, layout: alloc::Layout) -> Result<NonNull<[u8]>, alloc::AllocError> {
        let allocated = alloc::Global.allocate(layout)?;
        self.allocs.update(|x| x + 1);
        self.live_bytes.update(|b| b + layout.size());
        Ok(allocated)
    }
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: alloc::Layout) {
        unsafe {
            alloc::Global.deallocate(ptr, layout);
        }
        self.allocs.update(|x| x - 1);
        self.live_bytes.update(|b| b - layout.size());
    }
}
// ANCHOR_END: allocator-vtable

fn main() -> Result<(), Error> {
    // ANCHOR: allocator-use
    let stats = Stats {
        allocs: Cell::new(0),
        live_bytes: Cell::new(0),
    };
    // Adapt a Rust allocator into a libghostty allocator.
    let allocator = Allocator::from(&stats);

    // The terminal keeps a pointer to the allocator,
    // so `allocator` must outlive it.
    let mut terminal = Terminal::new_with_alloc(&allocator, 80, 24)?;
    // ANCHOR_END: allocator-use

    // Write enough styled output to fill the scrollback,
    // so the terminal has to grow its allocations.
    for i in 0..20000 {
        let line = format!("\x1b[3{}mline {i} with some text\x1b[0m\r\n", i % 8);
        terminal.vt_write(line.as_bytes());
    }

    println!("Allocations: {}", stats.allocs.get());
    println!("Live bytes before free: {}", stats.live_bytes.get());

    drop(terminal);
    println!("Live bytes after free: {}", stats.live_bytes.get());
    assert_eq!(stats.live_bytes.get(), 0);

    Ok(())
}
