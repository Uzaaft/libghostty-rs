//! Point at a cell, read it, and keep track of it as the terminal scrolls.
//!
//! A grid reference points at one cell in the terminal.
//!
//! An untracked reference is a snapshot that is only valid until the next
//! change to the terminal. A tracked reference follows its cell. Use one to
//! read particular cells. They aren't fast enough to draw a frame, so use
//! [Render State](crate::render) for that.
// FIXME: Transclude examples, link to guide
use std::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull};

use crate::{
    error::{Error, Result, from_optional_result_uninit, from_result, from_result_with_len},
    ffi,
    point::{self, Point},
    screen::{Cell, Row},
    style::Style,
    terminal::Terminal,
};

/// Methods related to [grid references](crate::grid_ref).
impl Terminal<'_, '_> {
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
        let mut grid_ref = ffi::GridRef::default();
        let result = unsafe {
            ffi::ghostty_terminal_grid_ref(self.inner.as_raw(), point.into(), &raw mut grid_ref)
        };
        from_result(result)?;
        Ok(unsafe { GridRef::from_raw(grid_ref) })
    }

    /// Create an owned tracked grid reference for a terminal point.
    ///
    /// This is the tracked variant of [`Terminal::grid_ref`]. The returned handle
    /// follows the referenced cell as the terminal's page list is modified:
    /// scrolling, pruning, resize/reflow, and other page-list operations update
    /// the tracked reference automatically.
    ///
    /// The reference is attached to the terminal screen/page-list that is
    /// active at creation time.
    ///
    /// If the point is outside the requested coordinate space, this returns
    /// `Err(Error::InvalidValue)`.
    ///
    /// If the tracked grid reference outlives this terminal, the handle remains
    /// valid, but it will always return `false` or `Ok(None)`.
    pub fn track_grid_ref(&self, point: Point) -> Result<TrackedGridRef> {
        let mut raw: ffi::TrackedGridRef = std::ptr::null_mut();
        let result = unsafe {
            ffi::ghostty_terminal_grid_ref_track(self.inner.as_raw(), point.into(), &raw mut raw)
        };
        from_result(result)?;

        let inner = NonNull::new(raw).ok_or(Error::InvalidValue)?;
        Ok(TrackedGridRef::new(inner, self.inner.ptr))
    }

    /// Convert a grid reference back to a point in the given coordinate system.
    ///
    /// This is the inverse of [`Terminal::grid_ref`]: given a grid reference, it
    /// returns the x/y coordinates in the requested coordinate system (active,
    /// viewport, screen, or history).
    ///
    /// The grid reference must have been obtained from the same terminal instance.
    /// Like all grid references, it is only valid until the next mutating
    /// terminal call.
    ///
    /// Not every grid reference is representable in every coordinate system.
    /// For example, a cell in scrollback history cannot be expressed in active
    /// coordinates, and a cell that has scrolled off the visible area cannot
    /// be expressed in viewport coordinates. In these cases, the function
    /// returns `Ok(None)`.
    pub fn point_from_grid_ref(
        &self,
        grid_ref: GridRef<'_>,
        space: point::Space,
    ) -> Result<Option<point::Coordinate>> {
        let mut point = MaybeUninit::<ffi::PointCoordinate>::zeroed();
        let result = unsafe {
            ffi::ghostty_terminal_point_from_grid_ref(
                self.inner.as_raw(),
                std::ptr::from_ref(&grid_ref.inner),
                space.into_raw(),
                point.as_mut_ptr(),
            )
        };

        from_optional_result_uninit(result, point).map(|value| value.map(Into::into))
    }
}

/// Resolved reference to a terminal cell position.
///
/// A grid reference is a resolved reference to a specific cell position in
/// the terminal's internal page structure. Obtain a grid reference from
/// [`Terminal::grid_ref`][crate::Terminal::grid_ref], then extract the cell
/// or row via [`GridRef::cell`] and [`GridRef::row`].
///
/// A grid reference is only valid until the next update to the terminal
/// instance. There is no guarantee that a grid reference will remain valid
/// after ANY operation, even if a seemingly unrelated part of the grid is
/// changed, so any information related to the grid reference should be read
/// and cached immediately after obtaining the grid reference.
///
/// This API is not meant to be used as the core of render loop.
/// It isn't built to sustain the framerates needed for rendering large screens.
/// Use the render state API for that.
#[derive(Clone, Copy, Debug)]
pub struct GridRef<'t> {
    pub(crate) inner: ffi::GridRef,
    pub(crate) _phan: PhantomData<&'t ffi::Terminal>,
}

impl GridRef<'_> {
    pub(crate) unsafe fn from_raw(inner: ffi::GridRef) -> Self {
        Self {
            inner,
            _phan: PhantomData,
        }
    }

    /// Get the row from a grid reference.
    pub fn row(&self) -> Result<Row> {
        let mut v = ffi::Row::default();
        let result =
            unsafe { ffi::ghostty_grid_ref_row(std::ptr::from_ref(&self.inner), &raw mut v) };
        from_result(result)?;
        Ok(Row(v))
    }
    /// Get the cell from a grid reference.
    pub fn cell(&self) -> Result<Cell> {
        let mut v = ffi::Cell::default();
        let result =
            unsafe { ffi::ghostty_grid_ref_cell(std::ptr::from_ref(&self.inner), &raw mut v) };
        from_result(result)?;
        Ok(Cell(v))
    }
    /// Get the style of the cell at the grid reference's position.
    pub fn style(&self) -> Result<Style> {
        let mut v = ffi::Style::default();
        let result =
            unsafe { ffi::ghostty_grid_ref_style(std::ptr::from_ref(&self.inner), &raw mut v) };
        from_result(result)?;
        Style::try_from(v)
    }

    /// Get the grapheme cluster codepoints for the cell at the grid
    /// reference's position.
    ///
    /// Writes the full grapheme cluster (the cell's primary codepoint
    /// followed by any combining codepoints) into the provided buffer.
    /// If the cell has no text, `Ok(0)` is returned.
    ///
    /// If the buffer is too small, the function returns
    /// `Err(Error::OutOfSpace { required })` where `required` is the
    /// required number of codepoints. The caller can then retry with
    /// a sufficiently sized buffer.
    pub fn graphemes(&self, buf: &mut [char]) -> Result<usize> {
        let mut len = 0;
        let result = unsafe {
            ffi::ghostty_grid_ref_graphemes(
                std::ptr::from_ref(&self.inner),
                std::ptr::from_mut(buf).cast(),
                buf.len(),
                &raw mut len,
            )
        };
        from_result_with_len(result, len)
    }

    /// Get the hyperlink URI for the cell at the grid reference's position.
    ///
    /// Writes the URI bytes into the provided buffer.
    /// If the cell has no hyperlink, `Ok(0)` is returned.
    ///
    /// If the buffer is too small, the function returns
    /// `Err(Error::OutOfSpace { required })` where `required` is the
    /// required number of codepoints. The caller can then retry with
    /// a sufficiently sized buffer.
    pub fn hyperlink_uri(&self, buf: &mut [u8]) -> Result<usize> {
        let mut len = 0;
        let result = unsafe {
            ffi::ghostty_grid_ref_hyperlink_uri(
                std::ptr::from_ref(&self.inner),
                std::ptr::from_mut(buf).cast(),
                buf.len(),
                &raw mut len,
            )
        };
        from_result_with_len(result, len)
    }
}

/// Owned grid references that move with the terminal.
///
/// A tracked grid reference follows its cell across normal screen operations.
/// For example scrolling, scrollback pruning, resize/reflow, and other
/// terminal mutations update the tracked reference automatically.
///
/// A tracked reference can still lose its original semantic location.
/// This can happen when the underlying grid is reset, pruned, or otherwise
/// discarded in a way that cannot be mapped to a meaningful new cell.
/// In that state, [`TrackedGridRef::has_value`] returns `false` and
/// [`TrackedGridRef::snapshot`] / [`TrackedGridRef::point`] return `Ok(None)`.
/// The handle remains valid, and callers may move it to a new point with
/// [`TrackedGridRef::set`].
///
/// To read cell data from a tracked reference, first snapshot it with
/// [`TrackedGridRef::snapshot`]. The returned [`GridRef`] is again an
/// untracked reference and follows the same short lifetime rules as any
/// other untracked grid reference.
///
/// A tracked reference belongs to the terminal screen/page-list that was
/// active when it was created or last set. Converting it to a point uses that
/// owning screen/page-list, even if the terminal has since switched between
/// primary and alternate screens. Calling [`TrackedGridRef::set`] resolves
/// the new point against the terminal's currently active screen/page-list
/// and may move the tracked reference between screens.
///
/// If the tracked grid reference outlives the terminal it is created from,
/// it remains valid, but all APIs return either `false` or `Ok(None)`.
///
/// Each tracked reference adds bookkeeping to terminal mutations. Use them
/// sparingly for long-lived anchors such as selections, search state, marks,
/// or application-side bookmarks.
#[derive(Debug)]
pub struct TrackedGridRef {
    inner: NonNull<ffi::TrackedGridRefImpl>,
    terminal: NonNull<ffi::TerminalImpl>,
}

impl TrackedGridRef {
    pub(crate) fn new(
        inner: NonNull<ffi::TrackedGridRefImpl>,
        terminal: NonNull<ffi::TerminalImpl>,
    ) -> Self {
        Self { inner, terminal }
    }

    /// Whether a tracked grid reference currently has a meaningful value.
    ///
    /// If the terminal that created the tracked reference has been dropped,
    /// this returns false.
    #[must_use]
    pub fn has_value(&self) -> bool {
        unsafe { ffi::ghostty_tracked_grid_ref_has_value(self.inner.as_ptr()) }
    }

    /// Snapshot a tracked grid reference into a regular [`GridRef`].
    ///
    /// The returned [`GridRef`] is an untracked snapshot and has the same lifetime
    /// rules as [`Terminal::grid_ref`]: it is only valid until the next terminal update.
    /// Snapshot immediately before calling [`GridRef::cell`], [`GridRef::row`],
    /// [`GridRef::graphemes`], [`GridRef::hyperlink_uri`], or [`GridRef::style`],
    ///
    /// If the tracked reference no longer has a meaningful value, this returns
    /// `Ok(None)`. This includes references whose owning terminal has been dropped.
    pub fn snapshot<'t>(&self, terminal: &'t Terminal<'_, '_>) -> Result<Option<GridRef<'t>>> {
        // The C ghostty_tracked_grid_ref_snapshot does not take a terminal, so
        // we validate the pairing here to keep the returned GridRef's lifetime
        // soundly tied to a terminal that actually owns the underlying pin.
        if self.terminal != terminal.inner.ptr {
            return Err(Error::InvalidValue);
        }
        let mut grid_ref = MaybeUninit::zeroed();
        let result = unsafe {
            ffi::ghostty_tracked_grid_ref_snapshot(self.inner.as_ptr(), grid_ref.as_mut_ptr())
        };

        from_optional_result_uninit(result, grid_ref).map(|value| {
            value.map(|raw| unsafe {
                // SAFETY: A successful libghostty snapshot initializes a
                // short-lived untracked grid reference for the provided
                // terminal. The returned Rust lifetime is tied to that
                // terminal borrow.
                GridRef::from_raw(raw)
            })
        })
    }

    /// Convert a tracked grid reference to a point in the requested coordinate space.
    ///
    /// This is the tracked equivalent of [`Terminal::point_from_grid_ref`].
    /// Unlike snapshotting, this does not expose an intermediate untracked
    /// [`GridRef`].
    ///
    /// A tracked reference is resolved against the terminal screen/page-list
    /// that currently owns the reference. If the terminal has switched between
    /// primary and alternate screens since the reference was created or last
    /// set, this may be different from the terminal's currently active screen.
    ///
    /// If the tracked reference no longer has a meaningful value, this returns
    /// `Ok(None)`. `Ok(None` is also returned when the reference cannot be represented
    /// in the requested coordinate space, including after the terminal that
    /// created the tracked reference has been dropped.
    pub fn point(&self, space: point::Space) -> Result<Option<point::Coordinate>> {
        let mut point = MaybeUninit::<ffi::PointCoordinate>::zeroed();
        let result = unsafe {
            ffi::ghostty_tracked_grid_ref_point(
                self.inner.as_ptr(),
                space.into_raw(),
                point.as_mut_ptr(),
            )
        };

        from_optional_result_uninit(result, point).map(|value| value.map(Into::into))
    }

    /// Move an existing tracked grid reference to a new terminal point.
    ///
    /// On success, the tracked reference begins tracking the new point and any
    /// prior "no value" state is cleared. On `Err(Error::OutOfMemory)`, the original
    /// tracked reference is left unchanged.
    ///
    /// The terminal must be the same terminal that created the tracked reference.
    /// The point is resolved against the terminal screen/page-list that is active
    /// at the time this function is called. If the terminal has switched between
    /// primary and alternate screens, this may move the tracked reference from
    /// one screen/page-list to the other.
    pub fn set(&mut self, terminal: &mut Terminal<'_, '_>, point: Point) -> Result<&mut Self> {
        // The C layer validates the terminal/tracked-ref pairing and returns
        // GHOSTTY_INVALID_VALUE on mismatch, so we don't duplicate the check
        // on the Rust side.
        let result = unsafe {
            ffi::ghostty_tracked_grid_ref_set(
                self.inner.as_ptr(),
                terminal.inner.as_raw(),
                point.into(),
            )
        };
        from_result(result)?;
        Ok(self)
    }
}

impl Drop for TrackedGridRef {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_tracked_grid_ref_free(self.inner.as_ptr()) }
    }
}
