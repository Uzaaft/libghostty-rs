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

use std::marker::PhantomData;
use std::ptr::NonNull;

pub mod alloc;
pub mod build_info;
pub mod error;
pub mod fmt;
pub mod osc;
pub mod paste;
pub mod render;
pub mod sgr;
pub mod style;
pub mod terminal;

#[doc(inline)]
pub use crate::{
    error::Error,
    render::RenderState,
    terminal::{Options as TerminalOptions, Terminal},
};

use crate::error::{from_result, from_result_with_len};

pub const EXPORTED_API_SYMBOLS: &[&str] = ffi::EXPORTED_API_SYMBOLS;

// ---------------------------------------------------------------------------
// Focus encode
// ---------------------------------------------------------------------------

pub fn focus_encode(event: ffi::GhosttyFocusEvent, buf: &mut [u8]) -> Result<usize, Error> {
    let mut written: usize = 0;
    let result = unsafe {
        ffi::ghostty_focus_encode(event, buf.as_mut_ptr().cast(), buf.len(), &mut written)
    };
    from_result_with_len(result, written)
}

// ---------------------------------------------------------------------------
// KeyEvent
// ---------------------------------------------------------------------------

pub struct KeyEvent {
    ptr: NonNull<ffi::GhosttyKeyEvent>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl KeyEvent {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyKeyEvent_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_key_event_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn as_raw(&self) -> ffi::GhosttyKeyEvent_ptr {
        self.ptr.as_ptr()
    }

    pub fn set_action(&mut self, action: ffi::GhosttyKeyAction) {
        unsafe { ffi::ghostty_key_event_set_action(self.ptr.as_ptr(), action) }
    }

    pub fn get_action(&self) -> ffi::GhosttyKeyAction {
        unsafe { ffi::ghostty_key_event_get_action(self.ptr.as_ptr()) }
    }

    pub fn set_key(&mut self, key: ffi::GhosttyKey) {
        unsafe { ffi::ghostty_key_event_set_key(self.ptr.as_ptr(), key) }
    }

    pub fn get_key(&self) -> ffi::GhosttyKey {
        unsafe { ffi::ghostty_key_event_get_key(self.ptr.as_ptr()) }
    }

    pub fn set_mods(&mut self, mods: ffi::GhosttyMods) {
        unsafe { ffi::ghostty_key_event_set_mods(self.ptr.as_ptr(), mods) }
    }

    pub fn get_mods(&self) -> ffi::GhosttyMods {
        unsafe { ffi::ghostty_key_event_get_mods(self.ptr.as_ptr()) }
    }

    pub fn set_consumed_mods(&mut self, mods: ffi::GhosttyMods) {
        unsafe { ffi::ghostty_key_event_set_consumed_mods(self.ptr.as_ptr(), mods) }
    }

    pub fn get_consumed_mods(&self) -> ffi::GhosttyMods {
        unsafe { ffi::ghostty_key_event_get_consumed_mods(self.ptr.as_ptr()) }
    }

    pub fn set_composing(&mut self, composing: bool) {
        unsafe { ffi::ghostty_key_event_set_composing(self.ptr.as_ptr(), composing) }
    }

    pub fn get_composing(&self) -> bool {
        unsafe { ffi::ghostty_key_event_get_composing(self.ptr.as_ptr()) }
    }

    pub fn set_utf8(&mut self, text: Option<&[u8]>) {
        match text {
            Some(bytes) => unsafe {
                ffi::ghostty_key_event_set_utf8(
                    self.ptr.as_ptr(),
                    bytes.as_ptr().cast(),
                    bytes.len(),
                )
            },
            None => unsafe {
                ffi::ghostty_key_event_set_utf8(self.ptr.as_ptr(), std::ptr::null(), 0)
            },
        }
    }

    pub fn set_unshifted_codepoint(&mut self, codepoint: u32) {
        unsafe { ffi::ghostty_key_event_set_unshifted_codepoint(self.ptr.as_ptr(), codepoint) }
    }

    pub fn get_unshifted_codepoint(&self) -> u32 {
        unsafe { ffi::ghostty_key_event_get_unshifted_codepoint(self.ptr.as_ptr()) }
    }
}

impl Drop for KeyEvent {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_event_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// KeyEncoder
// ---------------------------------------------------------------------------

pub struct KeyEncoder {
    ptr: NonNull<ffi::GhosttyKeyEncoder>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl KeyEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyKeyEncoder_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_key_encoder_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn setopt(&mut self, option: ffi::GhosttyKeyEncoderOption, value: *const std::ffi::c_void) {
        unsafe { ffi::ghostty_key_encoder_setopt(self.ptr.as_ptr(), option, value) }
    }

    pub fn setopt_from_terminal(&mut self, terminal: &Terminal) {
        unsafe {
            ffi::ghostty_key_encoder_setopt_from_terminal(self.ptr.as_ptr(), terminal.as_raw())
        }
    }

    pub fn encode(&mut self, event: &KeyEvent, buf: &mut [u8]) -> Result<usize, Error> {
        let mut written: usize = 0;
        let result = unsafe {
            ffi::ghostty_key_encoder_encode(
                self.ptr.as_ptr(),
                event.as_raw(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut written,
            )
        };
        from_result_with_len(result, written)
    }
}

impl Drop for KeyEncoder {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_encoder_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// MouseEvent
// ---------------------------------------------------------------------------

pub struct MouseEvent {
    ptr: NonNull<ffi::GhosttyMouseEvent>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl MouseEvent {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyMouseEvent_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_mouse_event_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn as_raw(&self) -> ffi::GhosttyMouseEvent_ptr {
        self.ptr.as_ptr()
    }

    pub fn set_action(&mut self, action: ffi::GhosttyMouseAction) {
        unsafe { ffi::ghostty_mouse_event_set_action(self.ptr.as_ptr(), action) }
    }

    pub fn get_action(&self) -> ffi::GhosttyMouseAction {
        unsafe { ffi::ghostty_mouse_event_get_action(self.ptr.as_ptr()) }
    }

    pub fn set_button(&mut self, button: ffi::GhosttyMouseButton) {
        unsafe { ffi::ghostty_mouse_event_set_button(self.ptr.as_ptr(), button) }
    }

    pub fn clear_button(&mut self) {
        unsafe { ffi::ghostty_mouse_event_clear_button(self.ptr.as_ptr()) }
    }

    pub fn get_button(&self) -> Option<ffi::GhosttyMouseButton> {
        let mut button: ffi::GhosttyMouseButton = 0;
        let has_button =
            unsafe { ffi::ghostty_mouse_event_get_button(self.ptr.as_ptr(), &mut button) };
        if has_button { Some(button) } else { None }
    }

    pub fn set_mods(&mut self, mods: ffi::GhosttyMods) {
        unsafe { ffi::ghostty_mouse_event_set_mods(self.ptr.as_ptr(), mods) }
    }

    pub fn get_mods(&self) -> ffi::GhosttyMods {
        unsafe { ffi::ghostty_mouse_event_get_mods(self.ptr.as_ptr()) }
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        let pos = ffi::GhosttyMousePosition { x, y };
        unsafe { ffi::ghostty_mouse_event_set_position(self.ptr.as_ptr(), pos) }
    }

    pub fn get_position(&self) -> ffi::GhosttyMousePosition {
        unsafe { ffi::ghostty_mouse_event_get_position(self.ptr.as_ptr()) }
    }
}

impl Drop for MouseEvent {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_event_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// MouseEncoder
// ---------------------------------------------------------------------------

pub struct MouseEncoder {
    ptr: NonNull<ffi::GhosttyMouseEncoder>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl MouseEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyMouseEncoder_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_mouse_encoder_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn setopt(
        &mut self,
        option: ffi::GhosttyMouseEncoderOption,
        value: *const std::ffi::c_void,
    ) {
        unsafe { ffi::ghostty_mouse_encoder_setopt(self.ptr.as_ptr(), option, value) }
    }

    pub fn setopt_from_terminal(&mut self, terminal: &Terminal) {
        unsafe {
            ffi::ghostty_mouse_encoder_setopt_from_terminal(self.ptr.as_ptr(), terminal.as_raw())
        }
    }

    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_mouse_encoder_reset(self.ptr.as_ptr()) }
    }

    pub fn encode(&mut self, event: &MouseEvent, buf: &mut [u8]) -> Result<usize, Error> {
        let mut written: usize = 0;
        let result = unsafe {
            ffi::ghostty_mouse_encoder_encode(
                self.ptr.as_ptr(),
                event.as_raw(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut written,
            )
        };
        from_result_with_len(result, written)
    }
}

impl Drop for MouseEncoder {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_encoder_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// Cell / Row helpers
// ---------------------------------------------------------------------------``

pub fn cell_get_content_tag(cell: ffi::GhosttyCell) -> Result<ffi::GhosttyCellContentTag, Error> {
    let mut value: ffi::GhosttyCellContentTag = 0;
    let result = unsafe {
        ffi::ghostty_cell_get(
            cell,
            ffi::GhosttyCellData_GHOSTTY_CELL_DATA_CONTENT_TAG,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}

pub fn cell_get_codepoint(cell: ffi::GhosttyCell) -> Result<u32, Error> {
    let mut value: u32 = 0;
    let result = unsafe {
        ffi::ghostty_cell_get(
            cell,
            ffi::GhosttyCellData_GHOSTTY_CELL_DATA_CODEPOINT,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}

pub fn cell_get_color_palette(
    cell: ffi::GhosttyCell,
) -> Result<ffi::GhosttyColorPaletteIndex, Error> {
    let mut value: ffi::GhosttyColorPaletteIndex = 0;
    let result = unsafe {
        ffi::ghostty_cell_get(
            cell,
            ffi::GhosttyCellData_GHOSTTY_CELL_DATA_COLOR_PALETTE,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}

pub fn cell_get_color_rgb(cell: ffi::GhosttyCell) -> Result<ffi::GhosttyColorRgb, Error> {
    let mut value = ffi::GhosttyColorRgb::default();
    let result = unsafe {
        ffi::ghostty_cell_get(
            cell,
            ffi::GhosttyCellData_GHOSTTY_CELL_DATA_COLOR_RGB,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}
