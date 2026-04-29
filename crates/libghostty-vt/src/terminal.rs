//! Types and functions around terminal state management.

use std::mem::MaybeUninit;

use crate::{
    alloc::{Allocator, Object},
    error::{Error, Result, from_optional_result, from_result},
    ffi::{self, TerminalData as Data, TerminalOption as Opt},
    key,
    screen::{GridRef, Screen},
    style::{self, RgbColor},
};

#[doc(inline)]
pub use ffi::{SizeReportSize, TerminalScrollbar as Scrollbar};

/// Complete terminal emulator state and rendering.
///
/// A terminal instance manages the full emulator state including the screen,
/// scrollback, cursor, styles, modes, and VT stream processing.
///
/// Once a terminal session is up and running, you can configure a key encoder
/// to write keyboard input via [`key::Encoder::set_options_from_terminal`].
///
/// # Effects
///
/// By default, the terminal sequence processing with [`Terminal::vt_write`]
/// only process sequences that directly affect terminal state and ignores
/// sequences that have side effect behavior or require responses. These
/// sequences include things like bell characters, title changes, device
/// attributes queries, and more. To handle these sequences, the user
/// must configure "effects."
///
/// Effects are callbacks that the terminal invokes in response to VT sequences
/// processed during [`Terminal::vt_write`]. They let the embedding application
/// react to terminal-initiated events such as bell characters, title changes,
/// device status report responses, and more.
///
/// Each effect is registered with its corresponding `Terminal::on_<effect>`
/// function, which accepts a closure with access to the terminal state and
/// possibly other parameters. Some examples include [`Terminal::on_bell`]
/// and [`Terminal::on_pty_write`].
///
/// All callbacks are invoked synchronously during [`Terminal::vt_write`].
/// Callbacks must be very careful to not block for too long or perform
/// expensive operations, since they are blocking further IO processing.
///
/// ## Shared state
///
/// **Unlike the C API**, you *cannot* specify arbitrary user data that's
/// shared between all callbacks, mainly because a safe, idiomatic Rust
/// equivalent of this pattern is very difficult to implement and use
/// due to Rust's much stricter safety guarantees. In turn, we use the
/// user data internally for callback dispatch purposes.
///
/// You should instead use idiomatic Rust mechanisms like [`Rc`](std::rc::Rc)s
/// to hold common, mutable state between callbacks (which is perfectly safe,
/// since everything is run on a single thread within a single `vt_write` call),
/// or with some other type with interior mutability.
///
/// ## Example: Registering effects and processing VT data
///
/// ```rust
/// use std::{cell::Cell, rc::Rc};
/// use libghostty_vt::{Terminal, TerminalOptions};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut terminal = Terminal::new(TerminalOptions {
///     cols: 80,
///     rows: 24,
///     max_scrollback: 0,
/// })?;
///
/// // Set up a simple bell counter
/// let bell_count = Rc::new(Cell::new(0usize));
/// terminal
///     .on_pty_write(|_term, data| {
///         println!("Replying {} bytes to the PTY", data.len());
///     })?
///    .on_bell({
///        let bell_count = bell_count.clone();
///        move |_term| {
///            bell_count.update(|v| v + 1);
///            println!("Bell! (count = {})", bell_count.get())
///        }
///     })?
///    .on_title_changed(|term| {
///        // Query the cursor position to confirm the terminal processed the
///        // title change (the title itself is tracked by the embedder via the
///        // OSC parser or its own state).
///        let col = term.cursor_x().unwrap();
///        println!("Title changed! (cursor at col {col})");
///    })?;
///
/// // Feed VT data that triggers effects:
/// // 1. Bell (BEL = 0x07)
/// terminal.vt_write(b"\x07");
/// // 2. Title change (OSC 2 ; <title> ST)
/// terminal.vt_write(b"\x1b]2;Hello Effects\x1b\\");
/// // 3. Device status report (DECRQM for wraparound mode ?7)
/// //    triggers write_pty with the response
/// terminal.vt_write(b"\x1B[?7$p");
/// // 4. Another bell to show the counter increments
/// terminal.vt_write(b"\x07");
///
/// assert_eq!(bell_count.get(), 2);
/// # Ok(())}
/// ```
#[derive(Debug)]
pub struct Terminal<'alloc: 'cb, 'cb> {
    pub(crate) inner: Object<'alloc, ffi::TerminalImpl>,
    // Keep callbacks in a heap allocation so C can store a userdata pointer
    // to the VTable itself. That pointer remains stable even if Terminal moves.
    vtable: Box<VTable<'alloc, 'cb>>,
}

/// Terminal initialization options.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Terminal width in cells. Must be greater than zero.
    pub cols: u16,
    /// Terminal height in cells. Must be greater than zero.
    pub rows: u16,
    /// Maximum number of lines to keep in scrollback history.
    pub max_scrollback: usize,
}

impl From<Options> for ffi::TerminalOptions {
    fn from(value: Options) -> Self {
        Self {
            cols: value.cols,
            rows: value.rows,
            max_scrollback: value.max_scrollback,
        }
    }
}

impl<'alloc: 'cb, 'cb> Terminal<'alloc, 'cb> {
    /// Create a new terminal instance.
    pub fn new(opts: Options) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), opts) }
    }

    /// Create a new terminal instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc>(
        alloc: &'alloc Allocator<'ctx>,
        opts: Options,
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), opts) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator, opts: Options) -> Result<Self> {
        let mut raw: ffi::Terminal = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_terminal_new(alloc, &raw mut raw, opts.into()) };
        from_result(result)?;
        Ok(Self {
            inner: Object::new(raw)?,
            vtable: Box::new(VTable::default()),
        })
    }

    /// Write VT-encoded data to the terminal for processing.
    ///
    /// Feeds raw bytes through the terminal's VT stream parser, updating
    /// terminal state accordingly. By default, sequences that require output
    /// (queries, device status reports) are silently ignored.
    /// Use [`Terminal::on_pty_write`] to install a callback that receives
    /// response data.
    ///
    /// This never fails. Any erroneous input or errors in processing the input
    /// are logged internally but do not cause this function to fail because
    /// this input is assumed to be untrusted and from an external source; so
    /// the primary goal is to keep the terminal state consistent and not allow
    /// malformed input to corrupt or crash.    
    pub fn vt_write(&mut self, data: &[u8]) {
        unsafe { ffi::ghostty_terminal_vt_write(self.inner.as_raw(), data.as_ptr(), data.len()) }
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
            ffi::ghostty_terminal_resize(
                self.inner.as_raw(),
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            )
        };
        from_result(result)
    }

    /// Perform a full reset of the terminal (RIS).
    ///
    /// Resets all terminal state back to its initial configuration,
    /// including modes, scrollback, scrolling region, and screen contents.
    /// The terminal dimensions are preserved.
    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_terminal_reset(self.inner.as_raw()) }
    }

    /// Scroll the terminal viewport.
    pub fn scroll_viewport(&mut self, scroll: ScrollViewport) {
        unsafe { ffi::ghostty_terminal_scroll_viewport(self.inner.as_raw(), scroll.into()) }
    }

    /// Resolve a point in the terminal grid to a grid reference.
    ///
    /// Resolves the given point (which can be in active, viewport, screen,
    /// or history coordinates) to a grid reference for that location. Use
    /// [`GridRef::cell`] and [`GridRef::row`] to extract the cell and row.
    ///
    /// Lookups in the active region and viewport are fast. Lookups in the
    /// screen and history may require traversing the full scrollback page
    /// list to resolve the y coordinate, so they can be expensive for large
    /// scrollback buffers.
    ///
    /// This function isn't meant to be used as the core of render loop. It
    /// isn't built to sustain the framerates needed for rendering large
    /// screens. Use the [render state API](crate::render::RenderState) for
    /// that. This API is instead meant for less strictly performance-sensitive
    /// use cases.
    pub fn grid_ref(&self, point: Point) -> Result<GridRef<'_>> {
        let mut grid_ref = ffi::sized!(ffi::GridRef);
        let result = unsafe {
            ffi::ghostty_terminal_grid_ref(self.inner.as_raw(), point.into(), &raw mut grid_ref)
        };
        from_result(result)?;
        Ok(unsafe { GridRef::from_raw(grid_ref) })
    }

    /// Get the current value of a terminal mode.
    pub fn mode(&self, mode: Mode) -> Result<bool> {
        let mut value = false;
        let result = unsafe {
            ffi::ghostty_terminal_mode_get(self.inner.as_raw(), mode.into(), &raw mut value)
        };
        from_result(result)?;
        Ok(value)
    }

    /// Set the value of a terminal mode.
    pub fn set_mode(&mut self, mode: Mode, value: bool) -> Result<&mut Self> {
        let result =
            unsafe { ffi::ghostty_terminal_mode_set(self.inner.as_raw(), mode.into(), value) };
        from_result(result)?;
        Ok(self)
    }

    pub(crate) fn get<T>(&self, tag: ffi::TerminalData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_terminal_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }
    pub(crate) fn get_optional<T>(&self, tag: ffi::TerminalData::Type) -> Result<Option<T>> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_terminal_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_optional_result(result, value)
    }
    pub(crate) fn set<T>(&self, tag: ffi::TerminalOption::Type, v: &T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_terminal_set(self.inner.as_raw(), tag, std::ptr::from_ref(v).cast())
        };
        from_result(result)
    }
    /// Set an option whose ABI expects the pointer value itself, not a pointer
    /// to Rust storage containing that value.
    pub(crate) fn set_ptr(
        &self,
        tag: ffi::TerminalOption::Type,
        ptr: *const std::ffi::c_void,
    ) -> Result<()> {
        let result = unsafe { ffi::ghostty_terminal_set(self.inner.as_raw(), tag, ptr) };
        from_result(result)
    }
    pub(crate) fn set_optional<T>(
        &mut self,
        tag: ffi::TerminalOption::Type,
        v: Option<&T>,
    ) -> Result<()> {
        let ptr = if let Some(v) = v {
            std::ptr::from_ref(v)
        } else {
            std::ptr::null()
        };

        let result = unsafe { ffi::ghostty_terminal_set(self.inner.as_raw(), tag, ptr.cast()) };
        from_result(result)
    }

    /// Get the terminal width in cells.
    pub fn cols(&self) -> Result<u16> {
        self.get(Data::COLS)
    }
    /// Get the terminal height in cells.
    pub fn rows(&self) -> Result<u16> {
        self.get(Data::ROWS)
    }
    /// Get the cursor column position (inner-indexed).
    pub fn cursor_x(&self) -> Result<u16> {
        self.get(Data::CURSOR_X)
    }
    /// Get the cursor row position within the active area (inner-indexed).
    pub fn cursor_y(&self) -> Result<u16> {
        self.get(Data::CURSOR_Y)
    }
    /// Get whether the cursor has a pending wrap (next print will soft-wrap).
    pub fn is_cursor_pending_wrap(&self) -> Result<bool> {
        self.get(Data::CURSOR_PENDING_WRAP)
    }
    /// Get whether the cursor is visible (DEC mode 25).
    pub fn is_cursor_visible(&self) -> Result<bool> {
        self.get(Data::CURSOR_VISIBLE)
    }
    /// Get the current SGR style of the cursor.
    ///
    /// This is the style that will be applied to newly printed characters.
    pub fn cursor_style(&self) -> Result<style::Style> {
        self.get::<ffi::Style>(Data::CURSOR_STYLE)
            .and_then(std::convert::TryInto::try_into)
    }
    /// Get the current Kitty keyboard protocol flags.
    pub fn kitty_keyboard_flags(&self) -> Result<key::KittyKeyFlags> {
        self.get::<ffi::KittyKeyFlags>(Data::KITTY_KEYBOARD_FLAGS)
            .map(key::KittyKeyFlags::from_bits_retain)
    }

    /// Get the scrollbar state for the terminal viewport.
    ///
    /// This may be expensive to calculate depending on where the viewport is
    /// (arbitrary pins are expensive). The caller should take care to only call
    /// this as needed and not too frequently.
    pub fn scrollbar(&self) -> Result<Scrollbar> {
        self.get(Data::SCROLLBAR)
    }
    /// Get the currently active screen.
    pub fn active_screen(&self) -> Result<Screen> {
        self.get(Data::ACTIVE_SCREEN)
    }
    /// Get whether any mouse tracking mode is active.
    ///
    /// Returns true if any of the mouse tracking modes (X1inner, normal, button,
    /// or any-event) are enabled.
    pub fn is_mouse_tracking(&self) -> Result<bool> {
        self.get(Data::MOUSE_TRACKING)
    }
    /// Get the terminal title as set by escape sequences (e.g. OSC inner/2).
    ///
    /// Returns a borrowed string, valid until the next call to
    /// [`Terminal::vt_write`] or [`Terminal::reset`]. An empty string is
    /// returned when no title has been set.
    pub fn title(&self) -> Result<&str> {
        let str = self.get::<ffi::String>(Data::TITLE)?;
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
        let str = self.get::<ffi::String>(Data::PWD)?;
        // SAFETY: We trust libghostty to return a valid borrowed string,
        // while we uphold that no mutation could happen during its lifetime.
        let str = unsafe { std::slice::from_raw_parts(str.ptr, str.len) };
        std::str::from_utf8(str).map_err(|_| Error::InvalidValue)
    }
    /// The total number of rows in the active screen including scrollback.
    pub fn total_rows(&self) -> Result<usize> {
        self.get(Data::TOTAL_ROWS)
    }
    ///  The number of scrollback rows (total rows minus viewport rows).
    pub fn scrollback_rows(&self) -> Result<usize> {
        self.get(Data::SCROLLBACK_ROWS)
    }

    /// The effective foreground color (override or default).
    pub fn fg_color(&self) -> Result<Option<RgbColor>> {
        self.get_optional::<ffi::ColorRgb>(Data::COLOR_FOREGROUND)
            .map(|v| v.map(Into::into))
    }
    /// The default foreground color (ignoring any OSC override).
    pub fn default_fg_color(&self) -> Result<Option<RgbColor>> {
        self.get_optional::<ffi::ColorRgb>(Data::COLOR_FOREGROUND_DEFAULT)
            .map(|v| v.map(Into::into))
    }
    /// Set the default foreground color.
    pub fn set_default_fg_color(&mut self, v: Option<RgbColor>) -> Result<&mut Self> {
        self.set_optional(Opt::COLOR_FOREGROUND, v.map(ffi::ColorRgb::from).as_ref())?;
        Ok(self)
    }

    /// The effective background color (override or default).
    pub fn bg_color(&self) -> Result<Option<RgbColor>> {
        self.get_optional::<ffi::ColorRgb>(Data::COLOR_BACKGROUND)
            .map(|v| v.map(Into::into))
    }
    /// The default background color (ignoring any OSC override).
    pub fn default_bg_color(&self) -> Result<Option<RgbColor>> {
        self.get_optional::<ffi::ColorRgb>(Data::COLOR_BACKGROUND_DEFAULT)
            .map(|v| v.map(Into::into))
    }
    /// Set the default background color.
    pub fn set_default_bg_color(&mut self, v: Option<RgbColor>) -> Result<&mut Self> {
        self.set_optional(Opt::COLOR_BACKGROUND, v.map(ffi::ColorRgb::from).as_ref())?;
        Ok(self)
    }

    /// The effective cursor color (override or default).
    pub fn cursor_color(&self) -> Result<Option<RgbColor>> {
        self.get_optional::<ffi::ColorRgb>(Data::COLOR_CURSOR)
            .map(|v| v.map(Into::into))
    }
    /// The default cursor color (ignoring any OSC override).
    pub fn default_cursor_color(&self) -> Result<Option<RgbColor>> {
        self.get_optional::<ffi::ColorRgb>(Data::COLOR_CURSOR_DEFAULT)
            .map(|v| v.map(Into::into))
    }
    /// Set the default cursor color.
    pub fn set_default_cursor_color(&mut self, v: Option<RgbColor>) -> Result<&mut Self> {
        self.set_optional(Opt::COLOR_CURSOR, v.map(ffi::ColorRgb::from).as_ref())?;
        Ok(self)
    }

    /// The current 256-color palette.
    pub fn color_palette(&self) -> Result<[RgbColor; 256]> {
        self.get::<[ffi::ColorRgb; 256]>(Data::COLOR_PALETTE)
            .map(|v| v.map(Into::into))
    }
    /// The default 256-color palette (ignoring any OSC overrides).
    pub fn default_color_palette(&self) -> Result<[RgbColor; 256]> {
        self.get::<[ffi::ColorRgb; 256]>(Data::COLOR_PALETTE_DEFAULT)
            .map(|v| v.map(Into::into))
    }
    /// Set the default 256-color palette.
    pub fn set_default_color_palette(&mut self, v: Option<[RgbColor; 256]>) -> Result<&mut Self> {
        self.set_optional(
            Opt::COLOR_PALETTE,
            v.map(|v| v.map(ffi::ColorRgb::from)).as_ref(),
        )?;
        Ok(self)
    }

    /// Set the maximum bytes the APC handler will buffer for all protocols.
    ///
    /// This prevents malicious input from causing unbounded memory allocation.
    /// A `None` value removes all overrides, reverting to the built-in defaults.
    pub fn set_apc_max_bytes(&mut self, max: Option<usize>) -> Result<&mut Self> {
        self.set_optional(ffi::TerminalOption::APC_MAX_BYTES, max.as_ref())?;
        Ok(self)
    }
}
impl Drop for Terminal<'_, '_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.inner.as_raw()) }
    }
}

/// A point in the terminal grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// Active area where the cursor can move.
    Active(PointCoordinate),
    /// Visible viewport (changes when scrolled).
    Viewport(PointCoordinate),
    /// Full screen including scrollback.
    Screen(PointCoordinate),
    /// Scrollback history only (before active area).
    History(PointCoordinate),
}

impl From<Point> for ffi::Point {
    fn from(value: Point) -> Self {
        match value {
            Point::Active(coord) => Self {
                tag: ffi::PointTag::ACTIVE,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
            Point::Viewport(coord) => Self {
                tag: ffi::PointTag::VIEWPORT,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
            Point::Screen(coord) => Self {
                tag: ffi::PointTag::SCREEN,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
            Point::History(coord) => Self {
                tag: ffi::PointTag::HISTORY,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
        }
    }
}

/// A coordinate in the terminal grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointCoordinate {
    /// Column (0-indexed).
    pub x: u16,
    /// Row (0-indexed). May exceed page size for screen/history tags.
    pub y: u32,
}
impl From<PointCoordinate> for ffi::PointCoordinate {
    fn from(value: PointCoordinate) -> Self {
        let PointCoordinate { x, y } = value;
        Self { x, y }
    }
}
impl From<ffi::PointCoordinate> for PointCoordinate {
    fn from(value: ffi::PointCoordinate) -> Self {
        let ffi::PointCoordinate { x, y } = value;
        Self { x, y }
    }
}

/// Scroll viewport behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollViewport {
    /// Scroll to the top of the scrollback.
    Top,
    /// Scroll to the bottom (active area).
    Bottom,
    /// Scroll by a delta amount (up is negative).
    Delta(isize),
}
impl From<ScrollViewport> for ffi::TerminalScrollViewport {
    fn from(value: ScrollViewport) -> Self {
        match value {
            ScrollViewport::Top => Self {
                tag: ffi::TerminalScrollViewportTag::TOP,
                value: ffi::TerminalScrollViewportValue::default(),
            },
            ScrollViewport::Bottom => Self {
                tag: ffi::TerminalScrollViewportTag::BOTTOM,
                value: ffi::TerminalScrollViewportValue::default(),
            },
            ScrollViewport::Delta(delta) => Self {
                tag: ffi::TerminalScrollViewportTag::DELTA,
                value: {
                    let mut v = ffi::TerminalScrollViewportValue::default();
                    v.delta = delta;
                    v
                },
            },
        }
    }
}

/// A terminal mode consisting of its value and its kind (DEC/ANSI).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Mode(pub ffi::Mode);

impl Mode {
    #![expect(missing_docs, reason = "no upstream documentation provided")]
    const ANSI_BIT: u16 = 1 << 15;

    /// Create a new mode from its numeric value and its kind.
    #[must_use]
    pub const fn new(v: u16, kind: ModeKind) -> Self {
        match kind {
            ModeKind::Ansi => Self(v | Self::ANSI_BIT),
            ModeKind::Dec => Self(v),
        }
    }

    /// The numeric value of the mode.
    #[must_use]
    pub fn value(self) -> u16 {
        (self.0) & 0x7fff
    }

    /// The kind of the mode (DEC/ANSI).
    #[must_use]
    pub fn kind(self) -> ModeKind {
        if (self.0) & Self::ANSI_BIT > 0 {
            ModeKind::Ansi
        } else {
            ModeKind::Dec
        }
    }

    pub const KAM: Self = Self::new(2, ModeKind::Ansi);
    pub const INSERT: Self = Self::new(4, ModeKind::Ansi);
    pub const SRM: Self = Self::new(12, ModeKind::Ansi);
    pub const LINEFEED: Self = Self::new(20, ModeKind::Ansi);

    pub const DECCKM: Self = Self::new(1, ModeKind::Dec);
    pub const _132_COLUMN: Self = Self::new(3, ModeKind::Dec);
    pub const SLOW_SCROLL: Self = Self::new(4, ModeKind::Dec);
    pub const REVERSE_COLORS: Self = Self::new(5, ModeKind::Dec);
    pub const ORIGIN: Self = Self::new(6, ModeKind::Dec);
    pub const WRAPAROUND: Self = Self::new(7, ModeKind::Dec);
    pub const AUTOREPEAT: Self = Self::new(8, ModeKind::Dec);
    pub const X10_MOUSE: Self = Self::new(9, ModeKind::Dec);
    pub const CURSOR_BLINKING: Self = Self::new(12, ModeKind::Dec);
    pub const CURSOR_VISIBLE: Self = Self::new(25, ModeKind::Dec);
    pub const ENABLE_MODE3: Self = Self::new(40, ModeKind::Dec);
    pub const REVERSE_WRAP: Self = Self::new(45, ModeKind::Dec);
    pub const ALT_SCREEN_LEGACY: Self = Self::new(47, ModeKind::Dec);
    pub const KEYPAD_KEYS: Self = Self::new(66, ModeKind::Dec);
    pub const LEFT_RIGHT_MARGIN: Self = Self::new(69, ModeKind::Dec);
    pub const NORMAL_MOUSE: Self = Self::new(1000, ModeKind::Dec);
    pub const BUTTON_MOUSE: Self = Self::new(1002, ModeKind::Dec);
    pub const ANY_MOUSE: Self = Self::new(1003, ModeKind::Dec);
    pub const FOCUS_EVENT: Self = Self::new(1004, ModeKind::Dec);
    pub const UTF8_MOUSE: Self = Self::new(1005, ModeKind::Dec);
    pub const SGR_MOUSE: Self = Self::new(1006, ModeKind::Dec);
    pub const ALT_SCROLL: Self = Self::new(1007, ModeKind::Dec);
    pub const URXVT_MOUSE: Self = Self::new(1015, ModeKind::Dec);
    pub const SGR_PIXELS_MOUSE: Self = Self::new(1016, ModeKind::Dec);
    pub const NUMLOCK_KEYPAD: Self = Self::new(1035, ModeKind::Dec);
    pub const ALT_ESC_PREFIX: Self = Self::new(1036, ModeKind::Dec);
    pub const ALT_SENDS_ESC: Self = Self::new(1039, ModeKind::Dec);
    pub const REVERSE_WRAP_EXT: Self = Self::new(1045, ModeKind::Dec);
    pub const ALT_SCREEN: Self = Self::new(1047, ModeKind::Dec);
    pub const SAVE_CURSOR: Self = Self::new(1048, ModeKind::Dec);
    pub const ALT_SCREEN_SAVE: Self = Self::new(1049, ModeKind::Dec);
    pub const BRACKETED_PASTE: Self = Self::new(2004, ModeKind::Dec);
    pub const SYNC_OUTPUT: Self = Self::new(2026, ModeKind::Dec);
    pub const GRAPHEME_CLUSTER: Self = Self::new(2027, ModeKind::Dec);
    pub const COLOR_SCHEME_REPORT: Self = Self::new(2031, ModeKind::Dec);
    pub const IN_BAND_RESIZE: Self = Self::new(2048, ModeKind::Dec);
}

/// The kind of a terminal mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ModeKind {
    /// DEC terminal mode.
    Dec,
    /// ANSI terminal mode.
    Ansi,
}

impl From<Mode> for ffi::Mode {
    fn from(value: Mode) -> Self {
        value.0
    }
}

/// Device attributes response data for all three DA levels.
/// Filled by the [`Terminal::on_device_attributes`] callback in response
/// to CSI c, CSI > c, or CSI = c queries. The terminal uses whichever
/// sub-struct matches the request type.
#[derive(Debug, Clone, Copy)]
pub struct DeviceAttributes {
    /// Primary device attributes (DA1).
    pub primary: PrimaryDeviceAttributes,
    /// Secondary device attributes (DA2).
    pub secondary: SecondaryDeviceAttributes,
    /// Tertiary device attributes (DA3).
    pub tertiary: TertiaryDeviceAttributes,
}

impl From<DeviceAttributes> for ffi::DeviceAttributes {
    fn from(value: DeviceAttributes) -> Self {
        Self {
            primary: value.primary.into(),
            secondary: value.secondary.into(),
            tertiary: value.tertiary.into(),
        }
    }
}

/// Primary device attributes (DA1) response data.
///
/// Returned as part of [`DeviceAttributes`] in response to a CSI c query.
#[derive(Debug, Clone, Copy)]
pub struct PrimaryDeviceAttributes(ffi::DeviceAttributesPrimary);

impl PrimaryDeviceAttributes {
    /// Construct primary device attributes from a conformance level
    /// and an array of device attribute features.
    ///
    /// # Panics
    ///
    /// **Panics** when more than 64 features are given.
    #[must_use]
    pub fn new<const N: usize>(
        conformance_level: ConformanceLevel,
        features: [DeviceAttributeFeature; N],
    ) -> Self {
        assert!(N <= 64);

        let mut f = [0u16; 64];
        f[..N].copy_from_slice(features.map(|f| f.0).as_slice());

        Self(ffi::DeviceAttributesPrimary {
            conformance_level: conformance_level.0,
            features: f,
            num_features: N,
        })
    }
}

impl From<PrimaryDeviceAttributes> for ffi::DeviceAttributesPrimary {
    fn from(value: PrimaryDeviceAttributes) -> Self {
        value.0
    }
}

/// The level of conformance to the behavior of a specific or a family of
/// physical terminal models.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConformanceLevel(pub u16);

impl ConformanceLevel {
    #![expect(clippy::doc_markdown, reason = "false positive")]
    #![expect(missing_docs, reason = "self-explanatory")]
    pub const VT100: Self = Self(ffi::DA_CONFORMANCE_VT100);
    pub const VT101: Self = Self(ffi::DA_CONFORMANCE_VT101);
    pub const VT102: Self = Self(ffi::DA_CONFORMANCE_VT102);
    pub const VT125: Self = Self(ffi::DA_CONFORMANCE_VT125);
    pub const VT131: Self = Self(ffi::DA_CONFORMANCE_VT131);
    pub const VT132: Self = Self(ffi::DA_CONFORMANCE_VT132);
    pub const VT220: Self = Self(ffi::DA_CONFORMANCE_VT220);
    pub const VT240: Self = Self(ffi::DA_CONFORMANCE_VT240);
    pub const VT320: Self = Self(ffi::DA_CONFORMANCE_VT320);
    pub const VT340: Self = Self(ffi::DA_CONFORMANCE_VT340);
    pub const VT420: Self = Self(ffi::DA_CONFORMANCE_VT420);
    pub const VT510: Self = Self(ffi::DA_CONFORMANCE_VT510);
    pub const VT520: Self = Self(ffi::DA_CONFORMANCE_VT520);
    pub const VT525: Self = Self(ffi::DA_CONFORMANCE_VT525);
    /// Equivalent to a VT2xx terminal.
    pub const LEVEL_2: Self = Self(ffi::DA_CONFORMANCE_LEVEL_2);
    /// Equivalent to a VT3xx terminal.
    pub const LEVEL_3: Self = Self(ffi::DA_CONFORMANCE_LEVEL_3);
    /// Equivalent to a VT4xx terminal.
    pub const LEVEL_4: Self = Self(ffi::DA_CONFORMANCE_LEVEL_4);
    /// Equivalent to a VT5xx terminal.
    pub const LEVEL_5: Self = Self(ffi::DA_CONFORMANCE_LEVEL_5);
}

/// A feature that a terminal can report to support.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceAttributeFeature(pub u16);

impl DeviceAttributeFeature {
    #![expect(missing_docs, reason = "no upstream documentation provided")]
    pub const COLUMNS_132: Self = Self(ffi::DA_FEATURE_COLUMNS_132);
    pub const PRINTER: Self = Self(ffi::DA_FEATURE_PRINTER);
    pub const REGIS: Self = Self(ffi::DA_FEATURE_REGIS);
    pub const SIXEL: Self = Self(ffi::DA_FEATURE_SIXEL);
    pub const SELECTIVE_ERASE: Self = Self(ffi::DA_FEATURE_SELECTIVE_ERASE);
    pub const USER_DEFINED_KEYS: Self = Self(ffi::DA_FEATURE_USER_DEFINED_KEYS);
    pub const NATIONAL_REPLACEMENT: Self = Self(ffi::DA_FEATURE_NATIONAL_REPLACEMENT);
    pub const TECHNICAL_CHARACTERS: Self = Self(ffi::DA_FEATURE_TECHNICAL_CHARACTERS);
    pub const LOCATOR: Self = Self(ffi::DA_FEATURE_LOCATOR);
    pub const TERMINAL_STATE: Self = Self(ffi::DA_FEATURE_TERMINAL_STATE);
    pub const WINDOWING: Self = Self(ffi::DA_FEATURE_WINDOWING);
    pub const HORIZONTAL_SCROLLING: Self = Self(ffi::DA_FEATURE_HORIZONTAL_SCROLLING);
    pub const ANSI_COLOR: Self = Self(ffi::DA_FEATURE_ANSI_COLOR);
    pub const RECTANGULAR_EDITING: Self = Self(ffi::DA_FEATURE_RECTANGULAR_EDITING);
    pub const ANSI_TEXT_LOCATOR: Self = Self(ffi::DA_FEATURE_ANSI_TEXT_LOCATOR);
    pub const CLIPBOARD: Self = Self(ffi::DA_FEATURE_CLIPBOARD);
}

/// Secondary device attributes (DA2) response data.
///
/// Returned as part of [`DeviceAttributes`] in response to a CSI > c query.
/// Response format: CSI > Pp ; Pv ; Pc c
#[derive(Debug, Copy, Clone)]
pub struct SecondaryDeviceAttributes {
    /// Terminal type identifier (Pp).
    pub device_type: DeviceType,
    /// Firmware/patch version number (Pv).
    pub firmware_version: u16,
    /// ROM cartridge registration number (Pc). Always 0 for emulators.
    pub rom_cartridge: u16,
}

impl From<SecondaryDeviceAttributes> for ffi::DeviceAttributesSecondary {
    fn from(value: SecondaryDeviceAttributes) -> Self {
        Self {
            device_type: value.device_type.0,
            firmware_version: value.firmware_version,
            rom_cartridge: value.rom_cartridge,
        }
    }
}

/// The type of terminal device being emulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceType(pub u16);

impl DeviceType {
    #![expect(missing_docs, reason = "self-explanatory")]
    pub const VT100: Self = Self(ffi::DA_DEVICE_TYPE_VT100);
    pub const VT220: Self = Self(ffi::DA_DEVICE_TYPE_VT220);
    pub const VT240: Self = Self(ffi::DA_DEVICE_TYPE_VT240);
    pub const VT330: Self = Self(ffi::DA_DEVICE_TYPE_VT330);
    pub const VT340: Self = Self(ffi::DA_DEVICE_TYPE_VT340);
    pub const VT320: Self = Self(ffi::DA_DEVICE_TYPE_VT320);
    pub const VT382: Self = Self(ffi::DA_DEVICE_TYPE_VT382);
    pub const VT420: Self = Self(ffi::DA_DEVICE_TYPE_VT420);
    pub const VT510: Self = Self(ffi::DA_DEVICE_TYPE_VT510);
    pub const VT520: Self = Self(ffi::DA_DEVICE_TYPE_VT520);
    pub const VT525: Self = Self(ffi::DA_DEVICE_TYPE_VT525);
}

/// Tertiary device attributes (DA3) response data.
///
/// Returned as part of [`DeviceAttributes`] in response to a CSI = c query.
/// Response format: DCS ! | D...D ST (DECRPTUI).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TertiaryDeviceAttributes {
    /// Unit ID encoded as 8 uppercase hex digits in the response.
    pub unit_id: u32,
}

impl From<TertiaryDeviceAttributes> for ffi::DeviceAttributesTertiary {
    fn from(value: TertiaryDeviceAttributes) -> Self {
        Self {
            unit_id: value.unit_id,
        }
    }
}

/// Color scheme reported in response to a CSI ? 996 n query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
#[expect(missing_docs, reason = "self-explanatory")]
pub enum ColorScheme {
    Light = ffi::ColorScheme::LIGHT,
    Dark = ffi::ColorScheme::DARK,
}

//---------------------------------------
// Callbacks
//---------------------------------------

/// You might be wondering just what the heck this is.
///
/// Truth to be told, you don't need to understand how it works
/// in order to use it. It does a bunch of voodoo behind the scenes
/// that make sure all the invariants of the C API are upheld, while
/// providing a convenient API for Rust users.
///
/// Each handler is defined in this following format:
/// ```ignore
/// pub fn on_foobar(
///     &mut self,
///     // The corresponding GhosttyTerminalOption
///     tag = FOOBAR,
///
///     // The name of the original function type in C,
///     // along with the extra C parameters and the expected C return type
///     from = GhosttyTerminalFoobarFn(foo: *const u8, bar: usize) -> bool,
///
///     // The name of mapped Rust function type,
///     // along with the Rust parameters and return type.
///     //
///     // `<'t>` is used to tie the return value to the lifetime of the
///     // terminal. The name is arbitrary - any lifetime marker will do.
///     to = <'t>FoobarFn(&'t [u8]) -> bool,
/// ) |term, func| {
///     // `term` is the terminal and `func` is the Rust callback.
///     // Both names are arbitrary.
///
///     // Convert the raw parameters into Rust types.
///     // This is just to illustrate how.
///     let slice = unsafe { std::slice::from_raw_parts(foo, bar) };
///
///     // Call into user logic and return.
///     func(&terminal, slice)
/// }
/// ```
macro_rules! handlers {
    {
        $(
            $(#[$fmeta:meta])*
            $vis:vis fn $name:ident(
                &mut self,
                tag = $tag:ident,
                from = $rawfnty:ident( $($rfname:ident: $rfty:ty),*$(,)? ) $(-> $rawrty:ty)?,
                $(#[$tmeta:meta])*
                to = $(<$lf:lifetime>)? $fnty:ident( $($fty:ty),*$(,)? ) $(-> $rty:ty)?,
            ) |$t:ident, $func:ident| $block:block
        )*
    } => {
        impl<'alloc, 'cb> $crate::terminal::Terminal<'alloc, 'cb> {$(
            $(#[$fmeta])*
            $vis fn $name(&mut self, f: impl $fnty<'alloc, 'cb>) -> $crate::error::Result<&mut Self> {
                unsafe extern "C" fn callback(
                    t: $crate::ffi::Terminal,
                    ud: *mut std::ffi::c_void,
                    $($rfname: $rfty),*
                ) $(-> $rawrty)? {
                    // SAFETY: USERDATA is set to the boxed VTable pointee
                    // (derived from a mutable reference for write provenance)
                    // before the callback is registered. ghostty invokes
                    // callbacks synchronously during vt_write, so the VTable
                    // remains alive and exclusively accessed for the duration
                    // of this call.
                    let vtable = unsafe { &mut *ud.cast::<VTable<'_, '_>>() };

                    let obj = $crate::alloc::Object::new(t).expect("received null terminal ptr in callback - this is a bug!");
                    // Build a temporary borrowed Terminal view for the callback
                    // without taking ownership of the underlying ghostty terminal.
                    let mut term = ::core::mem::ManuallyDrop::new($crate::terminal::Terminal::<'_, '_> {
                        inner: obj,
                        vtable: ::core::default::Default::default(),
                    });
                    let $t: &$crate::terminal::Terminal = &term;
                    let $func = vtable.$name.as_deref_mut()
                        .expect("no handler set but callback is still called - this is a bug!");
                    let ret = $block;

                    // SAFETY: The temporary vtable was allocated solely to satisfy
                    // the Terminal layout expected by the callback signature. Drop
                    // it explicitly while intentionally leaving the borrowed
                    // terminal handle itself untouched.
                    unsafe { ::core::ptr::drop_in_place(&mut term.vtable) };

                    ret
                }

                self.vtable.$name = Some(::std::boxed::Box::new(f));

                // USERDATA is a raw pointer option: pass the heap allocation
                // itself, not the address of the Box smart pointer field stored
                // inline in Terminal.
                //
                // Derive the pointer from a mutable reference so it carries
                // write provenance – the callback later reborrows it as &mut.
                let userdata = std::ptr::from_mut::<VTable<'alloc, 'cb>>(self.vtable.as_mut())
                    as *const ::std::ffi::c_void;
                self.set_ptr($crate::ffi::TerminalOption::USERDATA, userdata)?;

                // The callback must be coerced into a function *pointer*
                // and not a function *item* (which is a ZST whose address is meaningless).
                // :)
                let callback_ptr: unsafe extern "C" fn(
                    $crate::ffi::Terminal,
                    *mut ::std::ffi::c_void,
                    $($rfty),*
                ) $(-> $rawrty)? = callback;

                let result = unsafe {
                    $crate::ffi::ghostty_terminal_set(
                        self.inner.as_raw(),
                        $crate::ffi::TerminalOption::$tag,
                        callback_ptr as *const ::std::ffi::c_void
                    )
                };
                $crate::error::from_result(result)?;
                Ok(self)
            }
        )*}
        $(
            #[doc = concat!(
                "Callback type for [`Terminal::",
                stringify!($name),
                "`](Terminal::",
                stringify!($name),
                ").\n"
            )]
            $(#[$tmeta])*
            pub trait $fnty<'alloc, 'cb>:
                $(for<$lf>)? FnMut(
                    &$($lf)? $crate::terminal::Terminal<'alloc, 'cb>,
                    $($fty),*
                ) $(-> $rty)? + 'cb {}

            impl<'alloc, 'cb, F> $fnty<'alloc, 'cb> for F
            where
                F: $(for<$lf>)? FnMut(
                    &$($lf)? $crate::terminal::Terminal<'alloc, 'cb>,
                    $($fty),*
                ) $(-> $rty)? + 'cb
            {}
        )*

        struct VTable<'alloc, 'cb> {
            $($name: Option<::std::boxed::Box<dyn $fnty<'alloc, 'cb>>>),*
        }

        impl ::core::fmt::Debug for VTable<'_, '_> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                f.write_str("VTable {..}")
            }
        }

        impl ::core::default::Default for VTable<'_, '_> {
            fn default() -> Self {
                Self {
                    $($name: None),*
                }
            }
        }
    };
}

handlers! {
    /// Call the given function when the terminal needs to write data back
    /// to the pty (e.g. in response to a DECRQM query or device status report).
    pub fn on_pty_write(
        &mut self,
        tag = WRITE_PTY,
        from = GhosttyTerminalWritePtyFn(ptr: *const u8, len: usize),
        to = <'t>PtyWriteFn(&'t [u8]),
    ) |term, func| {
        // SAFETY: We trust libghostty to return valid memory given we
        // uphold all lifetime invariants (e.g. no `vt_write` calls
        // during this callback, which is guaranteed via the mutable reference).
        let data = unsafe { std::slice::from_raw_parts(ptr, len) };
        func(&term, data);
    }

    /// Call the given function when the terminal receives
    /// a BEL character (0x07).
    pub fn on_bell(
        &mut self,
        tag = BELL,
        from = GhosttyTerminalBellFn(),
        to = BellFn(),
    ) |term, func| {
        func(&term);
    }

    /// Call the given function when the terminal receives
    /// an ENQ character (0x05).
    pub fn on_enquiry(
        &mut self,
        tag = ENQUIRY,
        from = GhosttyTerminalEnquiryFn() -> ffi::String,
        to = <'t>EnquiryFn() -> Option<&'t str>,
    ) |term, func| {
        func(&term).unwrap_or("").into()
    }

    /// Call the given function when the terminal receives an XTVERSION
    /// query (CSI > q), and respond with the resulting version string
    /// (e.g. "myterm 1.0").
    pub fn on_xtversion(
        &mut self,
        tag = XTVERSION,
        from = GhosttyTerminalXtversionFn() -> ffi::String,
        to = <'t>XtversionFn() -> Option<&'t str>,
    ) |term, func| {
        func(&term).unwrap_or("").into()
    }

    /// Call the given function when the terminal title changes
    /// via escape sequences (e.g. OSC 0 or OSC 2).
    ///
    /// The new title can be queried from the terminal after
    /// the callback returns.
    pub fn on_title_changed(
        &mut self,
        tag = TITLE_CHANGED,
        from = GhosttyTerminalTitleChangedFn(),
        to = TitleChangedFn(),
    ) |term, func| {
        func(&term);
    }

    /// Call the given function in response to XTWINOPS size queries
    /// (CSI 14/16/18 t).
    pub fn on_size(
        &mut self,
        tag = SIZE,
        from = GhosttyTerminalSizeFn(out: *mut ffi::SizeReportSize) -> bool,
        to = SizeFn() -> Option<SizeReportSize>,
    ) |term, func| {
        if let Some(size) = func(&term) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size };
            true
        } else {
            false
        }
    }

    /// Call the given function in response to a color scheme
    /// device status report query (CSI ? 996 n).
    ///
    /// Return `Some` to report the current color scheme,
    /// or return `None` to silently ignore.
    pub fn on_color_scheme(
        &mut self,
        tag = COLOR_SCHEME,
        from = GhosttyTerminalColorSchemeFn(out: *mut ffi::ColorScheme::Type) -> bool,
        to = ColorSchemeFn() -> Option<ColorScheme>,
    ) |term, func| {
        if let Some(size) = func(&term) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size as ffi::ColorScheme::Type };
            true
        } else {
            false
        }
    }

    /// Call the given function in response to a device attributes query
    /// (CSI c, CSI > c, or CSI = c).
    ///
    /// Return `Some` with the response data,
    /// or return `None` to silently ignore.
    pub fn on_device_attributes(
        &mut self,
        tag = DEVICE_ATTRIBUTES,
        from = GhosttyTerminalDeviceAttributesFn(out: *mut ffi::DeviceAttributes) -> bool,
        to = DeviceAttributesFn() -> Option<DeviceAttributes>,
    ) |term, func| {
        if let Some(size) = func(&term) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size.into() };
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::mem::ManuallyDrop;
    use std::rc::Rc;

    #[inline(never)]
    fn build_terminal(callback_count: Rc<Cell<usize>>) -> Terminal<'static, 'static> {
        let mut terminal = Terminal::new(Options {
            cols: 80,
            rows: 24,
            max_scrollback: 1000,
        })
        .expect("terminal should initialize");

        terminal
            .on_device_attributes(move |_term| {
                callback_count.set(callback_count.get() + 1);
                Some(DeviceAttributes {
                    primary: PrimaryDeviceAttributes::new(
                        ConformanceLevel::VT220,
                        [DeviceAttributeFeature::ANSI_COLOR],
                    ),
                    secondary: SecondaryDeviceAttributes {
                        device_type: DeviceType::VT220,
                        firmware_version: 1,
                        rom_cartridge: 0,
                    },
                    tertiary: TertiaryDeviceAttributes { unit_id: 0 },
                })
            })
            .expect("callback should register");

        terminal
    }

    /// Move a value into distinct heap storage with an explicit byte-for-byte
    /// relocation so the test does not rely on optimizer or allocator behavior.
    fn relocate_into_new_box<T>(value: T) -> (Box<T>, usize, usize) {
        // Keep the source allocation alive without running T's destructor.
        // We need the bytes to remain initialized until after the copy.
        let src = Box::new(ManuallyDrop::new(value));
        let src_addr = std::ptr::from_ref(&**src).cast::<T>() as usize;

        unsafe {
            let dst_layout = std::alloc::Layout::new::<T>();
            let dst_ptr = std::alloc::alloc(dst_layout).cast::<T>();
            if dst_ptr.is_null() {
                std::alloc::handle_alloc_error(dst_layout);
            }

            let dst_addr = dst_ptr as usize;
            assert_ne!(
                src_addr, dst_addr,
                "test setup failed: source and destination storage unexpectedly match"
            );

            // SAFETY: src points to a fully initialized T wrapped in
            // ManuallyDrop, dst points to distinct uninitialized storage for
            // exactly one T, and the regions do not overlap.
            std::ptr::copy_nonoverlapping(std::ptr::from_ref(&**src).cast::<T>(), dst_ptr, 1);

            // SAFETY: src was allocated as Box<ManuallyDrop<T>> and must be
            // freed without dropping T because ownership was transferred by
            // the raw byte copy above.
            std::alloc::dealloc(
                Box::into_raw(src).cast::<u8>(),
                std::alloc::Layout::new::<ManuallyDrop<T>>(),
            );

            // SAFETY: We just initialized dst_ptr by copying a valid T into it,
            // so it now owns exactly one initialized T allocation.
            (Box::from_raw(dst_ptr), src_addr, dst_addr)
        }
    }

    /// Explicitly relocate the Terminal into distinct storage, then verify the
    /// callback still fires through the stable VTable userdata pointer.
    #[test]
    fn callbacks_survive_explicit_relocation() {
        let callback_count = Rc::new(Cell::new(0usize));
        let terminal = build_terminal(callback_count.clone());
        let (mut terminal, addr_before, addr_after) = relocate_into_new_box(terminal);
        assert_ne!(addr_before, addr_after);

        // Primary DA request (CSI c) should invoke on_device_attributes.
        terminal.vt_write(b"\x1b[c");
        assert_eq!(callback_count.get(), 1);
    }
}
