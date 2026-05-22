//! Idiomatic, safe Rust bindings for libghostty-vt, a terminal emulation library.
//!
//! # Memory management and lifetimes
//!
//! When creating the terminal and various other objects, you can control their
//! memory management via a **custom allocator**, usually specified with
//! methods like [`Terminal::new_with_alloc`]. Objects that accept allocators
//! are also bound by the `'alloc` lifetime, since they internally contain
//! a reference to the allocator. If you do not use a custom allocator,
//! feel free to always set the lifetime to `'static`.
//!
//! ## Using the unstable `Allocator` API
//!
//! You can adapt the existing, unstable `Allocator` API into a
//! [libghostty-friendly allocator](alloc::Allocator) via its `From`
//! implementation. Note that the `'alloc` lifetime must at least
//! live as long as the `Allocator` instance itself.
//!
//! # Thread safety
//!
//! All `libghostty-vt` objects are **not** thread-safe, and have been marked
//! `!Send + !Sync` accordingly. The expectation is for them to be managed
//! by a single thread, that may communicate with other threads via channels.
#![warn(clippy::pedantic)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]
#![warn(missing_copy_implementations)]
#![warn(clippy::allow_attributes)]
#![warn(clippy::allow_attributes_without_reason)]
#![allow(
    clippy::missing_errors_doc,
    reason = "underlying C API may return any error outside of expected and
    mitigated situations, and it is not feasible to document them all"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use libghostty_vt_sys as ffi;

pub mod alloc;
pub mod build_info;
pub mod error;
pub mod fmt;
pub mod focus;
pub mod key;
pub mod kitty;
pub mod log;
pub mod mouse;
pub mod osc;
pub mod paste;
pub mod render;
pub mod screen;
pub mod sgr;
pub mod style;
pub mod terminal;

#[doc(inline)]
pub use crate::{
    error::Error,
    log::{Logger, set_logger},
    render::RenderState,
    terminal::{Options as TerminalOptions, Terminal},
};

pub(crate) fn sys_set<T>(opt: ffi::SysOption::Type, val: *const T) -> error::Result<()> {
    let result = unsafe { ffi::ghostty_sys_set(opt, val.cast()) };
    error::from_result(result)
}
