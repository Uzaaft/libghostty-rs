use std::{marker::PhantomData, ptr::NonNull};

use crate::{
    alloc::Allocator,
    error::{Error, from_result},
    ffi,
};

pub struct Parser<'alloc> {
    ptr: NonNull<ffi::GhosttyOscParser>,
    _phan: PhantomData<&'alloc ffi::GhosttyAllocator>,
}

impl<'alloc> Parser<'alloc> {
    /// Create a new OSC parser.
    pub fn new() -> Result<Self, Error> {
        Self::new_with_alloc::<()>(None)
    }

    /// Create a new OSC parser with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(
        alloc: Option<&'alloc Allocator<'ctx, Ctx>>,
    ) -> Result<Self, Error> {
        let mut raw: ffi::GhosttyOscParser_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_osc_new(Allocator::to_c_ptr(alloc), &mut raw) };
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
