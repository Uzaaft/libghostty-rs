//! Adapters from standard library IO traits to libghostty objects.
use std::io::{Read, Write};

use crate::ffi;

/// Adapt a [`std::io::Write`] to a libghostty-friendly Writer object.
pub fn to_writer<W: Write>(w: &mut W) -> ffi::Writer {
    unsafe extern "C" fn trampoline<W: Write>(
        userdata: *mut ::std::os::raw::c_void,
        data: *const u8,
        len: usize,
    ) -> bool {
        // SAFETY: This trampoline should be inaccessible outside
        // of the writer interface, so it should be safe to assume
        // the userdata is the writer we need
        let w: &mut W = unsafe { &mut *userdata.cast::<W>() };

        // SAFETY: We trust libghostty to give us valid data
        let data = unsafe { std::slice::from_raw_parts(data, len) };

        w.write_all(data).is_ok()
    }

    ffi::Writer {
        userdata: std::ptr::from_mut(w).cast(),
        write: Some(trampoline::<W>),
    }
}

/// Adapt a [`std::io::Read`] to a libghostty-friendly Reader object.
pub fn to_reader<R: Read>(r: &mut R) -> ffi::Reader {
    unsafe extern "C" fn trampoline<R: Read>(
        userdata: *mut ::std::os::raw::c_void,
        buffer: *mut u8,
        capacity: usize,
        out_read: *mut usize,
    ) -> bool {
        // SAFETY: This trampoline should be inaccessible outside
        // of the writer interface, so it should be safe to assume
        // the userdata is the writer we need
        let r: &mut R = unsafe { &mut *userdata.cast::<R>() };

        // SAFETY: We trust libghostty to give us valid data
        let buf = unsafe { std::slice::from_raw_parts_mut(buffer, capacity) };

        match r.read(buf) {
            // SAFETY: Ditto
            Ok(len) => unsafe {
                *out_read = len;
                true
            },
            Err(_) => false,
        }
    }

    ffi::Reader {
        userdata: std::ptr::from_mut(r).cast(),
        read: Some(trampoline::<R>),
    }
}
