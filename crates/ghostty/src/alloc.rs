//! Adapting custom allocators to work with libghostty.
use std::{ffi::c_void, marker::PhantomData};

#[cfg(feature = "allocator_api")]
use allocator_api2::alloc;

use crate::ffi::{GhosttyAllocator, GhosttyAllocatorVtable};

/// A custom allocator that libghostty uses for its memory allocations.
///
/// The allocator may depend on some external state `Ctx` for the
/// duration of lifetime `'ctx`. This is useful for adapting external,
/// stateful allocators that may not have a `'static` lifetime.
///
/// One example of a custom allocator that *does* have a `'static`
/// lifetime is Rust's own default allocator, which can also be used
/// within libghostty as `Self::GLOBAL`.
pub struct Allocator<'ctx, Ctx: 'ctx = ()> {
    pub(crate) inner: GhosttyAllocator,
    _phan: PhantomData<&'ctx Ctx>,
}

impl<'alloc, 'ctx: 'alloc, Ctx> Allocator<'ctx, Ctx> {
    pub(crate) fn to_raw(&self) -> *const GhosttyAllocator {
        std::ptr::from_ref(&self.inner)
    }
}

//------------------------------------
// GlobalAlloc
//------------------------------------

impl Allocator<'static> {
    /// A custom allocator based on Rust's built-in
    /// [global allocator](std::alloc::GlobalAlloc).
    pub const GLOBAL: Self = Self {
        inner: GhosttyAllocator {
            ctx: std::ptr::null_mut(),
            vtable: &GhosttyAllocatorVtable {
                alloc: Some(_global_alloc),
                free: Some(_global_free),
                resize: Some(_global_resize),
                remap: Some(_global_remap),
            },
        },
        _phan: PhantomData,
    };
}

unsafe extern "C" fn _global_alloc(
    _allocator: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) -> *mut c_void {
    let Ok(layout) = std::alloc::Layout::from_size_align(len, 1 << alignment) else {
        return std::ptr::null_mut();
    };
    unsafe { std::alloc::alloc(layout).cast::<c_void>() }
}

unsafe extern "C" fn _global_free(
    _allocator: *mut c_void,
    mem: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) {
    let Ok(layout) = std::alloc::Layout::from_size_align(len, 1 << alignment) else {
        return;
    };
    unsafe { std::alloc::dealloc(mem.cast::<u8>(), layout) }
}
unsafe extern "C" fn _global_resize(
    _allocator: *mut c_void,
    _mem: *mut c_void,
    _old_len: usize,
    _alignment: u8,
    _new_len: usize,
    _ret_addr: usize,
) -> bool {
    false
}
unsafe extern "C" fn _global_remap(
    _allocator: *mut c_void,
    mem: *mut c_void,
    old_len: usize,
    alignment: u8,
    new_len: usize,
    _ret_addr: usize,
) -> *mut c_void {
    let Ok(layout) = std::alloc::Layout::from_size_align(old_len, 1 << alignment) else {
        return std::ptr::null_mut();
    };
    unsafe { std::alloc::realloc(mem.cast::<u8>(), layout, new_len).cast::<c_void>() }
}

//------------------------------------
// Allocator API
//------------------------------------

/// Adapt a Rust Allocator into a libghostty Allocator.
#[cfg(feature = "allocator_api")]
impl<'ctx, A: alloc::Allocator + 'ctx> From<A> for Allocator<'ctx, A> {
    fn from(value: A) -> Self {
        Self {
            inner: GhosttyAllocator {
                ctx: std::ptr::from_ref(value.by_ref()) as *mut std::ffi::c_void,
                vtable: &GhosttyAllocatorVtable {
                    alloc: Some(_alloc::<A>),
                    free: Some(_free::<A>),
                    resize: Some(_resize),
                    remap: Some(_remap::<A>),
                },
            },
            _phan: PhantomData,
        }
    }
}

#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _alloc<A: alloc::Allocator>(
    allocator: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) -> *mut c_void {
    let layout = alloc::Layout::from_size_align(len, 1 << alignment).ok();

    unsafe { get_allocator::<A>(allocator) }
        .and_then(|alloc| alloc.allocate(layout?).ok())
        .map(|p| p.as_ptr().cast::<c_void>())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _free<A: alloc::Allocator>(
    allocator: *mut c_void,
    mem: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) {
    let Some(mem) = NonNull::new(mem.cast::<u8>()) else {
        return;
    };
    let Some(layout) = alloc::Layout::from_size_align(len, 1 << alignment).ok() else {
        return;
    };
    if let Some(alloc) = unsafe { get_allocator::<A>(allocator) } {
        unsafe { alloc.deallocate(mem, layout) };
    }
}

/// Resize (grow or shrink) an allocation *in-place*.
///
/// Rather unfortunately, Rust's Allocator API does not guarantee that
/// growing or shrinking an allocation would necessarily be in-place.
/// Therefore, we have to assume rather pessimistically that every
/// resizing operation might relocate the memory block, so in-place
/// resizes are always impossible.
#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _resize(
    _allocator: *mut c_void,
    _mem: *mut c_void,
    _old_len: usize,
    _alignment: u8,
    _new_len: usize,
    _ret_addr: usize,
) -> bool {
    false
}

/// Resize (grow or shrink) an allocation, *allowing relocation if necessary*,
/// returning `null` if resizing requires reallocation.
#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _remap<A: alloc::Allocator>(
    allocator: *mut c_void,
    mem: *mut c_void,
    old_len: usize,
    alignment: u8,
    new_len: usize,
    _ret_addr: usize,
) -> *mut c_void {
    let mem = NonNull::new(mem.cast::<u8>());
    let old_layout = alloc::Layout::from_size_align(old_len, 1 << alignment).ok();
    let new_layout = alloc::Layout::from_size_align(new_len, 1 << alignment).ok();

    unsafe { get_allocator::<A>(allocator) }
        .and_then(|alloc| {
            if new_len < old_len {
                unsafe { alloc.shrink(mem?, old_layout?, new_layout?) }.ok()
            } else {
                unsafe { alloc.grow(mem?, old_layout?, new_layout?) }.ok()
            }
        })
        .map(|p| p.as_ptr().cast::<c_void>())
        .unwrap_or(std::ptr::null_mut())
}

/// Get the allocator back from a vtable function.
///
/// # Safety
///
/// This function only behaves correctly if called by one of the vtable functions.
/// In particular, it expects the vtable function to be used correctly, which means
/// libghostty must have received a valid allocator object from elsewhere in this
/// crate. If any of these preconditions are unmet, this will definitely cause
/// Undefined Behavior.
///
/// The returned allocator must **never** be smuggled outside the lifetime of the caller.
#[inline(always)]
#[cfg(feature = "allocator_api")]
unsafe fn get_allocator<'a, A: alloc::Allocator>(ptr: *mut c_void) -> Option<&'a A> {
    unsafe { ptr.cast::<A>().as_ref() }
}
