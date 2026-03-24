//! Idiomatic, safe Rust bindings for `libghostty-vt`, a terminal emulation library.
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
pub use ghostty_sys as ffi;

pub mod alloc;
pub mod build_info;
pub mod error;
pub mod fmt;
pub mod focus;
pub mod key;
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
    render::RenderState,
    terminal::{Options as TerminalOptions, Terminal},
};
