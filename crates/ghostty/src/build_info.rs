//! Query compile-time build configuration of libghostty-vt.
//!
//! These values reflect the options the library was built with and are constant for the lifetime of the process.
//!
//! # Example
//! ```rust
//! use ghostty::{Error, build_info::*};
//!
//! fn print_build_info() -> Result<(), Error> {
//!     println!(
//!         "SIMD: {}",
//!         if supports_simd().unwrap_or(false) { "enabled" } else { "disabled" }
//!     );
//!     println!(
//!         "Kitty graphics: {}",
//!         if supports_kitty_graphics().unwrap_or(false) { "enabled" } else { "disabled" }
//!     );
//!     println!(
//!         "Tmux control mode: {}",
//!         if supports_tmux_control_mode().unwrap_or(false) { "enabled" } else { "disabled" }
//!     );
//!     Ok(())
//! }
//! ```

use std::mem::MaybeUninit;

use crate::{
    error::{Error, Result, from_result},
    ffi,
};

/// Whether SIMD-accelerated code paths are enabled.
pub fn supports_simd() -> Result<bool> {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_SIMD)
}

/// Whether Kitty graphics protocol support is available.
pub fn supports_kitty_graphics() -> Result<bool> {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_KITTY_GRAPHICS)
}

/// Whether tmux control mode support is available.
pub fn supports_tmux_control_mode() -> Result<bool> {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_TMUX_CONTROL_MODE)
}

/// The optimization mode the library was built with.
pub fn optimization_mode() -> Result<OptimizeMode> {
    build_info::<ffi::GhosttyOptimizeMode>(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_OPTIMIZE)
        .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
}

fn build_info<T>(tag: ffi::GhosttyBuildInfo) -> Result<T> {
    let mut value = MaybeUninit::zeroed();
    let result = unsafe { ffi::ghostty_build_info(tag, std::ptr::from_mut(&mut value).cast()) };
    from_result(result)?;
    // SAFETY: Value should be initialized after successful call.
    unsafe { value.assume_init() }
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum OptimizeMode {
    Debug = ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_DEBUG,
    ReleaseSafe = ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SAFE,
    ReleaseSmall = ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SMALL,
    ReleaseFast = ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_FAST,
}
