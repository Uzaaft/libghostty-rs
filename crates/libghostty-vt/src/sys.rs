//! Process-wide host services for targets that need to override platform defaults.

use crate::{
    error::{Error, Result},
    ffi,
};
use std::sync::RwLock;

/// Fill the entire buffer from a cryptographically secure source, returning false
/// if entropy is unavailable. Predictable sources must never be used: Ghostty
/// uses these bytes for protocol secrets and session grants.
pub type SecureRandomSource = fn(&mut [u8]) -> bool;

static RANDOM_SOURCE: RwLock<Option<SecureRandomSource>> = RwLock::new(None);

/// Override secure randomness, or restore the platform source with `None`.
/// Targets without platform entropy need an override for Kitty paste events.
///
/// # Safety
///
/// Register at startup before terminal functionality is used. The caller must
/// serialize registration with all Ghostty calls across every thread because
/// upstream system options are process-global and are not synchronized.
pub unsafe fn set_secure_random_source(source: Option<SecureRandomSource>) -> Result<()> {
    unsafe extern "C" fn callback(_: *mut std::ffi::c_void, ptr: *mut u8, len: usize) -> bool {
        let Ok(source) = RANDOM_SOURCE.read().map(|source| *source) else {
            return false;
        };
        let Some(source) = source else {
            return false;
        };
        // SAFETY: Ghostty lends a writable buffer for this call. Rust still
        // requires a non-null pointer for an empty slice, so handle it separately.
        let bytes = if len == 0 {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(ptr, len) }
        };
        source(bytes)
    }
    let callback: ffi::SysRandomSecureFn = source.map(|_| callback as _);
    *RANDOM_SOURCE.write().map_err(|_| Error::InvalidValue)? = source;
    crate::sys_set(
        ffi::SysOption::RANDOM_SECURE,
        callback.map_or(std::ptr::null(), |f| f as *const std::ffi::c_void),
    )
}
