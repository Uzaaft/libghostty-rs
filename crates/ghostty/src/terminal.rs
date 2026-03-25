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
pub struct Terminal<'alloc, 'ud, UserData> {
    pub(crate) inner: Object<'alloc, ffi::GhosttyTerminal>,
    cbs: Callbacks<'alloc, 'ud, UserData>,
}

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

impl<'alloc: 'ud, 'ud, UserData: 'ud> Terminal<'alloc, 'ud, UserData> {
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
        Ok(Self {
            inner: Object::new(raw)?,
            cbs: Default::default(),
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
    /// in-band size report if mode 2inner48 is enabled.
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

    /// Get the current value of a terminal mode.
    pub fn mode(&self, mode: Mode) -> Result<bool> {
        let mut value = false;
        let result =
            unsafe { ffi::ghostty_terminal_mode_get(self.inner.as_raw(), mode.into(), &mut value) };
        from_result(result)?;
        Ok(value)
    }

    /// Set the value of a terminal mode.
    pub fn set_mode(&mut self, mode: Mode, value: bool) -> Result<()> {
        let result =
            unsafe { ffi::ghostty_terminal_mode_set(self.inner.as_raw(), mode.into(), value) };
        from_result(result)
    }

    fn get<T>(&self, tag: ffi::GhosttyTerminalData) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_terminal_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    fn set<T>(&self, tag: ffi::GhosttyTerminalOption, v: &T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_terminal_set(self.inner.as_raw(), tag, std::ptr::from_ref(v).cast())
        };
        from_result(result)
    }

    /// Get the terminal width in cells.
    pub fn cols(&self) -> Result<u16> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_COLS)
    }
    /// Get the terminal height in cells.
    pub fn rows(&self) -> Result<u16> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_ROWS)
    }
    /// Get the cursor column position (inner-indexed).
    pub fn cursor_x(&self) -> Result<u16> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_CURSOR_X)
    }
    /// Get the cursor row position within the active area (inner-indexed).
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
    /// Returns true if any of the mouse tracking modes (X1inner, normal, button,
    /// or any-event) are enabled.
    pub fn is_mouse_tracking(&self) -> Result<bool> {
        self.get(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING)
    }
    /// Get the terminal title as set by escape sequences (e.g. OSC inner/2).
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

    fn update_cbs(&mut self) -> Result<()> {
        self.set::<Callbacks<'alloc, 'ud, UserData>>(
            ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_USERDATA,
            &self.cbs,
        )
    }

    /// Set the user data passed to all callbacks.
    pub fn set_userdata(&mut self, ud: &'ud mut UserData) -> Result<()> {
        self.cbs.ud = Some(ud);
        self.update_cbs()
    }
}

impl<UserData> Drop for Terminal<'_, '_, UserData> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.inner.as_raw()) }
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
    X1innerMouse = 9,
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

//---------------------------------------
// Callbacks
//---------------------------------------

macro_rules! handlers {
    {
        $(
            $(#[$fmeta:meta])*
            $vis:vis fn $name:ident(
                &mut self,
                tag = $tag:ident,
                from = $rawfnty:ident( $($rfname:ident: $rfty:ty),*$(,)? ) -> $rawrty:ty,
                $(#[$tmeta:meta])*
                to = $(<$lf:lifetime>)? $fnty:ident( $($fty:ty),*$(,)? ) $(-> $rty:ty)?,
            ) |$prep:ident| $block:block
        )*
    } => {
        impl<'alloc: 'ud, 'ud, UserData: 'ud> Terminal<'alloc, 'ud, UserData> {$(
            $(#[$fmeta])*
            $vis fn $name(&mut self, f: Option<$fnty<'alloc, 'ud, UserData>>) -> Result<()> {
                unsafe extern "C" fn callback<'alloc: 'ud, 'ud, UserData: 'ud>(
                    t: *mut $crate::ffi::GhosttyTerminal,
                    ud: *mut ::std::ffi::c_void,
                    $($rfname: $rfty),*
                ) -> $rawrty {
                    let $prep = unsafe { prep_callback::<'alloc, 'ud, UserData>(t, ud) }
                        .and_then(|(t, cbs)| Some((t, cbs.ud.as_deref_mut(), cbs.$name.as_deref()?)));
                    $block
                }

                if let Some(f) = f {
                    self.cbs.$name = Some(f);
                    self.update_cbs()?;

                    self.set::<$crate::ffi::$rawfnty>(
                        $crate::ffi::$tag,
                        &Some(callback::<'alloc, 'ud, UserData>),
                    )
                } else {
                    self.cbs.$name = None;
                    self.update_cbs()?;
                    self.set::<$crate::ffi::$rawfnty>(
                        $crate::ffi::$tag,
                        &None,
                    )
                }
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
            pub type $fnty<'alloc, 'ud, UserData> =
                ::std::boxed::Box<dyn $(for<$lf>)? Fn(
                    &$($lf)? Terminal<'alloc, 'ud, UserData>,
                    ::core::option::Option<&'ud mut UserData>,
                    $($fty),*
                ) $(-> $rty)?>;
        )*

        struct Callbacks<'alloc, 'ud, UserData: 'ud> {
            ud: Option<&'ud mut UserData>,
            $($name: Option<$fnty<'alloc, 'ud, UserData>>),*
        }

        impl<'alloc, 'ud, UserData: 'ud> Default for Callbacks<'alloc, 'ud, UserData> {
            fn default() -> Self {
                Self {
                    ud: None,
                    $($name: None),*
                }
            }
        }
    };
}

handlers! {
    /// Set the callback invoked when the terminal needs to write data
    /// back to the pty (e.g. in response to a DECRQM query or device status
    /// report).
    ///
    /// Set to `None` to ignore such sequences.
    pub fn on_pty_write(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_WRITE_PTY,
        from = GhosttyTerminalWritePtyFn(ptr: *const u8, len: usize) -> (),
        to = WritePtyFn(&[u8]),
    ) |prep| {
        if let Some((t, ud, func)) = prep {
            // SAFETY: We trust libghostty to return valid memory given we
            // uphold all lifetime invariants (e.g. no `vt_write` calls
            // during this callback, which is guaranteed via the mutable reference).
            let data = unsafe { std::slice::from_raw_parts(ptr, len) };
            func(&t, ud, data);
        }
    }

    /// Set the callback invoked when the terminal
    /// receives a BEL character (0x07).
    ///
    /// Set to `None` ignore bell events.
    pub fn on_bell(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_BELL,
        from = GhosttyTerminalBellFn() -> (),
        to = BellFn(),
    ) |prep| {
        if let Some((t, ud, func)) = prep {
            func(&t, ud);
        }
    }

    /// Set the callback invoked when the terminal
    /// receives an ENQ character (0x05).
    ///
    /// Set to `None` to send no response.
    pub fn on_enquiry(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_ENQUIRY,
        from = GhosttyTerminalEnquiryFn() -> ffi::GhosttyString,
        to = <'t>EnquiryFn() -> Option<&'t str>,
    ) |prep| {
        if let Some((t, ud, func)) = prep {
            func(&t, ud).unwrap_or("").into()
        } else {
            "".into()
        }
    }

    /// Set the callback invoked when the terminal
    /// receives an XTVERSION query (CSI > q).
    ///
    /// Set to `None` to report the default "libghostty" string.
    pub fn on_xtversion(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_XTVERSION,
        from = GhosttyTerminalXtversionFn() -> ffi::GhosttyString,
        to = <'t>XtversionFn() -> Option<&'t str>,
    ) |prep| {
        if let Some((t, ud, func)) = prep {
            func(&t, ud).unwrap_or("").into()
        } else {
            "".into()
        }
    }

    /// Set the callback invoked when the terminal title changes
    /// via escape sequences (e.g. OSC 0 or OSC 2).
    ///
    /// Set to `None` to ignore title change events.
    pub fn on_title_changed(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_TITLE_CHANGED,
        from = GhosttyTerminalTitleChangedFn() -> (),
        to = TitleChanged(),
    ) |prep| {
        if let Some((t, ud, func)) = prep {
            func(&t, ud)
        }
    }

    /// Set the callback invoked in response to XTWINOPS size queries
    /// (CSI 14/16/18 t).
    ///
    /// Set to `None` to silently ignore size queries.
    pub fn on_size(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_SIZE,
        from = GhosttyTerminalSizeFn(out: *mut ffi::GhosttySizeReportSize) -> bool,
        to = SizeFn() -> Option<ffi::GhosttySizeReportSize>,
    ) |prep| {
        if let Some((t, ud, func)) = prep && let Some(size) = func(&t, ud) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size };
            true
        } else {
            false
        }
    }

    /// Set the callback invoked in response to a color scheme device status
    /// report query (CSI ? 996 n).
    ///
    /// Return `Some` to report the current scheme, or return `None` to
    /// silently ignore.
    ///
    /// Set to `None` to ignore color scheme queries.
    pub fn on_color_scheme(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_SCHEME,
        from = GhosttyTerminalColorSchemeFn(out: *mut ffi::GhosttyColorScheme) -> bool,
        to = ColorSchemeFn() -> Option<ffi::GhosttyColorScheme>,
    ) |prep| {
        if let Some((t, ud, func)) = prep && let Some(size) = func(&t, ud) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size };
            true
        } else {
            false
        }
    }

    /// Set the callback invoked in response to a device attributes query
    /// (CSI c, CSI > c, or CSI = c).
    ///
    /// Return `Some` with the response data, or return `None` to silently ignore.
    ///
    /// Set to `None` to ignore device attributes queries.
    pub fn on_device_attributes(
        &mut self,
        tag = GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_DEVICE_ATTRIBUTES,
        from = GhosttyTerminalDeviceAttributesFn(out: *mut ffi::GhosttyDeviceAttributes) -> bool,
        to = DeviceAttributesFn() -> Option<ffi::GhosttyDeviceAttributes>,
    ) |prep| {
        if let Some((t, ud, func)) = prep && let Some(size) = func(&t, ud) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size };
            true
        } else {
            false
        }
    }
}

unsafe fn prep_callback<'alloc: 'ud, 'ud, UserData: 'ud>(
    t: *mut ffi::GhosttyTerminal,
    ud: *mut std::ffi::c_void,
) -> Option<(
    Terminal<'alloc, 'ud, UserData>,
    &'ud mut Callbacks<'alloc, 'ud, UserData>,
)> {
    // SAFETY: Lifetime system should already ensure the userdata
    // reference lasts longer than 'ud here.
    let cbs = unsafe { &mut *ud.cast::<Callbacks<'alloc, 'ud, UserData>>() };

    let obj = Object::new(t).ok()?;
    let t = Terminal::<'alloc, 'ud, UserData> {
        inner: obj,
        cbs: Default::default(),
    };
    Some((t, cbs))
}
