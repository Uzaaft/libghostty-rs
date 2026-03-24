//! Types and functions around terminal state management.

use std::mem::MaybeUninit;

use crate::{
    alloc::{Allocator, Object},
    error::{Error, Result, from_result},
    ffi, key, style,
};

/// Complete terminal emulator state and rendering.
///
/// A terminal instance manages the full emulator state including the screen,
/// scrollback, cursor, styles, modes, and VT stream processing.
pub struct Terminal<'alloc>(pub(crate) Object<'alloc, ffi::GhosttyTerminal>);

/// Terminal initialization options.
pub struct Options {
    /// Terminal width in cells. Must be greater than zero.
    pub cols: u16,
    /// Terminal height in cells. Must be greater than zero.
    pub rows: u16,
    /// Maximum number of lines to keep in scrollback history.
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
    pub fn new(opts: Options) -> Result<Self> {
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
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), opts) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator, opts: Options) -> Result<Self> {
        let mut raw: ffi::GhosttyTerminal_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_terminal_new(alloc, &mut raw, opts.into()) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    /// Write VT-encoded data to the terminal for processing.
    ///
    /// Feeds raw bytes through the terminal's VT stream parser, updating
    /// terminal state accordingly. By default, sequences that require output
    /// (queries, device status reports) are silently ignored.
    /// Use [`Terminal::set_write_pty_fn`] to install a callback that receives
    /// response data.
    ///
    /// This never fails. Any erroneous input or errors in processing the input
    /// are logged internally but do not cause this function to fail because
    /// this input is assumed to be untrusted and from an external source; so
    /// the primary goal is to keep the terminal state consistent and not allow
    /// malformed input to corrupt or crash.    
    pub fn vt_write(&mut self, data: &[u8]) {
        unsafe { ffi::ghostty_terminal_vt_write(self.0.as_raw(), data.as_ptr(), data.len()) }
    }

    /// Resize the terminal to the given dimensions.
    ///
    /// Changes the number of columns and rows in the terminal. The primary
    /// screen will reflow content if wraparound mode is enabled; the alternate
    /// screen does not reflow. If the dimensions are unchanged, this is a no-op.
    ///
    /// This also updates the terminal's pixel dimensions (used for image
    /// protocols and size reports), disables synchronized output mode (allowed
    /// by the spec so that resize results are shown immediately), and sends an
    /// in-band size report if mode 2048 is enabled.
    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_terminal_resize(self.0.as_raw(), cols, rows, cell_width_px, cell_height_px)
        };
        from_result(result)
    }

    /// Perform a full reset of the terminal (RIS).
    ///
    /// Resets all terminal state back to its initial configuration,
    /// including modes, scrollback, scrolling region, and screen contents.
    /// The terminal dimensions are preserved.
    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_terminal_reset(self.0.as_raw()) }
    }

    /// Scroll the terminal viewport.
    pub fn scroll_viewport(&mut self, scroll: ScrollViewport) {
        unsafe { ffi::ghostty_terminal_scroll_viewport(self.0.as_raw(), scroll.into()) }
    }

    /// Get the current value of a terminal mode.
    pub fn mode(&self, mode: Mode) -> Result<bool> {
        let mut value = false;
        let result =
            unsafe { ffi::ghostty_terminal_mode_get(self.0.as_raw(), mode.into(), &mut value) };
        from_result(result)?;
        Ok(value)
    }

    /// Set the value of a terminal mode.
    pub fn set_mode(&mut self, mode: Mode, value: bool) -> Result<()> {
        let result = unsafe { ffi::ghostty_terminal_mode_set(self.0.as_raw(), mode.into(), value) };
        from_result(result)
    }

    fn get<T>(&self, tag: ffi::GhosttyTerminalData) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result =
            unsafe { ffi::ghostty_terminal_get(self.0.as_raw(), tag, value.as_mut_ptr().cast()) };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// Get the terminal width in cells.
    pub fn cols(&self) -> Result<u16> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_COLS)
    }
    /// Get the terminal height in cells.
    pub fn rows(&self) -> Result<u16> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_ROWS)
    }
    /// Get the cursor column position (0-indexed).
    pub fn cursor_x(&self) -> Result<u16> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_X)
    }
    /// Get the cursor row position within the active area (0-indexed).
    pub fn cursor_y(&self) -> Result<u16> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_Y)
    }
    /// Get whether the cursor has a pending wrap (next print will soft-wrap).
    pub fn is_cursor_pending_wrap(&self) -> Result<bool> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_PENDING_WRAP)
    }
    /// Get whether the cursor is visible (DEC mode 25).
    pub fn is_cursor_visible(&self) -> Result<bool> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_VISIBLE)
    }
    /// Get the current SGR style of the cursor.
    ///
    /// This is the style that will be applied to newly printed characters.
    pub fn cursor_style(&self) -> Result<style::Style> {
        self.get::<ffi::GhosttyStyle>(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_STYLE)
            .and_then(|v| v.try_into())
    }
    /// Get the current Kitty keyboard protocol flags.
    pub fn kitty_keyboard_flags(&self) -> Result<key::KittyKeyFlags> {
        self.get::<ffi::GhosttyKittyKeyFlags>(
            ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS,
        )
        .map(key::KittyKeyFlags::from_bits_retain)
    }

    /// Get the scrollbar state for the terminal viewport.
    ///
    /// This may be expensive to calculate depending on where the viewport is
    /// (arbitrary pins are expensive). The caller should take care to only call
    /// this as needed and not too frequently.
    pub fn scrollbar(&self) -> Result<ffi::GhosttyTerminalScrollbar> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_SCROLLBAR)
    }
    /// Get the currently active screen.
    pub fn active_screen(&self) -> Result<ffi::GhosttyTerminalScreen> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN)
    }
    /// Get whether any mouse tracking mode is active.
    ///
    /// Returns true if any of the mouse tracking modes (X10, normal, button,
    /// or any-event) are enabled.
    pub fn is_mouse_tracking(&self) -> Result<bool> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING)
    }
    /// Get the terminal title as set by escape sequences (e.g. OSC 0/2).
    ///
    /// Returns a borrowed string, valid until the next call to
    /// [`Terminal::vt_write`] or [`Terminal::reset`]. An empty string is
    /// returned when no title has been set.
    pub fn title(&self) -> Result<&str> {
        let str = self.get::<ffi::GhosttyString>(
            ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
        )?;
        // SAFETY: We trust libghostty to return a valid borrowed string,
        // while we uphold that no mutation could happen during its lifetime.
        let str = unsafe { std::slice::from_raw_parts(str.ptr, str.len) };
        std::str::from_utf8(str).map_err(|_| Error::InvalidValue)
    }

    /// Get the current working directory as set by escape sequences (e.g. OSC 7).
    ///
    /// Returns a borrowed string, valid until the next call to
    /// [`Terminal::vt_write`] or [`Terminal::reset`]. An empty string is
    /// returned when no title has been set.
    pub fn pwd(&self) -> Result<&str> {
        let str =
            self.get::<ffi::GhosttyString>(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_PWD)?;
        // SAFETY: We trust libghostty to return a valid borrowed string,
        // while we uphold that no mutation could happen during its lifetime.
        let str = unsafe { std::slice::from_raw_parts(str.ptr, str.len) };
        std::str::from_utf8(str).map_err(|_| Error::InvalidValue)
    }
    /// The total number of rows in the active screen including scrollback.
    pub fn total_rows(&self) -> Result<usize> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_TOTAL_ROWS)
    }
    ///  The number of scrollback rows (total rows minus viewport rows).
    pub fn scrollback_rows(&self) -> Result<usize> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS)
    }
}

impl<'alloc> Drop for Terminal<'alloc> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.0.as_raw()) }
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
