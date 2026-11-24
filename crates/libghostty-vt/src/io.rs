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

        // SAFETY: libghostty supplies non-NULL writable storage for capacity
        // bytes. It may be uninitialized, and safe Read implementations may
        // inspect the entire slice, so initialize it before creating the slice.
        let buf = unsafe {
            buffer.write_bytes(0, capacity);
            std::slice::from_raw_parts_mut(buffer, capacity)
        };

        match r.read(buf) {
            // SAFETY: libghostty supplies a writable out parameter.
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

#[cfg(test)]
mod soundness_tests {
    use super::*;

    #[test]
    fn reader_can_inspect_the_entire_destination() {
        struct InspectingReader;
        impl Read for InspectingReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                // A safe Read implementation may inspect bytes it does not
                // overwrite, even when it returns fewer bytes than capacity.
                assert!(buffer.iter().all(|&byte| byte == 0));
                buffer[..2].copy_from_slice(&[0x31, 0x8b]);
                Ok(2)
            }
        }

        let mut reader = InspectingReader;
        let callback = to_reader(&mut reader);
        // Native runs use nonzero bytes for a deterministic failure. Miri
        // additionally checks the original uninitialized-buffer regression.
        let mut storage = if cfg!(miri) {
            std::mem::MaybeUninit::<[u8; 9]>::uninit()
        } else {
            std::mem::MaybeUninit::new([0xa5; 9])
        };
        for _ in 0..2 {
            let mut read = usize::MAX;
            // SAFETY: Model the native callback contract: live userdata,
            // writable non-NULL storage, positive capacity and an out pointer.
            assert!(unsafe {
                callback.read.unwrap()(
                    callback.userdata,
                    storage.as_mut_ptr().cast(),
                    9,
                    &raw mut read,
                )
            });
            assert_eq!(read, 2);
            // SAFETY: The reader adapter initializes all nine bytes, not
            // just the two bytes reported by the Read implementation.
            let bytes = unsafe { storage.assume_init_mut() };
            assert_eq!(*bytes, [0x31, 0x8b, 0, 0, 0, 0, 0, 0, 0]);
            // Reused native buffers may contain old bytes. Initialization must
            // happen on every callback, not just on the first read.
            bytes.fill(0xa5);
        }
    }
}
