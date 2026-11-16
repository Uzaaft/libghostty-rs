//! Idiomatic, safe Rust bindings for [`libghostty-vt`](https://lib.ghostty.org/vt),
//! a portable, high-performance C library for terminal emulation. It is
//! [open source](https://github.com/ghostty-org/ghostty) and
//! [MIT licensed](https://github.com/ghostty-org/ghostty/blob/main/LICENSE).
//!
//! Guide-level documentation is available in the [Book], which is a Rustified
//! one-to-one translation of the original  [C documentation](https://lib.ghostty.org/vt).
//! The API reference here will frequently link back to the book, so make sure
//! to familiarize yourself with it. The [Getting Started] section is especially
//! important for beginners.
//!
#![doc = concat!("[Book]: ", crate::book!())]
#![doc = concat!("[Getting Started]: ", crate::book!("getting-started/index"))]
//!
//! # Rust version
//!
//! The minimum supported Rust version is 1.96. Enabling `allocator_api`
//! requires Rust 1.100 or newer and adapts standard library allocators.
//!
//! # Sections
//!
//! libghostty-vt can be broadly broken down into these sections, whose modules
//! are all related to each other. Start with the [Terminal](crate::terminal)
//! module if you don't know where to begin!
//!
//! ## Terminal
//!
//! - [Terminal](crate::terminal) - Create a terminal, write VT data to it,
//!   and read its state
//! - [Effects](crate::effect) - Respond to what programs ask of
//!   the terminal, such as the bell, title changes, and queries
//! - [Modes](crate::mode) - Pack DEC and ANSI mode numbers and
//!   encode DECRPM reports
//! - [Snapshots](crate::snapshot) - Save a terminal's full state and restore
//!   it later
//!
//! ## Screen Contents
//!
//! - [Cells and Rows](crate::screen) - Read the contents of grid cells and rows
//! - [Points](crate::point) - Name a grid position in the active area,
//! - [Grid References](crate::grid_ref) - Point at a cell, read it, and keep
//!   track of it as the terminal scrolls
//! - [Style](crate::style) - Read a cell's colors and text attributes
//! - [Selection](crate::selection) - Select words, lines, and ranges, and turn
//!   pointer gestures into selections
//! - [Search](crate::search) - Find a string in a terminal, including its scrollback
//! - [Formatter](crate::fmt) - Turn terminal content into plain text, VT
//!   sequences, or HTML
//!
//! ## Rendering
//!
//! - [Render State](crate::render) - Find out what changed since the last frame
//!   and read the rows and cells to draw
//! - [Kitty Graphics](crate::kitty::graphics) - Read the images that programs
//!   send with the Kitty graphics protocol
//!
//! ## Input
//!
//! Turn user input into bytes for the pty.
//!
//! - [Keys](crate::key) - Encode key events with the legacy encoding or the
//!   Kitty keyboard protocol
//! - [Mouse](crate::mouse) - Encode mouse events with the X10, UTF-8, SGR,
//!   URxvt, or SGR-Pixels protocols.
//! - [Focus](crate::focus) - Encode focus gained and lost events, for mode 1004
//! - [Paste](crate::paste) - Paste into a terminal, and check paste data for
//!   unsafe content
//!
//! ## Building Blocks
//!
//! Parsers and encoders that work without a terminal.
//!
//! - [OSC Parser](crate::osc) - Parse OSC sequences into commands
//! - [SGR Parser](crate::sgr) - Parse SGR sequences into text attributes
//! - [Terminal Reports](crate::report) - Reports size, color scheme, etc.
//!   information to the running program
//! - [Colors](crate::color) - Parse colors, generate palettes, and look up
//!   X11 color names
//! - [Unicode](crate::unicode) - Measure codepoints and graphemes the way
//!   the terminal does
//!
//! ## Library
//!
//! Types and settings shared by every API.
//!
//! - [Error Handling](crate::error) - All the possible errors libghostty-vt returns.
//! - [Build Info](crate::build_info) - Find out how this copy of the library
//!   was built and which features it has
//! - [Allocators](crate::alloc) - Run the library on your own memory allocator
//! - [System Interface](crate::sys) - Plug in your own system functions, such
//!   as a PNG decoder
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/ghostty-org/ghostty/2d0fb81751def478e2f8a5f7e2ee91fa9cbf9bff/images/icons/icon_128@2x.png"
)]
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
#![allow(clippy::doc_markdown, reason = "false positives")]
#![cfg_attr(docsrs, feature(doc_cfg))]

// Cargo's rust-version cannot express a higher minimum for an optional feature.
#[cfg(feature = "allocator_api")]
#[rustversion::before(1.100)]
compile_error!("The `allocator_api` feature requires Rust 1.100 or newer.");

pub use libghostty_vt_sys as ffi;

// Make sure that `Terminal`'s own impl blocks (i.e. core functions)
// are placed *before* any extra impl blocks from other modules,
// e.g. Kitty Graphics extensions, Selection APIs
pub mod terminal;

pub mod alloc;
pub mod build_info;
pub mod color;
pub mod effect;
pub mod error;
pub mod fmt;
pub mod focus;
pub mod grid_ref;
pub(crate) mod io;
pub mod key;
pub mod kitty;
pub mod log;
pub mod mode;
pub mod mouse;
pub mod osc;
pub mod paste;
pub mod point;
pub mod render;
pub mod report;
pub mod screen;
pub mod search;
pub mod selection;
pub mod sgr;
pub mod snapshot;
pub mod style;
pub mod sys;
pub mod unicode;

#[doc(inline)]
pub use crate::{
    error::Error,
    log::{Logger, set_logger},
    render::RenderState,
    terminal::Terminal,
};

pub(crate) fn sys_set<T>(opt: ffi::SysOption::Type, val: *const T) -> error::Result<()> {
    let result = unsafe { ffi::ghostty_sys_set(opt, val.cast()) };
    error::from_result(result)
}

/// Get the URL to a specific page in the book.
macro_rules! book {
    ($page:expr, $section:expr) => {
        concat!(crate::book!($page), "#", $section)
    };
    ($page:expr) => {
        concat!(crate::book!(), "/", $page, ".html")
    };
    () => {
        env!("LIBGHOSTTY_BOOK_URL")
    };
}
use book;

/// Form a Markdown link reference to the guide.
macro_rules! guide_ref {
    ($name:expr, $id:expr, $section:expr) => {
        concat!(
            "[",
            $name,
            "]: ",
            crate::book!(
                concat!("guides/", $id),
                concat!("guide-", $id, "-", $section)
            ),
        )
    };
    ($name:expr, $id:expr) => {
        concat!(
            "[",
            $name,
            "]: ",
            crate::book!(concat!("guides/", $id), concat!("guide-", $id)),
        )
    };
}
use guide_ref;
