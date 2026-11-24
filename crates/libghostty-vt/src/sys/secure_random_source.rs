use crate::ffi;

/// A source of secure random bytes.
///
/// See [`sys::SECURE_RANDOM_SOURCE`](SECURE_RANDOM_SOURCE) for more details.
pub trait SecureRandomSource: 'static + Sync {
    /// Fills the buffer with cryptographically secure random bytes. The library
    /// uses this for secrets, so it must be a real CSPRNG (`getrandom`,
    /// `arc4random_buf`, `BCryptGenRandom`, `crypto.getRandomValues`, ...); a
    /// predictable source is a security hole. Returns `true` if the buffer was
    /// filled, `false` if no entropy is available.
    fn fill(&self, buf: &mut [u8]) -> bool;
}

/// The custom secure random source.
///
/// By default libghostty draws secure random bytes from the platform
/// (`getrandom` or `arc4random_buf` on POSIX, CNG on Windows). Targets without
/// one, such as wasm32-freestanding, have no default, and operations that need
/// entropy fail with [`Error::IoError`](crate::Error::IoError) until a source
/// is set. When set, it is used instead of the platform source on every target.
///
/// This is a process-global setting. It is simplest to set it once at
/// startup, before using any terminal functionality that depends on it.
pub static SECURE_RANDOM_SOURCE: super::SysFunc<dyn SecureRandomSource> = super::SysFunc::new(
    ffi::SysOption::RANDOM_SECURE,
    trampoline as Trampoline as *const std::ffi::c_void,
);

type Trampoline = unsafe extern "C" fn(_: *mut std::ffi::c_void, ptr: *mut u8, len: usize) -> bool;

unsafe extern "C" fn trampoline(_: *mut std::ffi::c_void, ptr: *mut u8, len: usize) -> bool {
    let Some(source) = SECURE_RANDOM_SOURCE.get() else {
        return false;
    };
    let bytes: &mut [u8] = if len == 0 {
        // `slice::from_raw_parts_mut` needs a non-null pointer even for
        // an empty slice.
        &mut []
    } else {
        // SAFETY: libghostty lends `len` writable bytes for this call.
        // They may be uninitialized (the paste event password buffer is),
        // and the safe source is allowed to read them, so zero them
        // before handing them out.
        unsafe {
            ptr.write_bytes(0, len);
            std::slice::from_raw_parts_mut(ptr, len)
        }
    };
    source.fill(bytes)
}
