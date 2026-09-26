//! Process-wide host services, for targets that need to override platform
//! defaults.

use std::sync::{PoisonError, RwLock};

use crate::{error::Result, ffi};

/// A source of cryptographically secure random bytes.
///
/// It must fill the entire buffer and return `true`, or return `false` if no
/// entropy is available. libghostty uses these bytes for secrets, so it must be
/// a real CSPRNG (getrandom, arc4random_buf, BCryptGenRandom,
/// crypto.getRandomValues, ...); a predictable source is a security hole.
///
/// The buffer is zeroed before it is passed in.
pub type SecureRandomSource = fn(&mut [u8]) -> bool;

static RANDOM_SOURCE: RwLock<Option<SecureRandomSource>> = RwLock::new(None);

/// Override the secure random source, or restore the platform default with
/// `None`.
///
/// By default libghostty draws secure random bytes from the platform (getrandom
/// or arc4random_buf on POSIX, CNG on Windows). Targets without one, such as
/// wasm32-freestanding, have no default, and operations that need entropy fail
/// with [`Error::IoError`](crate::Error::IoError) until a source is set. When
/// set, it is used instead of the platform source on every target.
///
/// The source runs inside calls into libghostty, so a panic in it aborts the
/// process.
///
/// # Safety
///
/// This is a process-global setting that libghostty does not synchronize. Set
/// it once at startup, before using any terminal functionality that depends
/// on it, and serialize the call with all other libghostty calls on every
/// thread.
pub unsafe fn set_secure_random_source(source: Option<SecureRandomSource>) -> Result<()> {
    unsafe extern "C" fn callback(_: *mut std::ffi::c_void, ptr: *mut u8, len: usize) -> bool {
        // The stored value is a plain function pointer, so a panic elsewhere
        // cannot have left it half-updated.
        let Some(source) = *RANDOM_SOURCE.read().unwrap_or_else(PoisonError::into_inner) else {
            return false;
        };
        let bytes: &mut [u8] = if len == 0 {
            // `slice::from_raw_parts_mut` needs a non-null pointer even for
            // an empty slice.
            &mut []
        } else {
            // SAFETY: libghostty lends `len` writable bytes for this call.
            // They may be uninitialized, and the safe source is allowed to
            // read them, so zero them before handing them out.
            unsafe {
                ptr.write_bytes(0, len);
                std::slice::from_raw_parts_mut(ptr, len)
            }
        };
        source(bytes)
    }

    let callback: ffi::SysRandomSecureFn = source.map(|_| callback as _);
    *RANDOM_SOURCE
        .write()
        .unwrap_or_else(PoisonError::into_inner) = source;
    crate::sys_set(
        ffi::SysOption::RANDOM_SECURE,
        callback.map_or(std::ptr::null(), |f| f as *const std::ffi::c_void),
    )
}
