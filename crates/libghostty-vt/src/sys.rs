//! Plug in your own system functions.
//!
//! The system interface is a set of process-global function pointers for
//! things the library can't do by itself.
//!
//! Setting one enables the feature that depends on it. For example, setting
//! [a PNG decoder](PNG_DECODER) enables PNG image support in the Kitty
//! Graphics Protocol. Do this before calling any other function in the library.
//! See [`SysFunc::install`] for a full breakdown of our safety guarantees.
use crate::{
    error::{Error, Result, from_result},
    ffi,
};
use std::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

static GATE: Gate = Gate(AtomicU8::new(Gate::OPEN));

pub(crate) struct Gate(AtomicU8);

impl Gate {
    const OPEN: u8 = 0;
    const INSTALLING: u8 = 1;
    const FROZEN: u8 = 2;

    /// Freeze all handlers, preventing them from being modified.
    ///
    /// This is critical for our safety guarantees against data races, since
    /// libghostty-vt does not provide any synchronization, and you can easily
    /// footgun yourself by arbitrarily deciding to swap out any system function
    /// while libghostty-vt functions are already running, potentially also on
    /// a different thread.
    ///
    /// Since the intention for system functions is that they should be installed
    /// once upon startup, it's a reasonable compromise then to freeze all handlers
    /// once any libghostty-vt function is called. This is arguably too strict,
    /// but Rust is built upon conservatively rejecting programs that *might* be
    /// safe depending on the circumstances anyway...
    pub(crate) fn freeze(&self) {
        if self.0.load(Ordering::Acquire) == Self::FROZEN {
            return; // hot path: one load
        }
        loop {
            match self.0.compare_exchange(
                Self::OPEN,
                Self::FROZEN,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) | Err(Self::FROZEN) => return,
                // An install is mid-flight; the C side can't be entered while
                // set() runs, so wait for it to finish.
                Err(_) => core::hint::spin_loop(),
            }
        }
    }

    fn guard(&self) -> Result<InstallGuard<'_>> {
        match self.0.compare_exchange(
            Self::OPEN,
            Self::INSTALLING,
            Ordering::Acquire,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(InstallGuard { gate: self }),
            Err(_) => Err(Error::InvalidValue),
        }
    }
}

struct InstallGuard<'g> {
    gate: &'g Gate,
}

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.gate.0.store(Gate::OPEN, Ordering::Release);
    }
}

/// An installable global system function.
///
/// The system interface is a set of process-global function pointers for
/// things the library can't do by itself.
///
/// For most users, simply call [`install`](Self::install) at the start of the
/// program with a corresponding handler. This must be done before any
/// libghostty-vt functions are called.
#[derive(Debug)]
pub struct SysFunc<H: ?Sized + Sync + 'static> {
    installed: AtomicBool,
    handler: UnsafeCell<Option<&'static H>>,
    key: ffi::SysOption::Type,
    trampoline: *const std::ffi::c_void,
}

// SAFETY: We gate access to the handler behind both the FROZEN flag and
// the state variable, both of which are atomic. Data races should not
// occur unless the user explicitly uses an `_unsafe` function to access
// the handler, which means they bear the responsibility to use it correctly.
unsafe impl<H: ?Sized + Sync + 'static> Sync for SysFunc<H> {}

impl<H: ?Sized + Sync + 'static> SysFunc<H> {
    const fn new(key: ffi::SysOption::Type, trampoline: *const std::ffi::c_void) -> Self {
        Self {
            installed: AtomicBool::new(false),
            handler: UnsafeCell::new(None),
            key,
            trampoline,
        }
    }

    fn get(&self) -> Option<&'static H> {
        if !self.installed.load(Ordering::Acquire) {
            return None;
        }
        unsafe { *self.handler.get() }
    }

    /// Install the given handler as a global system function.
    ///
    /// Due to safety concerns in the C library, it is only possible to install
    /// a handler **once** safely, before *any* libghostty-vt function is called.
    /// This includes constructing libghostty-vt types. This is checked at runtime
    /// to ensure no Undefined Behavior or data races can occur.
    ///
    /// If you need esoteric use cases such as hotswapping handlers or installing
    /// handlers after libghostty-vt has already been initialized, you must be
    /// absolutely sure what you're doing conforms to our safety guarantees.
    /// In this case you may use [`install_unsafe`](Self::install_unsafe) instead.
    ///
    /// # Safety
    ///
    /// Handlers call directly into libghostty-vt's C API, and so are **not**
    /// panic-safe. Any panic will terminate the entire process.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidValue`] if one handler has already been installed,
    /// other libghostty-vt functions have already been called. These are safety
    /// bugs that must be fixed by the library user.
    ///
    /// Returns [`Error::Unsupported`] if the handler type is not known to the
    /// C library at runtime.
    pub fn install(&self, h: &'static H) -> Result<()> {
        self.install_with(|| h)
    }

    /// Install the handler returned by the closure as a global system function.
    ///
    /// Compared to [`install`](Self::install), this is mainly useful for
    /// situations where it's not possible or convenient to immediately obtain
    /// a static reference to the handler, for example when the handler requires
    /// extra state and has to be boxed:
    ///
    /// ```rust,ignore
    /// use libghostty::sys;
    ///
    /// sys::LOGGER.install_with(|| Box::leak(Box::new(my_logger)))?;
    /// ```
    ///
    /// The closure is not invoked if installation fails.
    ///
    /// # Safety
    ///
    /// Handlers call directly into libghostty-vt's C API, and so are **not**
    /// panic-safe. Any panic will terminate the entire process.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidValue`] if one handler has already been installed,
    /// other libghostty-vt functions have already been called. These are safety
    /// bugs that must be fixed by the library user.
    ///
    /// Returns [`Error::Unsupported`] if the handler type is not known to the
    /// C library at runtime.
    pub fn install_with(&self, h: impl FnOnce() -> &'static H) -> Result<()> {
        let _gate = GATE.guard();
        if self.installed.load(Ordering::Acquire) {
            return Err(Error::InvalidValue);
        }

        unsafe { self.install_unsafe_with(h) }
    }

    /// Install the given handler as a global system function, **ignoring all
    /// safety guarantees**.
    ///
    /// # Safety
    ///
    /// This is a very dangerous function to call. You must be absolutely sure
    /// what you're doing is safe, which includes the following criteria:
    ///
    ///   - No data race could occur when installing the handler. This means
    ///     you need to prevent cases where two or more threads try to access
    ///     or write the handler simultaneously. This is unsafe even in the C API.
    ///   - Handler registration must not be re-entrant. You must not register
    ///     a handler while the same handler is currently being run.
    ///   - You must ensure that the handler is safe to call from multiple
    ///     threads (i.e. [`Sync`]). This is normally guaranteed by the type
    ///     system but is worth reiterating.
    ///
    /// In return, you're allowed to:
    ///
    ///   - Replace existing handlers. Note that the old handler's resources will
    ///     **not** be cleaned up. For example, if the old handler was created
    ///     by [leaking](Box::leak) a [`Box`], the memory will be leaked forever.
    ///     Any [`Drop`] implementation would also not be called.
    ///   - Install handlers after libghostty-vt functions have already been
    ///     called.
    ///   - Install handlers on architectures without compare-and-swap (CAS)
    ///     instructions necessary to maintain our atomic checks.
    ///
    /// Handlers call directly into libghostty-vt's C API, and so are **not**
    /// panic-safe. Any panic will terminate the entire process.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if the handler type is not known to the
    /// C library at runtime.
    pub unsafe fn install_unsafe(&self, h: &'static H) -> Result<()> {
        // SAFETY: Caller has sworn a blood oath that this will be safe
        unsafe { self.install_unsafe_with(|| h) }
    }

    /// Install the handler returned by the closure as a global system function,
    /// **ignoring all safety guarantees**.
    ///
    /// Compared to [`install_unsafe`](Self::install_unsafe), this is mainly useful for
    /// situations where it's not possible or convenient to immediately obtain
    /// a static reference to the handler, for example when the handler requires
    /// extra state and has to be boxed:
    ///
    /// ```rust,ignore
    /// use libghostty::sys;
    ///
    /// unsafe {
    ///     sys::LOGGER.install_unsafe_with(|| Box::leak(Box::new(my_logger)))?;
    /// }
    /// ```
    ///
    /// The closure is not invoked if installation fails.
    ///
    /// # Safety
    ///
    /// This function is very dangerous if misused. See
    /// [`install_unsafe`](Self::install_unsafe).
    ///
    /// Handlers call directly into libghostty-vt's C API, and so are **not**
    /// panic-safe. Any panic will terminate the entire process.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if the handler type is not known to the
    /// C library at runtime.
    pub unsafe fn install_unsafe_with(&self, h: impl FnOnce() -> &'static H) -> Result<()> {
        from_result(unsafe { ffi::ghostty_sys_set(self.key, self.trampoline) })?;
        unsafe { *self.handler.get() = Some(h()) };
        self.installed.store(true, Ordering::Release);
        Ok(())
    }

    /// Uninstall the handler if one is currently installed,
    /// **ignoring all safety guarantees**.
    ///
    /// # Safety
    ///
    /// Handlers are designed to be installed once at startup before any
    /// libghostty-vt function runs, so uninstalling handlers is very rare
    /// and also hard to implement safely. Therefore this is only available
    /// as an unsafe function with the same safety requirements as
    /// [`install_unsafe`](Self::install_unsafe).
    ///
    /// Removing an existing handler also does not release any resources
    /// associated with the handler, such as [leaking](Box::leak)
    /// a [`Box`] via [`install_with`](Self::install_with) or
    /// [`install_unsafe_with`](Self::install_unsafe_with).
    /// Any [`Drop`] implementation would also not be called.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] if the handler type is not known to the
    /// C library at runtime.
    pub unsafe fn uninstall(&self) -> Result<()> {
        from_result(unsafe { ffi::ghostty_sys_set(self.key, std::ptr::null()) })?;
        unsafe { *self.handler.get() = None };
        self.installed.store(false, Ordering::Release);
        Ok(())
    }
}

mod log;
pub use log::*;

mod png;
pub use png::*;

mod secure_random_source;
pub use secure_random_source::*;
