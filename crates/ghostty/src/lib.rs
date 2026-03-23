use std::marker::PhantomData;
use std::ptr::NonNull;

pub use ghostty_sys as ffi;

pub const EXPORTED_API_SYMBOLS: &[&str] = ffi::EXPORTED_API_SYMBOLS;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Error {
    OutOfMemory,
    InvalidValue,
    OutOfSpace { required: usize },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfMemory => write!(f, "out of memory"),
            Self::InvalidValue => write!(f, "invalid value"),
            Self::OutOfSpace { required } => {
                write!(f, "out of space, {required} bytes required")
            }
        }
    }
}

impl std::error::Error for Error {}

fn from_result(code: ffi::GhosttyResult) -> Result<(), Error> {
    match code {
        ffi::GhosttyResult_GHOSTTY_SUCCESS => Ok(()),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_MEMORY => Err(Error::OutOfMemory),
        ffi::GhosttyResult_GHOSTTY_INVALID_VALUE => Err(Error::InvalidValue),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_SPACE => Err(Error::OutOfSpace { required: 0 }),
        _ => Err(Error::InvalidValue),
    }
}

fn from_result_with_len(code: ffi::GhosttyResult, len: usize) -> Result<usize, Error> {
    match code {
        ffi::GhosttyResult_GHOSTTY_SUCCESS => Ok(len),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_MEMORY => Err(Error::OutOfMemory),
        ffi::GhosttyResult_GHOSTTY_INVALID_VALUE => Err(Error::InvalidValue),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_SPACE => Err(Error::OutOfSpace { required: len }),
        _ => Err(Error::InvalidValue),
    }
}

// ---------------------------------------------------------------------------
// FormatterFormat
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatterFormat {
    Plain,
    Vt,
    Html,
}

impl FormatterFormat {
    fn to_raw(self) -> ffi::GhosttyFormatterFormat {
        match self {
            Self::Plain => ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_PLAIN,
            Self::Vt => ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_VT,
            Self::Html => ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_HTML,
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal
// ---------------------------------------------------------------------------

pub struct Terminal {
    ptr: NonNull<ffi::GhosttyTerminal>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self, Error> {
        let opts = ffi::GhosttyTerminalOptions {
            cols,
            rows,
            max_scrollback,
        };
        let mut raw: ffi::GhosttyTerminal_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_terminal_new(std::ptr::null(), &mut raw, opts) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn as_raw(&self) -> ffi::GhosttyTerminal_ptr {
        self.ptr.as_ptr()
    }

    pub fn vt_write(&mut self, data: &[u8]) {
        unsafe { ffi::ghostty_terminal_vt_write(self.ptr.as_ptr(), data.as_ptr(), data.len()) }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        let result = unsafe { ffi::ghostty_terminal_resize(self.ptr.as_ptr(), cols, rows) };
        from_result(result)
    }

    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_terminal_reset(self.ptr.as_ptr()) }
    }

    pub fn scroll_viewport_top(&mut self) {
        let behavior = ffi::GhosttyTerminalScrollViewport {
            tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_TOP,
            value: ffi::GhosttyTerminalScrollViewportValue::default(),
        };
        unsafe { ffi::ghostty_terminal_scroll_viewport(self.ptr.as_ptr(), behavior) }
    }

    pub fn scroll_viewport_bottom(&mut self) {
        let behavior = ffi::GhosttyTerminalScrollViewport {
            tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
            value: ffi::GhosttyTerminalScrollViewportValue::default(),
        };
        unsafe { ffi::ghostty_terminal_scroll_viewport(self.ptr.as_ptr(), behavior) }
    }

    pub fn scroll_viewport_delta(&mut self, delta: isize) {
        let mut value = ffi::GhosttyTerminalScrollViewportValue::default();
        value.delta = delta;
        let behavior = ffi::GhosttyTerminalScrollViewport {
            tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_DELTA,
            value,
        };
        unsafe { ffi::ghostty_terminal_scroll_viewport(self.ptr.as_ptr(), behavior) }
    }

    pub fn mode_get(&self, mode: ffi::GhosttyMode) -> Result<bool, Error> {
        let mut value = false;
        let result = unsafe { ffi::ghostty_terminal_mode_get(self.ptr.as_ptr(), mode, &mut value) };
        from_result(result)?;
        Ok(value)
    }

    pub fn mode_set(&mut self, mode: ffi::GhosttyMode, value: bool) -> Result<(), Error> {
        let result = unsafe { ffi::ghostty_terminal_mode_set(self.ptr.as_ptr(), mode, value) };
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

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------------

pub struct Formatter<'t> {
    ptr: NonNull<ffi::GhosttyFormatter>,
    _terminal: PhantomData<&'t Terminal>,
}

impl<'t> Formatter<'t> {
    pub fn new(terminal: &'t Terminal, format: FormatterFormat, trim: bool) -> Result<Self, Error> {
        let mut opts = ffi::GhosttyFormatterTerminalOptions::default();
        opts.size = std::mem::size_of::<ffi::GhosttyFormatterTerminalOptions>();
        opts.emit = format.to_raw();
        opts.trim = trim;

        let mut raw: ffi::GhosttyFormatter_ptr = std::ptr::null_mut();
        let result = unsafe {
            ffi::ghostty_formatter_terminal_new(std::ptr::null(), &mut raw, terminal.as_raw(), opts)
        };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _terminal: PhantomData,
        })
    }

    pub fn format_to_vec(&self) -> Result<Vec<u8>, Error> {
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let result = unsafe {
            ffi::ghostty_formatter_format_alloc(
                self.ptr.as_ptr(),
                std::ptr::null(),
                &mut out_ptr,
                &mut out_len,
            )
        };
        from_result(result)?;

        if out_ptr.is_null() || out_len == 0 {
            return Ok(Vec::new());
        }

        let vec = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
        unsafe { libc_free(out_ptr.cast()) };
        Ok(vec)
    }
}

impl Drop for Formatter<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.ptr.as_ptr()) }
    }
}

unsafe extern "C" {
    #[link_name = "free"]
    fn libc_free(ptr: *mut std::ffi::c_void);
}

// ---------------------------------------------------------------------------
// OscParser
// ---------------------------------------------------------------------------

pub struct OscParser {
    ptr: NonNull<ffi::GhosttyOscParser>,
    _not_send_sync: PhantomData<*mut ()>,
}

pub struct OscCommand<'p> {
    ptr: ffi::GhosttyOscCommand_ptr,
    _parser: PhantomData<&'p OscParser>,
}

impl OscCommand<'_> {
    pub fn command_type(&self) -> ffi::GhosttyOscCommandType {
        unsafe { ffi::ghostty_osc_command_type(self.ptr) }
    }

    pub fn as_raw(&self) -> ffi::GhosttyOscCommand_ptr {
        self.ptr
    }
}

impl OscParser {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyOscParser_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_osc_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_osc_reset(self.ptr.as_ptr()) }
    }

    pub fn next_byte(&mut self, byte: u8) {
        unsafe { ffi::ghostty_osc_next(self.ptr.as_ptr(), byte) }
    }

    pub fn end(&mut self, terminator: u8) -> OscCommand<'_> {
        let raw = unsafe { ffi::ghostty_osc_end(self.ptr.as_ptr(), terminator) };
        OscCommand {
            ptr: raw,
            _parser: PhantomData,
        }
    }
}

impl Drop for OscParser {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_osc_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// SgrParser
// ---------------------------------------------------------------------------

pub struct SgrParser {
    ptr: NonNull<ffi::GhosttySgrParser>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl SgrParser {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttySgrParser_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_sgr_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn set_params(&mut self, params: &[u16], separators: Option<&[u8]>) -> Result<(), Error> {
        let sep_ptr = match separators {
            Some(seps) => {
                assert!(
                    seps.len() == params.len(),
                    "separators length must equal params length"
                );
                seps.as_ptr().cast::<std::os::raw::c_char>()
            }
            None => std::ptr::null(),
        };
        let result = unsafe {
            ffi::ghostty_sgr_set_params(self.ptr.as_ptr(), params.as_ptr(), sep_ptr, params.len())
        };
        from_result(result)
    }

    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_sgr_reset(self.ptr.as_ptr()) }
    }

    pub fn next_attr(&mut self) -> Option<ffi::GhosttySgrAttribute> {
        let mut attr = ffi::GhosttySgrAttribute::default();
        let has_next = unsafe { ffi::ghostty_sgr_next(self.ptr.as_ptr(), &mut attr) };
        if has_next { Some(attr) } else { None }
    }
}

impl Drop for SgrParser {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_sgr_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// Paste utility
// ---------------------------------------------------------------------------

pub fn paste_is_safe(data: &str) -> bool {
    unsafe { ffi::ghostty_paste_is_safe(data.as_ptr().cast(), data.len()) }
}

// ---------------------------------------------------------------------------
// Build info
// ---------------------------------------------------------------------------

pub fn build_info_simd() -> Result<bool, Error> {
    let mut value = false;
    let result = unsafe {
        ffi::ghostty_build_info(
            ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_SIMD,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}

pub fn build_info_kitty_graphics() -> Result<bool, Error> {
    let mut value = false;
    let result = unsafe {
        ffi::ghostty_build_info(
            ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_KITTY_GRAPHICS,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}

pub fn build_info_tmux_control_mode() -> Result<bool, Error> {
    let mut value = false;
    let result = unsafe {
        ffi::ghostty_build_info(
            ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_TMUX_CONTROL_MODE,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}

pub fn build_info_optimize() -> Result<ffi::GhosttyOptimizeMode, Error> {
    let mut value: ffi::GhosttyOptimizeMode = ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_DEBUG;
    let result = unsafe {
        ffi::ghostty_build_info(
            ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_OPTIMIZE,
            std::ptr::from_mut(&mut value).cast(),
        )
    };
    from_result(result)?;
    Ok(value)
}

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
// RenderState
// ---------------------------------------------------------------------------

pub struct RenderState {
    ptr: NonNull<ffi::GhosttyRenderState>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl RenderState {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyRenderState_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn as_raw(&self) -> ffi::GhosttyRenderState_ptr {
        self.ptr.as_ptr()
    }

    pub fn update(&mut self, terminal: &mut Terminal) -> Result<(), Error> {
        let result =
            unsafe { ffi::ghostty_render_state_update(self.ptr.as_ptr(), terminal.as_raw()) };
        from_result(result)
    }

    pub fn dirty(&self) -> Result<ffi::GhosttyRenderStateDirty, Error> {
        let mut value: ffi::GhosttyRenderStateDirty =
            ffi::GhosttyRenderStateDirty_GHOSTTY_RENDER_STATE_DIRTY_FALSE;
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_DIRTY,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn cols(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_COLS,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn rows(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROWS,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn populate_row_iterator(&self, iter: &mut RenderStateRowIterator) -> Result<(), Error> {
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                std::ptr::from_mut(&mut iter.ptr).cast::<std::ffi::c_void>(),
            )
        };
        from_result(result)
    }

    pub fn cursor_visible(&self) -> Result<bool, Error> {
        let mut value = false;
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn cursor_viewport_has_value(&self) -> Result<bool, Error> {
        let mut value = false;
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn cursor_viewport_x(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn cursor_viewport_y(&self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        let result = unsafe {
            ffi::ghostty_render_state_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn colors_get(&self) -> Result<ffi::GhosttyRenderStateColors, Error> {
        let mut colors = ffi::GhosttyRenderStateColors::default();
        colors.size = std::mem::size_of::<ffi::GhosttyRenderStateColors>();
        let result =
            unsafe { ffi::ghostty_render_state_colors_get(self.ptr.as_ptr(), &mut colors) };
        from_result(result)?;
        Ok(colors)
    }

    pub fn set_dirty(&mut self, dirty: ffi::GhosttyRenderStateDirty) -> Result<(), Error> {
        let result = unsafe {
            ffi::ghostty_render_state_set(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateOption_GHOSTTY_RENDER_STATE_OPTION_DIRTY,
                std::ptr::from_ref(&dirty).cast(),
            )
        };
        from_result(result)
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// RenderStateRowIterator
// ---------------------------------------------------------------------------

pub struct RenderStateRowIterator {
    ptr: NonNull<ffi::GhosttyRenderStateRowIterator>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl RenderStateRowIterator {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyRenderStateRowIterator_ptr = std::ptr::null_mut();
        let result =
            unsafe { ffi::ghostty_render_state_row_iterator_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn next(&mut self) -> bool {
        unsafe { ffi::ghostty_render_state_row_iterator_next(self.ptr.as_ptr()) }
    }

    pub fn dirty(&self) -> Result<bool, Error> {
        let mut value = false;
        let result = unsafe {
            ffi::ghostty_render_state_row_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn raw_row(&self) -> Result<ffi::GhosttyRow, Error> {
        let mut value: ffi::GhosttyRow = 0;
        let result = unsafe {
            ffi::ghostty_render_state_row_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_RAW,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn populate_cells(&self, cells: &mut RenderStateRowCells) -> Result<(), Error> {
        let result = unsafe {
            ffi::ghostty_render_state_row_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                std::ptr::from_mut(&mut cells.ptr).cast::<std::ffi::c_void>(),
            )
        };
        from_result(result)
    }

    pub fn set_dirty(&mut self, dirty: bool) -> Result<(), Error> {
        let result = unsafe {
            ffi::ghostty_render_state_row_set(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowOption_GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY,
                std::ptr::from_ref(&dirty).cast(),
            )
        };
        from_result(result)
    }
}

impl Drop for RenderStateRowIterator {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_iterator_free(self.ptr.as_ptr()) }
    }
}

// ---------------------------------------------------------------------------
// RenderStateRowCells
// ---------------------------------------------------------------------------

pub struct RenderStateRowCells {
    ptr: NonNull<ffi::GhosttyRenderStateRowCells>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl RenderStateRowCells {
    pub fn new() -> Result<Self, Error> {
        let mut raw: ffi::GhosttyRenderStateRowCells_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_row_cells_new(std::ptr::null(), &mut raw) };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    pub fn next(&mut self) -> bool {
        unsafe { ffi::ghostty_render_state_row_cells_next(self.ptr.as_ptr()) }
    }

    pub fn select(&mut self, x: u16) -> Result<(), Error> {
        let result = unsafe { ffi::ghostty_render_state_row_cells_select(self.ptr.as_ptr(), x) };
        from_result(result)
    }

    pub fn raw_cell(&self) -> Result<ffi::GhosttyCell, Error> {
        let mut value: ffi::GhosttyCell = 0;
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn style(&self) -> Result<ffi::GhosttyStyle, Error> {
        let mut value = ffi::GhosttyStyle::default();
        value.size = std::mem::size_of::<ffi::GhosttyStyle>();
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn graphemes_len(&self) -> Result<u32, Error> {
        let mut value: u32 = 0;
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Ok(value)
    }

    pub fn graphemes_buf(&self, buf: &mut [char]) -> Result<(), Error> {
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.ptr.as_ptr(),
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                buf.as_mut_ptr().cast(),
            )
        };
        from_result(result)
    }
}

impl Drop for RenderStateRowCells {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_cells_free(self.ptr.as_ptr()) }
    }
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
