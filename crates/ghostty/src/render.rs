//! Managing [render states](RenderState) of the terminal.

use std::{marker::PhantomData, mem::MaybeUninit};

use crate::{
    alloc::{Allocator, Object},
    error::{Error, Result, from_result},
    ffi,
    style::{RgbColor, Style},
    terminal::Terminal,
};

/// Represents the state required to render a visible screen (a viewport) of
/// a terminal instance.
///
/// This is stateful and optimized for repeated updates from a single terminal
/// instance and only updating dirty regions of the screen.
///
/// The key design principle of this API is that it only needs read/write
/// access to the terminal instance during the update call. This allows the
/// render state to minimally impact terminal IO performance and also allows
/// the renderer to be safely multi-threaded (as long as a lock is held
/// during the update call to ensure exclusive access to the terminal instance).
///
/// The basic usage of this API is:
///
///  1. Create an empty render state
///  2. Update it from a terminal instance whenever you need.
///  3. Read from the render state to get the data needed to draw your frame.
///
/// # Dirty Tracking
///
/// Dirty tracking is a key feature of the render state that allows renderers
/// to efficiently determine what parts of the screen have changed and only
/// redraw changed regions.
///
/// The render state API keeps track of dirty state at two independent layers:
/// a global dirty state that indicates whether the entire frame is clean,
/// partially dirty, or fully dirty, and a per-row dirty state that allows
/// tracking which rows in a partially dirty frame have changed.
///
/// The user of the render state API is expected to unset both of these.
/// The update call does not unset dirty state, it only updates it.
///
/// An extremely important detail: **setting one dirty state doesn't unset
/// the other.** For example, setting the global dirty state to false does
/// not reset the row-level dirty flags. So, the caller of the render state
/// API must be careful to manage both layers of dirty state correctly.
///
/// # Examples
///
/// ## Creating and updating render state
///
/// ```rust
/// // Create a terminal and render state, then update the render state
/// // from the terminal. The render state captures a snapshot of everything
/// // needed to draw a frame.
/// use ghostty::{Terminal, TerminalOptions, RenderState};
///
/// let mut terminal = Terminal::new(TerminalOptions {
///     cols: 40,
///     rows: 5,
///     max_scrollback: 10000,
/// }).unwrap();
///
/// let mut render_state = RenderState::new().unwrap();
///
/// // Feed some styled content into the terminal.
/// terminal.vt_write(b"Hello, \x1b[1;32mworld\x1b[0m!\r\n");
/// terminal.vt_write(b"\x1b[4munderlined\x1b[0m text\r\n");
/// terminal.vt_write(b"\x1b[38;2;255;128;0morange\x1b[0m\r\n");
///
/// assert!(render_state.update(&terminal).is_ok());
/// ```
///
/// ## Checking dirty state
///
/// ```rust
/// // Check the global dirty state to decide how much work the renderer
/// // needs to do. After rendering, reset it to false.
/// # use ghostty::{Terminal, TerminalOptions, RenderState, render::Dirty};
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new().unwrap();
/// let snapshot = render_state.update(&terminal).unwrap();
///
/// match snapshot.dirty().unwrap() {
///     Dirty::Clean => println!("Frame is clean, nothing to draw."),
///     Dirty::Partial => println!("Partial redraw needed."),
///     Dirty::Full => println!("Full redraw needed."),
/// }
/// ```
///
/// ## Reading colors
///
/// ```rust
/// // Retrieve colors (background, foreground, palette) from the render
/// // state. These are needed to resolve palette-indexed cell colors.
/// # use ghostty::{Terminal, TerminalOptions, RenderState};
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new().unwrap();
/// let snapshot = render_state.update(&terminal).unwrap();
/// let colors = snapshot.colors().unwrap();
///
/// println!(
///     "Background: {:02x}{:02x}{:02x}",
///     colors.background.r, colors.background.g, colors.background.b
/// );
/// println!(
///     "Foreground: {:02x}{:02x}{:02x}",
///     colors.background.r, colors.background.g, colors.background.b
/// );
/// ```
///
/// ## Reading cursor state
///
/// ```rust
/// // Read cursor position and visual style from the render state.
/// use ghostty::render::CursorViewport;
/// # use ghostty::{Terminal, TerminalOptions, RenderState};
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new().unwrap();
/// let snapshot = render_state.update(&terminal).unwrap();
///
/// if snapshot.cursor_visible().unwrap() {
///     if let Some(CursorViewport { x, y, .. }) = snapshot.cursor_viewport().unwrap() {
///         let style = snapshot.cursor_visual_style().unwrap();
///         println!("Cursor at ({x}, {y}), style {style:?}");
///     }
/// }
/// ```
///
/// ## Iterating rows and cells
///
/// ```rust
/// // Iterate rows via the row iterator. For each dirty row, iterate its
/// // cells, read codepoints/graphemes and styles, and emit ANSI-colored
/// // output as a simple "renderer".
/// # use ghostty::{Terminal, TerminalOptions, RenderState};
/// # let terminal = Terminal::new(TerminalOptions {
/// #     cols: 80,
/// #     rows: 25,
/// #     max_scrollback: 10000,
/// # }).unwrap();
/// # let mut render_state = RenderState::new().unwrap();
/// use ghostty::render::{RowIterator, CellIterator};
///
/// // During setup:
/// let mut rows = RowIterator::new().unwrap();
/// let mut cells = CellIterator::new().unwrap();
///
/// // On each frame:
/// let snapshot = render_state.update(&terminal).unwrap();
/// let mut row_iter = rows.update(&snapshot);
/// let mut row_index = 0;
///
/// while let Some(row) = row_iter.next() {
///     // Check per-row dirty state; a real renderer would skip clean rows.
///     print!(
///         "Row {row_index} [{}]",
///         if row.dirty().unwrap() { "dirty" } else { "clean" }
///     );
///
///     // Get cells for this row (reuses the same cells handle).
///     let mut cell_iter = cells.update(&row);
///     while let Some(cell) = cell_iter.next() {
///         let graphemes = cell.graphemes().unwrap();
///         println!("{:?}", &graphemes);
///     }
///     row_index += 1;
///     println!()
/// }
/// ```
pub struct RenderState<'alloc>(Object<'alloc, ffi::GhosttyRenderState>);

/// A snapshot of the render state after an update.
///
/// This struct exists to guard data accessed from the render state from
/// being accidentally modified after an update. If you find yourself unable
/// to update the render state due to borrow checker errors, make sure to
/// drop the active snapshot (and data that depends on it) before updating.
pub struct Snapshot<'alloc, 's>(&'s mut RenderState<'alloc>);

/// Opaque handle to a render-state row iterator.
///
/// The row iterator must be [updated](RowIterator::update) from a snapshot of
/// the render state in order to function, as most data is only accessible
/// per [iteration](RowIteration).
pub struct RowIterator<'alloc>(Object<'alloc, ffi::GhosttyRenderStateRowIterator>);

/// An active iteration over the rows in the render state.
///
/// Row iterations are created by [updating](RowIterator::update) row iterators
/// with a snapshot of the render state. The borrow checker statically
/// guarantees that all accesses of the data do not outlive the given snapshot,
/// at the cost of added lifetime annotations.
pub struct RowIteration<'alloc, 's> {
    iter: &'s mut RowIterator<'alloc>,
    // NOTE: While in theory the snapshot borrow should have its own
    // lifetime 'ss where 'rs: 'ss, but it gets very unwieldy and honestly
    // one wouldn't run into too many situations where this simpler constraint
    // isn't enough.
    _phan: PhantomData<&'s Snapshot<'alloc, 's>>,
}

/// Opaque handle to a render state cell iterator.
///
/// The cell iterator must be [updated](CellIterator::update) from a
/// [row](RowIteration) in order to function, as most data is only
/// accessible per [iteration](CellIteration).
pub struct CellIterator<'alloc>(Object<'alloc, ffi::GhosttyRenderStateRowCells>);

/// An active iteration over the cells on a given row
/// within the render state.
///
/// Cell iterations are created by [updating](CellIterator::update) row iterators
/// at a given [row](RowEntry). The borrow checker statically
/// guarantees that all accesses of the data do not outlive the given snapshot,
/// at the cost of added lifetime annotations.
pub struct CellIteration<'alloc, 's> {
    iter: &'s mut CellIterator<'alloc>,
    _phan: PhantomData<&'s RowIteration<'alloc, 's>>,
}

//--------------------------
// Impl blocks
//--------------------------

impl<'alloc> RenderState<'alloc> {
    /// Create a new render state instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new render state instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(alloc: &'alloc Allocator<'ctx, Ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self> {
        let mut raw: ffi::GhosttyRenderState_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_new(alloc, &mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    pub fn update<'s, UserData>(
        &'s mut self,
        terminal: &Terminal<'_, '_, UserData>,
    ) -> Result<Snapshot<'alloc, 's>> {
        let result =
            unsafe { ffi::ghostty_render_state_update(self.0.as_raw(), terminal.inner.as_raw()) };
        from_result(result)?;
        Ok(Snapshot(self))
    }
}

impl Drop for RenderState<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_free(self.0.as_raw()) }
    }
}

impl<'alloc, 's> Snapshot<'alloc, 's> {
    fn get<T>(&self, tag: ffi::GhosttyRenderStateData) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_render_state_get(self.0.0.as_raw(), tag, value.as_mut_ptr().cast())
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    fn set<T>(&self, tag: ffi::GhosttyRenderStateOption, value: T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_render_state_set(self.0.0.as_raw(), tag, std::ptr::from_ref(&value).cast())
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)
    }

    /// Get the current dirty state.
    pub fn dirty(&self) -> Result<Dirty> {
        self.get::<ffi::GhosttyRenderStateDirty>(
            ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_DIRTY,
        )
        .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }

    /// Get the viewport width.
    pub fn cols(&self) -> Result<u16> {
        self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_COLS)
    }

    /// Get the viewport height.
    pub fn rows(&self) -> Result<u16> {
        self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROWS)
    }

    /// Get the cursor color that may have been explicitly set by the terminal state.
    pub fn cursor_color(&self) -> Result<Option<RgbColor>> {
        let has_value =
            self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_COLOR_CURSOR_HAS_VALUE)?;
        if has_value {
            let color =
                self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_COLOR_CURSOR)?;
            Ok(Some(color))
        } else {
            Ok(None)
        }
    }

    /// Whether the cursor is currently visible based on terminal modes.
    pub fn cursor_visible(&self) -> Result<bool> {
        self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE)
    }

    /// Whether the cursor is currently blinking based on terminal modes.
    pub fn cursor_blinking(&self) -> Result<bool> {
        self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_BLINKING)
    }

    /// Whether the cursor is at a password input field.
    pub fn cursor_password_input(&self) -> Result<bool> {
        self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_PASSWORD_INPUT)
    }

    /// Get the visual style of the cursor.
    pub fn cursor_visual_style(&self) -> Result<CursorVisualStyle> {
        self.get::<ffi::GhosttyRenderStateCursorVisualStyle>(
            ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE,
        )
        .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }

    /// Get the relative position of the cursor and other information
    /// if it is currently visible within the viewport.
    pub fn cursor_viewport(&self) -> Result<Option<CursorViewport>> {
        let has_value = self
            .get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE)?;
        if has_value {
            let x =
                self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X)?;
            let y =
                self.get(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y)?;
            let at_wide_tail = self.get(
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_WIDE_TAIL,
            )?;
            Ok(Some(CursorViewport { x, y, at_wide_tail }))
        } else {
            Ok(None)
        }
    }

    pub fn colors(&self) -> Result<Colors> {
        let mut colors = ffi::GhosttyRenderStateColors {
            size: std::mem::size_of::<ffi::GhosttyRenderStateColors>(),
            ..Default::default()
        };
        let result =
            unsafe { ffi::ghostty_render_state_colors_get(self.0.0.as_raw(), &mut colors) };
        from_result(result)?;

        Ok(Colors {
            background: colors.background.into(),
            foreground: colors.foreground.into(),
            cursor: if colors.cursor_has_value {
                Some(colors.cursor.into())
            } else {
                None
            },
            palette: colors.palette.map(|c| c.into()),
        })
    }

    pub fn set_dirty(&self, dirty: Dirty) -> Result<()> {
        self.set(
            ffi::GhosttyRenderStateOption_GHOSTTY_RENDER_STATE_OPTION_DIRTY,
            ffi::GhosttyRenderStateDirty::from(dirty),
        )
    }
}

impl<'alloc> RowIterator<'alloc> {
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(alloc: &'alloc Allocator<'ctx, Ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self> {
        let mut raw: ffi::GhosttyRenderStateRowIterator_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_row_iterator_new(alloc, &mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    pub fn update<'s>(
        &'s mut self,
        snapshot: &'s Snapshot<'alloc, 's>,
    ) -> RowIteration<'alloc, 's> {
        let result = unsafe {
            ffi::ghostty_render_state_get(
                snapshot.0.0.as_raw(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                std::ptr::from_mut(&mut self.0.ptr).cast(),
            )
        };
        assert!(from_result(result).is_ok());

        RowIteration {
            iter: self,
            _phan: PhantomData,
        }
    }
}

impl Drop for RowIterator<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_iterator_free(self.0.as_raw()) }
    }
}

impl<'alloc, 's> RowIteration<'alloc, 's> {
    // Can't actually implement Iterator - this is lending.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&Self> {
        if unsafe { ffi::ghostty_render_state_row_iterator_next(self.iter.0.as_raw()) } {
            Some(self)
        } else {
            None
        }
    }

    fn get<T>(&self, tag: ffi::GhosttyRenderStateRowData) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_render_state_row_get(self.iter.0.as_raw(), tag, value.as_mut_ptr().cast())
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    fn set<T>(&self, tag: ffi::GhosttyRenderStateRowOption, value: T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_render_state_row_set(
                self.iter.0.as_raw(),
                tag,
                std::ptr::from_ref(&value).cast(),
            )
        };
        from_result(result)
    }
    pub fn dirty(&self) -> Result<bool> {
        self.get(ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY)
    }

    pub fn raw_row(&self) -> Result<ffi::GhosttyRow> {
        self.get(ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_RAW)
    }

    pub fn set_dirty(&self, dirty: bool) -> Result<()> {
        self.set(
            ffi::GhosttyRenderStateRowOption_GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY,
            dirty,
        )
    }
}

impl<'alloc> CellIterator<'alloc> {
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(alloc: &'alloc Allocator<'ctx, Ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self> {
        let mut raw: ffi::GhosttyRenderStateRowCells_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_render_state_row_cells_new(alloc, &mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    pub fn update<'s>(
        &'s mut self,
        row: &'s RowIteration<'alloc, 's>,
    ) -> CellIteration<'alloc, 's> {
        let result = unsafe {
            ffi::ghostty_render_state_row_get(
                row.iter.0.as_raw(),
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                std::ptr::from_mut(&mut self.0.ptr).cast(),
            )
        };
        assert!(from_result(result).is_ok());

        CellIteration {
            iter: self,
            _phan: PhantomData,
        }
    }
}

impl Drop for CellIterator<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_cells_free(self.0.as_raw()) }
    }
}

impl<'alloc, 's> CellIteration<'alloc, 's> {
    // Can't actually implement Iterator - this is lending.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&Self> {
        if unsafe { ffi::ghostty_render_state_row_cells_next(self.iter.0.as_raw()) } {
            Some(self)
        } else {
            None
        }
    }

    pub fn select(&mut self, x: u16) -> Result<()> {
        let result = unsafe { ffi::ghostty_render_state_row_cells_select(self.iter.0.as_raw(), x) };
        from_result(result)
    }

    fn get<T>(&self, tag: ffi::GhosttyRenderStateRowCellsData) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.iter.0.as_raw(),
                tag,
                value.as_mut_ptr().cast(),
            )
        };
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }
    pub fn raw_cell(&self) -> Result<ffi::GhosttyCell> {
        self.get(ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW)
    }

    pub fn style(&self) -> Result<Style> {
        let mut value = ffi::sized!(ffi::GhosttyStyle);
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.iter.0.as_raw(),
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                std::ptr::from_mut(&mut value).cast(),
            )
        };
        from_result(result)?;
        Style::try_from(value)
    }

    pub fn graphemes(&self) -> Result<Vec<char>> {
        let len = self.get(
            ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
        )?;
        let mut graphemes = Vec::<char>::with_capacity(len);

        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.iter.0.as_raw(),
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                graphemes.as_mut_ptr().cast(),
            )
        };
        from_result(result)?;
        Ok(graphemes)
    }
}

//---------------------------
// Auxiliary types
//---------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorViewport {
    pub x: u16,
    pub y: u16,
    pub at_wide_tail: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    /// The default/current background color for the render state.
    pub background: RgbColor,
    /// The default/current foreground color for the render state.
    pub foreground: RgbColor,
    /// The cursor color which may be explicitly set by terminal state.
    pub cursor: Option<RgbColor>,
    /// The active 256-color palette for this render state.
    pub palette: [RgbColor; 256],
}

/// Dirty state of a render state after update.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum Dirty {
    /// Not dirty at all; rendering can be skipped.
    Clean = ffi::GhosttyRenderStateDirty_GHOSTTY_RENDER_STATE_DIRTY_FALSE,
    /// Some rows changed; renderer can redraw incrementally.
    Partial = ffi::GhosttyRenderStateDirty_GHOSTTY_RENDER_STATE_DIRTY_PARTIAL,
    /// Global state changed; renderer should redraw everything.
    Full = ffi::GhosttyRenderStateDirty_GHOSTTY_RENDER_STATE_DIRTY_FULL,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum CursorVisualStyle {
    Bar = ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR,
    Block = ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK,
    Underline =
        ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE,
    BlockHollow = ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW,
}
