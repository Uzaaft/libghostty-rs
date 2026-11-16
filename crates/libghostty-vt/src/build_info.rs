//! Find out how this copy of the library was built and which features it has.
//!
//! Build info is what this copy of the library knows about how it was compiled:
//! its version, its optimization mode, and which features it has.
//!
//! You need this when your program has to adapt to the library it loaded, such
//! as checking for Kitty graphics before you offer images. You don't need a
//! terminal. These values are constant for the lifetime of the process, so you
//! only have to read them once.
//!
//! To choose what a build contains, see [Optional Features].
//!
#![doc = crate::book_ref!("Optional Features", "features")]
//!
//! # Example
//! ```rust
//! use libghostty_vt::{build_info, error::Error};
//!
//! fn query_build_info() -> Result<(), Error> {
//!     println!("SIMD: {}", build_info::supports_simd()?);
//!     println!("Kitty graphics: {}", build_info::supports_kitty_graphics()?);
//!     println!(
//!         "Tmux control mode: {}",
//!         build_info::supports_tmux_control_mode()?
//!     );
//!
//!     println!("Version: {:?}", build_info::version_string());
//!     println!("Version major: {}", build_info::major_version()?);
//!     println!("Version minor: {}", build_info::minor_version()?);
//!     println!("Version patch: {}", build_info::patch_version()?);
//!
//!     let pre_version = build_info::pre_version()?;
//!     if !pre_version.is_empty() {
//!         println!("Version pre  : {}", pre_version.escape_ascii());
//!     } else {
//!         println!("Version pre  : (none)");
//!     }
//!
//!     let build_version = build_info::build_version()?;
//!     if !build_version.is_empty() {
//!         println!("Version build: {}", build_version.escape_ascii());
//!     } else {
//!         println!("Version build: (none)");
//!     }
//!     Ok(())
//! }
//! ```

use std::{ffi::CStr, mem::MaybeUninit};

use crate::{
    error::{Error, Result, from_result},
    ffi::{self, BuildInfo as Info},
};

/// Whether SIMD-accelerated code paths are enabled.
pub fn supports_simd() -> Result<bool> {
    build_info(Info::SIMD)
}

/// Whether this build includes the `kitty-graphics` feature, which provides
/// the Kitty graphics protocol and the [`crate::kitty::graphics`] module.
///
/// It is always false on targets without an operating system, such as
/// `wasm32-freestanding`. See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_kitty_graphics() -> Result<bool> {
    build_info(Info::KITTY_GRAPHICS)
}

/// Whether tmux control mode support is available.
pub fn supports_tmux_control_mode() -> Result<bool> {
    build_info(Info::TMUX_CONTROL_MODE)
}

/// Whether snapshot encoding and decoding are available.
pub fn supports_snapshot() -> Result<bool> {
    build_info(Info::FEATURE_SNAPSHOT)
}

/// Whether this build includes the `formatter` feature, which provides the
/// [`crate::fmt`] module.
///
/// See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_formatter() -> Result<bool> {
    build_info(Info::FEATURE_FORMATTER)
}

/// Whether this build includes the `selection` feature, which provides the
/// `Terminal::select_*` methods and the [`crate::selection`] module.
///
/// See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_selection() -> Result<bool> {
    build_info(Info::FEATURE_SELECTION)
}

/// Whether this build includes the `search` feature, which provides the
/// [`crate::search`] module.
///
/// See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_search() -> Result<bool> {
    build_info(Info::FEATURE_SEARCH)
}

/// Whether this build includes the `render-state` feature, which provides
/// the [`crate::render`] module.
///
/// See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_render_state() -> Result<bool> {
    build_info(Info::FEATURE_RENDER_STATE)
}

/// Whether this build includes the `input_encode` feature, which provides the
/// key, mouse, focus and paste encoding functions.
///
/// See [Optional Features] for the full list.
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_input_encode() -> Result<bool> {
    build_info(Info::FEATURE_INPUT_ENCODE)
}

/// Whether this build includes the `color` feature, which provides the
/// [`crate::color`] module.
///
/// See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_color() -> Result<bool> {
    build_info(Info::FEATURE_COLOR)
}

/// Whether grid references, cells and rows can be inspected.
///
/// See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_grid_introspection() -> Result<bool> {
    build_info(Info::FEATURE_GRID_INTROSPECTION)
}

/// Whether this build includes the `glyph-protocol` feature, which lets
/// applications register custom glyphs with Ghostty's glyph protocol.
///
/// It has no functions of its own. See [Optional Features].
///
#[doc = crate::book_ref!("Optional Features", "features")]
pub fn supports_glyph_protocol() -> Result<bool> {
    build_info(Info::FEATURE_GLYPH_PROTOCOL)
}

/// The optimization mode the library was built with.
pub fn optimize_mode() -> Result<OptimizeMode> {
    build_info::<ffi::OptimizeMode::Type>(Info::OPTIMIZE)
        .and_then(|v| v.try_into().map_err(|_| Error::Unsupported))
}

/// The full version string (e.g. "1.2.3" or "1.2.3-dev+abcdef").
///
/// # Example
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let version = libghostty_vt::build_info::version_string();
/// println!("libghostty-vt {:?}", version);
/// # Ok(())
/// # }
/// ```
#[must_use]
pub fn version_string() -> &'static CStr {
    // SAFETY: libghostty promises to return a NUL-terminated string
    // that is always valid.
    unsafe { CStr::from_ptr(ffi::ghostty_version()) }
}
/// The major version number.
pub fn major_version() -> Result<usize> {
    build_info(Info::VERSION_MAJOR)
}
/// The minor version number.
pub fn minor_version() -> Result<usize> {
    build_info(Info::VERSION_MINOR)
}
/// The patch version number.
pub fn patch_version() -> Result<usize> {
    build_info(Info::VERSION_PATCH)
}
/// The pre metadata string (e.g. "alpha", "beta", "dev").
///
/// Has zero length if no pre metadata is present.
///
/// The bytes are not guaranteed to be valid UTF-8.
pub fn pre_version() -> Result<&'static [u8]> {
    build_string(Info::VERSION_PRE)
}
/// The build metadata string (e.g. commit hash).
///
/// Has zero length if no build metadata is present.
///
/// The bytes are not guaranteed to be valid UTF-8.
pub fn build_version() -> Result<&'static [u8]> {
    build_string(Info::VERSION_BUILD)
}

/// Query one of the `GhosttyString` build info fields as raw bytes.
fn build_string(tag: ffi::BuildInfo::Type) -> Result<&'static [u8]> {
    build_info::<ffi::String>(tag)
        // SAFETY: libghostty fills these strings from its comptime build
        // options (`terminal_options`), or from a `""` literal when the
        // pre/build metadata is absent. Both live in the library's static
        // data, so they're valid for `'static`. Upstream makes no UTF-8 or
        // NUL-termination promise, so we hand out the bytes as-is.
        .map(|s| unsafe { s.to_bytes() })
}

fn build_info<T>(tag: ffi::BuildInfo::Type) -> Result<T> {
    let mut value = MaybeUninit::zeroed();
    let result = unsafe { ffi::ghostty_build_info(tag, std::ptr::from_mut(&mut value).cast()) };
    from_result(result)?;
    // SAFETY: Value should be initialized after successful call.
    Ok(unsafe { value.assume_init() })
}

/// The optimization mode libghostty is compiled with.
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum OptimizeMode {
    /// Debug mode.
    ///
    /// Very slow with all safety checks enabled.
    Debug = ffi::OptimizeMode::DEBUG,
    /// Release mode optimized for safety.
    ///
    /// Faster than debug due to better code generation,
    /// but still very slow due to active safety checks.
    ReleaseSafe = ffi::OptimizeMode::RELEASE_SAFE,
    /// Release mode optimized for size.
    ReleaseSmall = ffi::OptimizeMode::RELEASE_SMALL,
    /// Release mode optimized for speed.
    ReleaseFast = ffi::OptimizeMode::RELEASE_FAST,
}
