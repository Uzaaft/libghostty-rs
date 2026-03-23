//! Query compile-time build configuration of libghostty-vt.
//!
//! These values reflect the options the library was built with and are constant for the lifetime of the process.
//!
//! # Example
//! ```rust
//! use ghostty::{Error, build_info::*};
//!
//! fn print_build_info() -> Result<(), Error> {
//!     println!("SIMD: {}", if supports_simd() { "enabled" } else { "disabled" });
//!     println!("Kitty graphics: {}", if supports_kitty_graphics() { "enabled" } else { "disabled" });
//!     println!("Tmux control mode: {}", if supports_tmux_control_mode() { "enabled" } else { "disabled" });
//!     Ok(())
//! }
//! ```

use std::mem::MaybeUninit;

use crate::{error::from_result, ffi};

/// Whether SIMD-accelerated code paths are enabled.
pub fn supports_simd() -> bool {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_SIMD)
}

/// Whether Kitty graphics protocol support is available.
pub fn supports_kitty_graphics() -> bool {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_KITTY_GRAPHICS)
}

/// Whether tmux control mode support is available.
pub fn supports_tmux_control_mode() -> bool {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_TMUX_CONTROL_MODE)
}

/// The optimization mode the library was built with.
pub fn optimization_mode() -> OptimizeMode {
    build_info::<ffi::GhosttyOptimizeMode>(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_OPTIMIZE).into()
}

fn build_info<T>(tag: ffi::GhosttyBuildInfo) -> T {
    let mut value = MaybeUninit::zeroed();
    let result = unsafe { ffi::ghostty_build_info(tag, std::ptr::from_mut(&mut value).cast()) };
    // Since we manually model every possible query, this should never fail.
    assert!(from_result(result).is_ok());
    // SAFETY: Value should be initialized after successful call.
    unsafe { value.assume_init() }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OptimizeMode {
    Debug,
    ReleaseSafe,
    ReleaseSmall,
    ReleaseFast,
}

impl From<ffi::GhosttyOptimizeMode> for OptimizeMode {
    fn from(value: ffi::GhosttyOptimizeMode) -> Self {
        match value {
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_DEBUG => Self::Debug,
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SAFE => Self::ReleaseSafe,
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SMALL => Self::ReleaseSmall,
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_FAST => Self::ReleaseFast,
            _ => unreachable!(),
        }
    }
}
