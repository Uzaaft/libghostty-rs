//! Parsing and handling OSC (Operating System Command) escape sequences.

use std::{marker::PhantomData, ptr::NonNull};

use crate::{
    alloc::Allocator,
    error::{Error, from_result},
    ffi,
};

/// OSC (Operating System Command) sequence parser and command handling.
///
/// The parser operates in a streaming fashion, processing input byte-by-byte to handle OSC sequences
/// that may arrive in fragments across multiple reads. This interface makes it easy to integrate
/// into most environments and avoids over-allocating buffers.
pub struct Parser<'alloc> {
    ptr: NonNull<ffi::GhosttyOscParser>,
    _phan: PhantomData<&'alloc ffi::GhosttyAllocator>,
}

impl<'alloc> Parser<'alloc> {
    /// Create a new OSC parser.
    pub fn new() -> Result<Self, Error> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new OSC parser with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(
        alloc: &'alloc Allocator<'ctx, Ctx>,
    ) -> Result<Self, Error> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self, Error> {
        let mut raw: ffi::GhosttyOscParser_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_osc_new(alloc, &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _phan: PhantomData,
        })
    }

    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_osc_reset(self.ptr.as_ptr()) }
    }

    pub fn next_byte(&mut self, byte: u8) {
        unsafe { ffi::ghostty_osc_next(self.ptr.as_ptr(), byte) }
    }

    pub fn end<'p>(&'p mut self, terminator: u8) -> Command<'p, 'alloc> {
        let raw = unsafe { ffi::ghostty_osc_end(self.ptr.as_ptr(), terminator) };
        Command {
            ptr: raw,
            _parser: PhantomData,
        }
    }
}

impl Drop for Parser<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_osc_free(self.ptr.as_ptr()) }
    }
}

pub struct Command<'p, 'alloc> {
    ptr: ffi::GhosttyOscCommand_ptr,
    _parser: PhantomData<&'p Parser<'alloc>>,
}

impl Command<'_, '_> {
    pub fn command_type(&self) -> ffi::GhosttyOscCommandType {
        unsafe { ffi::ghostty_osc_command_type(self.ptr) }
    }
}
