//! Types and functions around terminal state management.

use std::{marker::PhantomData, ptr::NonNull};

use crate::{
    alloc::Allocator,
    error::{Error, from_result},
    ffi,
};

/// Complete terminal emulator state and rendering.
///
/// A terminal instance manages the full emulator state including the screen,
/// scrollback, cursor, styles, modes, and VT stream processing.
pub struct Terminal<'alloc> {
    ptr: NonNull<ffi::GhosttyTerminal>,
    _alloc: PhantomData<&'alloc ffi::GhosttyAllocator>,
}

pub struct Options {
    pub cols: u16,
    pub rows: u16,
    pub max_scrollback: usize,
}

impl From<Options> for ffi::GhosttyTerminalOptions {
    fn from(value: Options) -> Self {
        Self {
            cols: value.cols,
            rows: value.rows,
            max_scrollback: value.max_scrollback,
        }
    }
}

impl<'alloc> Terminal<'alloc> {
    /// Create a new terminal instance.
    pub fn new(opts: Options) -> Result<Self, Error> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), opts) }
    }

    /// Create a new terminal instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(
        alloc: &'alloc Allocator<'ctx, Ctx>,
        opts: Options,
    ) -> Result<Self, Error> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), opts) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator, opts: Options) -> Result<Self, Error> {
        let mut raw: ffi::GhosttyTerminal_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_terminal_new(alloc, &mut raw, opts.into()) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _alloc: PhantomData,
        })
    }

    pub fn as_raw(&self) -> ffi::GhosttyTerminal_ptr {
        self.ptr.as_ptr()
    }

    pub fn vt_write(&mut self, data: &[u8]) {
        // SAFETY: `self.ptr` stays valid before and after this operation.
        // `data` is guaranteed to contain valid, in-bounds data per Rust's
        // safety guarantees.
        unsafe { ffi::ghostty_terminal_vt_write(self.ptr.as_ptr(), data.as_ptr(), data.len()) }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        // SAFETY: `self.ptr` stays valid before and after this operation.
        let result = unsafe { ffi::ghostty_terminal_resize(self.ptr.as_ptr(), cols, rows) };
        from_result(result)
    }

    pub fn reset(&mut self) {
        // SAFETY: `self.ptr` stays valid before and after this operation.
        unsafe { ffi::ghostty_terminal_reset(self.ptr.as_ptr()) }
    }

    pub fn scroll_viewport(&mut self, scroll: ScrollViewport) {
        // SAFETY: `self.ptr` stays valid before and after this operation.
        unsafe { ffi::ghostty_terminal_scroll_viewport(self.ptr.as_ptr(), scroll.into()) }
    }

    pub fn mode(&self, mode: Mode) -> Result<bool, Error> {
        let mut value = false;
        // SAFETY: `self.ptr` stays valid before and after this operation.
        let result =
            unsafe { ffi::ghostty_terminal_mode_get(self.ptr.as_ptr(), mode.into(), &mut value) };
        from_result(result)?;
        Ok(value)
    }

    pub fn set_mode(&mut self, mode: Mode, value: bool) -> Result<(), Error> {
        // SAFETY: `self.ptr` stays valid before and after this operation.
        let result =
            unsafe { ffi::ghostty_terminal_mode_set(self.ptr.as_ptr(), mode.into(), value) };
        from_result(result)
    }

    pub fn cols(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_terminal_get(
                self.ptr.as_ptr(),
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_COLS,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn rows(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_terminal_get(
                self.ptr.as_ptr(),
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_ROWS,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn cursor_x(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_terminal_get(
                self.ptr.as_ptr(),
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_X,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn cursor_y(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_terminal_get(
                self.ptr.as_ptr(),
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_Y,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn scrollbar(&self) -> Result<ffi::GhosttyTerminalScrollbar, Error> {
        let mut value = ffi::GhosttyTerminalScrollbar::default();
        let result = unsafe {
            ffi::ghostty_terminal_get(
                self.ptr.as_ptr(),
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }
}

impl<'alloc> Drop for Terminal<'alloc> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.ptr.as_ptr()) }
    }
}

pub enum ScrollViewport {
    Top,
    Bottom,
    Delta(isize),
}
impl From<ScrollViewport> for ffi::GhosttyTerminalScrollViewport {
    fn from(value: ScrollViewport) -> Self {
        match value {
            ScrollViewport::Top => Self {
                tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_TOP,
                value: ffi::GhosttyTerminalScrollViewportValue::default(),
            },
            ScrollViewport::Bottom => Self {
                tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_TOP,
                value: ffi::GhosttyTerminalScrollViewportValue::default(),
            },
            ScrollViewport::Delta(delta) => Self {
                tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_TOP,
                value: {
                    let mut v = ffi::GhosttyTerminalScrollViewportValue::default();
                    v.delta = delta;
                    v
                },
            },
        }
    }
}

/// A terminal mode consisting of its value and its kind (DEC/ANSI).
#[non_exhaustive]
#[repr(u16)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Kam = 2 | Self::ANSI_BIT,
    Insert = 4 | Self::ANSI_BIT,
    Srm = 12 | Self::ANSI_BIT,
    Linefeed = 20 | Self::ANSI_BIT,

    Decckm = 1,
    _132Column = 3,
    SlowScroll = 4,
    ReverseColors = 5,
    Origin = 6,
    Wraparound = 7,
    Autorepeat = 8,
    X10Mouse = 9,
    CursorBlinking = 12,
    CursorVisible = 25,
    EnableMode3 = 40,
    ReverseWrap = 45,
    AltScreenLegacy = 47,
    KeypadKeys = 66,
    LeftRightMargin = 69,
    NormalMouse = 1000,
    ButtonMouse = 1002,
    AnyMouse = 1003,
    FocusEvent = 1004,
    Utf8Mouse = 1005,
    SgrMouse = 1006,
    AltScroll = 1007,
    UrxvtMouse = 1015,
    SgrPixelsMouse = 1016,
    NumlockKeypad = 1035,
    AltEscPrefix = 1036,
    AltSendsEsc = 1039,
    ReverseWrapExt = 1045,
    AltScreen = 1047,
    SaveCursor = 1048,
    AltScreenSave = 1049,
    BracketedPaste = 2004,
    SyncOutput = 2026,
    GraphemeCluster = 2027,
    ColorSchemeReport = 2031,
    InBandResize = 2048,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ModeKind {
    Dec,
    Ansi,
}

impl Mode {
    const ANSI_BIT: u16 = 1 << 15;

    pub fn value(self) -> u16 {
        (self as u16) & 0x7fff
    }

    pub fn kind(self) -> ModeKind {
        if (self as u16) & Self::ANSI_BIT > 0 {
            ModeKind::Ansi
        } else {
            ModeKind::Dec
        }
    }
}
impl From<Mode> for ffi::GhosttyMode {
    fn from(value: Mode) -> Self {
        value as Self
    }
}
