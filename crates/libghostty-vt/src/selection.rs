//! Select words, lines, and ranges, and turn pointer gestures into selections.
//!
//! A selection is a range of terminal content between two cells. It is a
//! snapshot, only valid until the next change to the terminal.
//!
//! You need a terminal, and usually a grid reference to start from.
// FIXME: Transclude examples, link to guide
use std::marker::PhantomData;

use crate::{
    error::{Error, Result, from_optional_result, from_result},
    ffi,
    grid_ref::GridRef,
    point::Point,
    terminal::Terminal,
};

pub mod gesture;

/// A snapshot selection range defined by two grid references.
///
/// # Preconditions
///
/// For every method that interacts with the terminal,
/// the selection's start and end grid refs must both be valid untracked
/// snapshots for the given terminal's currently active screen. In practice,
/// they must come from that terminal and screen, and no mutating terminal call
/// may have occurred since the refs were produced or reconstructed from
/// tracked refs. Passing refs from another terminal, another screen, or stale
/// refs violates this precondition.
#[derive(Clone, Copy, Debug)]
pub struct Selection<'t> {
    pub(crate) inner: ffi::Selection,
    pub(crate) terminal: ffi::Terminal,
    _phan: PhantomData<&'t ffi::Terminal>,
}
impl<'t> Selection<'t> {
    /// Create a new selection between two endpoints.
    ///
    /// Both endpoints are inclusive. The endpoints preserve selection direction
    /// and may be reversed; callers must not assume that start is the top-left
    /// endpoint or that end is the bottom-right endpoint.
    ///
    /// When `rectangle` is false, the endpoints describe a linear selection. When
    /// `rectangle` is true, the same endpoints are interpreted as opposite corners
    /// of a rectangular/block selection.
    #[must_use]
    pub fn new(
        terminal: &'t Terminal<'_, '_>,
        start: GridRef<'t>,
        end: GridRef<'t>,
        rectangle: bool,
    ) -> Self {
        // SAFETY: provided by the type system
        unsafe {
            Self::from_raw(
                terminal.inner.as_raw(),
                ffi::Selection {
                    start: start.inner,
                    end: end.inner,
                    rectangle,
                },
            )
        }
    }

    /// # Safety
    ///
    /// Caller must guarantee that the selection is bound by the lifetime `'t`,
    /// corresponding to the given terminal.
    pub(crate) unsafe fn from_raw(terminal: ffi::Terminal, value: ffi::Selection) -> Self {
        Self {
            terminal,
            inner: value,
            _phan: PhantomData,
        }
    }

    /// Start of the selection range (inclusive).
    ///
    /// This may be before start in terminal order. It is an untracked
    /// [`GridRef`] snapshot and follows untracked grid-ref lifetime rules.
    #[must_use]
    pub fn start(&self) -> GridRef<'t> {
        unsafe { GridRef::from_raw(self.inner.start) }
    }
    /// End of the selection range (inclusive).
    ///
    /// This may be before start in terminal order. It is an untracked
    /// [`GridRef`] snapshot and follows untracked grid-ref lifetime rules.
    #[must_use]
    pub fn end(&self) -> GridRef<'t> {
        unsafe { GridRef::from_raw(self.inner.end) }
    }
    /// Whether the endpoints are interpreted as a rectangular/block
    /// selection rather than a linear selection.
    #[must_use]
    pub fn rectangle(&self) -> bool {
        self.inner.rectangle
    }

    /// Adjust a selection snapshot using terminal selection semantics.
    ///
    /// The logical end endpoint is always moved, regardless of whether the
    /// selection is forward or reversed visually. The input selection remains
    /// a snapshot: after adjustment, call [`Terminal::set_selection`] to
    /// install it as the terminal-owned selection if desired.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn adjust(&mut self, adjustment: Adjustment) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_terminal_selection_adjust(
                self.terminal,
                &raw mut self.inner,
                adjustment.into(),
            )
        };
        from_result(result)
    }

    /// Test whether a terminal point is inside a selection snapshot.
    ///
    /// This uses the same selection semantics as the terminal, including
    /// rectangular/block selections and linear selections spanning multiple rows.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn contains(&self, point: Point) -> Result<bool> {
        let mut contains = false;
        let result = unsafe {
            ffi::ghostty_terminal_selection_contains(
                self.terminal,
                &raw const self.inner,
                point.into(),
                &raw mut contains,
            )
        };
        from_result(result)?;
        Ok(contains)
    }

    /// Test whether two selection snapshots are equal.
    ///
    /// Equality uses the terminal's internal selection semantics: both endpoint
    /// pins must match and both selections must have the same rectangular/block
    /// state. This avoids requiring callers to compare raw [`GridRef`] internals.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn equals(&self, other: &Self) -> Result<bool> {
        let mut equal = false;
        let result = unsafe {
            ffi::ghostty_terminal_selection_equal(
                self.terminal,
                &raw const self.inner,
                &raw const other.inner,
                &raw mut equal,
            )
        };
        from_result(result)?;
        Ok(equal)
    }

    /// Get the current endpoint ordering of a selection snapshot.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn order(&self) -> Result<Order> {
        let mut order = ffi::SelectionOrder::FORWARD;

        let result = unsafe {
            ffi::ghostty_terminal_selection_order(
                self.terminal,
                &raw const self.inner,
                &raw mut order,
            )
        };
        from_result(result)?;
        Order::try_from(order).map_err(|_| Error::Unsupported)
    }

    /// Return a selection snapshot with endpoints ordered as requested.
    ///
    /// Use [`Order::Forward`] to get top-left to bottom-right bounds,
    /// and [`Order::Reverse`] to get bottom-right to top-left bounds.
    /// Mirrored desired orders are accepted but normalized the same as forward.
    /// The output selection is a fresh untracked snapshot and is not installed as
    /// the terminal's current selection.
    ///
    /// See [#Preconditions](#preconditions) for the necessary preconditions.
    pub fn to_ordered(&self, desired: Order) -> Result<Self> {
        let mut selection = ffi::Selection::default();
        let result = unsafe {
            ffi::ghostty_terminal_selection_ordered(
                self.terminal,
                &raw const self.inner,
                desired.into(),
                &raw mut selection,
            )
        };
        from_result(result)?;
        Ok(unsafe { Self::from_raw(self.terminal, selection) })
    }
}

/// Methods related to [selections](crate::selection).
impl Terminal<'_, '_> {
    /// Get the active screen's current selection.
    ///
    /// On success, returns an untracked snapshot of the terminal-owned selection.
    /// The [`Selection`] object is caller-owned and may be kept, but the grid
    /// references inside it are untracked borrowed references into the active screen.
    /// They are only valid until the next mutating terminal call, such as
    /// the various `Terminal::set_*` methods, [`Terminal::vt_write`],
    /// [`Terminal::resize`], or [`Terminal::reset`].
    ///
    /// Returns `Ok(None)` when there is no active selection.
    pub fn selection(&self) -> Result<Option<Selection<'_>>> {
        self.get_optional::<ffi::Selection>(ffi::TerminalData::SELECTION)
            .map(|v| v.map(|raw| unsafe { Selection::from_raw(self.inner.as_raw(), raw) }))
    }

    /// Set the active screen selection.
    ///
    /// The selection's grid references must be valid for this terminal's
    /// active screen at the time of the call.
    ///
    /// Passing `None` clears the active screen selection.
    ///
    /// This function does not take `&mut self` since it does not invalidate
    /// any state that relies on the terminal.
    pub fn set_selection(&self, selection: Option<Selection<'_>>) -> Result<&Self> {
        self.set_optional(
            ffi::TerminalOption::SELECTION,
            selection.as_ref().map(|v| &v.inner),
        )?;
        Ok(self)
    }

    /// Create a new selection between two endpoints.
    ///
    /// Both endpoints are inclusive. The endpoints preserve selection direction
    /// and may be reversed; callers must not assume that start is the top-left
    /// endpoint or that end is the bottom-right endpoint.
    ///
    /// When `rectangle` is false, the endpoints describe a linear selection. When
    /// `rectangle` is true, the same endpoints are interpreted as opposite corners
    /// of a rectangular/block selection.
    ///
    /// This is an alias of [`Selection::new`], albeit slightly simpler to write.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    #[must_use]
    pub fn select<'t>(
        &'t self,
        start: GridRef<'t>,
        end: GridRef<'t>,
        rectangle: bool,
    ) -> Selection<'t> {
        Selection::new(self, start, end, rectangle)
    }

    /// Derive a selection snapshot covering all selectable terminal content.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    pub fn select_all(&self) -> Result<Option<Selection<'_>>> {
        let mut value = ffi::Selection::default();
        let result =
            unsafe { ffi::ghostty_terminal_select_all(self.inner.as_raw(), &raw mut value) };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(self.inner.as_raw(), v)
        }))
    }

    /// Start deriving a selection snapshot covering all selectable terminal content.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    pub fn select_line<'t>(&'t self, grid_ref: GridRef<'t>) -> SelectLine<'t> {
        SelectLine::new(self, grid_ref)
    }

    /// Derive a command-output selection snapshot from a terminal grid reference.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    pub fn select_output<'t>(&'t self, grid_ref: GridRef<'t>) -> Result<Option<Selection<'t>>> {
        let mut value = ffi::Selection::default();

        let result = unsafe {
            ffi::ghostty_terminal_select_output(
                self.inner.as_raw(),
                &raw const grid_ref.inner,
                &raw mut value,
            )
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(self.inner.as_raw(), v)
        }))
    }

    /// Start deriving a word selection snapshot from a terminal grid reference.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    pub fn select_word<'t>(&'t self, grid_ref: GridRef<'t>) -> SelectWord<'t> {
        SelectWord::new(self, grid_ref)
    }

    /// Start deriving the nearest word selection snapshot between two
    /// terminal grid refs.
    ///
    /// Starting at `options.start`, this searches toward `options.end`
    /// (inclusive) and returns the first selectable word found using
    /// Ghostty's word-selection rules.
    ///
    /// This is useful for implementing double-click-and-drag selection in a UI.
    /// If a user double-clicks one word and drags across spaces or punctuation
    /// toward another word, selecting only the word directly under the current
    /// pointer can flicker or collapse when the pointer is between words.
    /// Instead, ask for the nearest word between the original click and the
    /// drag point, ask again in the reverse direction, and combine the two word
    /// bounds into the drag selection.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use libghostty_vt::{
    ///     Error,
    ///     Terminal,
    ///     point::{Point, Coordinate},
    ///     grid_ref::GridRef,
    ///     selection::{Selection, SelectWordBetween},
    /// };
    /// # fn main() -> libghostty_vt::error::Result<()> {
    /// # let terminal = Terminal::new(80, 24).unwrap();
    ///
    /// // Double-click-and-drag style selection. Suppose the user double-clicks
    /// // "git" and drags to "status". The pointer may pass over whitespace, so
    /// // select the nearest word between the original click and current drag point
    /// // in both directions, then combine the outer word bounds.
    /// fn ref_at<'t>(terminal: &'t Terminal<'_, '_>, x: u16, y: u32) -> Result<GridRef<'t>, Error> {
    ///     terminal.grid_ref(Point::Active(Coordinate { x, y }))
    /// }
    ///
    /// let click_ref = ref_at(&terminal, 2, 0)?; // the "git" in "git status"
    /// let drag_ref = ref_at(&terminal, 6, 0)?;  // the "status" in "git status"
    ///
    /// let start_word = terminal.select_word_between(
    ///     SelectWordBetween::new(click_ref.clone(), drag_ref.clone())
    /// )?;
    ///
    /// let end_word = terminal.select_word_between(
    ///     SelectWordBetween::new(drag_ref, click_ref)
    /// )?;
    ///
    /// let drag_selection = Selection::new(
    ///     start_word.unwrap().start(),
    ///     end_word.unwrap().end(),
    ///     false,
    /// );
    /// # Ok(())}
    /// ```
    pub fn select_word_between<'t>(
        &'t self,
        start: GridRef<'t>,
        end: GridRef<'t>,
    ) -> SelectWordBetween<'t> {
        SelectWordBetween::new(self, start, end)
    }
}

/// Options for [deriving a line selection](Terminal::select_line)
/// from a terminal grid reference.
///
/// If [`with_whitespace`](Self::with_whitespace) is not called,
/// Ghostty's default line-trim whitespace codepoints are used.
#[derive(Debug)]
#[must_use]
pub struct SelectLine<'t> {
    terminal: ffi::Terminal,
    inner: ffi::TerminalSelectLineOptions,
    _phan: PhantomData<&'t ffi::Terminal>,
}
impl<'t> SelectLine<'t> {
    /// Create a new set of options for [deriving a line selection](Terminal::select_line),
    /// from the given grid reference.
    pub fn new(terminal: &'t Terminal<'_, '_>, grid_ref: GridRef<'t>) -> Self {
        Self {
            terminal: terminal.inner.as_raw(),
            inner: ffi::TerminalSelectLineOptions {
                ref_: grid_ref.inner,
                whitespace: std::ptr::null(),
                whitespace_len: 0,
                semantic_prompt_boundary: false,
                ..ffi::sized!(ffi::TerminalSelectLineOptions)
            },
            _phan: PhantomData,
        }
    }

    /// Specify the codepoints to trim from the start and end of the line.
    pub fn with_whitespace(mut self, value: &[char]) -> Self {
        // Note: it's always safe to reinterpret char as a u32,
        // as long as no mutation occurs.
        self.inner.whitespace = value.as_ptr().cast();
        self.inner.whitespace_len = value.len();
        self
    }

    /// Specify whether semantic prompt state changes should bound the line
    /// selection.
    pub fn with_semantic_prompt_boundary(mut self, value: bool) -> Self {
        self.inner.semantic_prompt_boundary = value;
        self
    }

    /// Finish selecting a line.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    pub fn finish(self) -> Result<Option<Selection<'t>>> {
        let mut value = ffi::Selection::default();

        let result = unsafe {
            ffi::ghostty_terminal_select_line(self.terminal, &raw const self.inner, &raw mut value)
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(self.terminal, v)
        }))
    }
}

/// Options for [deriving a word selection](Terminal::select_word)
/// from a terminal grid reference.
///
/// If [`with_boundary_codepoints`](Self::with_boundary_codepoints)
/// is not called, Ghostty's default word-boundary codepoints are used.
#[derive(Debug)]
#[must_use]
pub struct SelectWord<'t> {
    terminal: ffi::Terminal,
    inner: ffi::TerminalSelectWordOptions,
    _phan: PhantomData<&'t ffi::Terminal>,
}
impl<'t> SelectWord<'t> {
    fn new(terminal: &'t Terminal<'_, '_>, grid_ref: GridRef<'t>) -> Self {
        Self {
            terminal: terminal.inner.as_raw(),
            inner: ffi::TerminalSelectWordOptions {
                ref_: grid_ref.inner,
                ..ffi::sized!(ffi::TerminalSelectWordOptions)
            },
            _phan: PhantomData,
        }
    }

    /// Specify the word-boundary codepoints.
    pub fn with_boundary_codepoints(mut self, value: &[char]) -> Self {
        // Note: it's always safe to reinterpret char as a u32,
        // as long as no mutation occurs.
        self.inner.boundary_codepoints = value.as_ptr().cast();
        self.inner.boundary_codepoints_len = value.len();
        self
    }

    /// Finish selecting a word.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    pub fn finish(self) -> Result<Option<Selection<'t>>> {
        let mut value = ffi::Selection::default();

        let result = unsafe {
            ffi::ghostty_terminal_select_word(self.terminal, &raw const self.inner, &raw mut value)
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(self.terminal, v)
        }))
    }
}

/// Options for [deriving the nearest word selection](Terminal::select_word_between)
/// between two grid references.
///
/// If [`with_boundary_codepoints`](Self::with_boundary_codepoints)
/// is not called, Ghostty's default word-boundary codepoints are used.
#[derive(Debug)]
#[must_use]
pub struct SelectWordBetween<'t> {
    terminal: ffi::Terminal,
    inner: ffi::TerminalSelectWordBetweenOptions,
    _phan: PhantomData<&'t ffi::Terminal>,
}
impl<'t> SelectWordBetween<'t> {
    fn new(terminal: &'t Terminal<'_, '_>, start: GridRef<'t>, end: GridRef<'t>) -> Self {
        Self {
            terminal: terminal.inner.as_raw(),
            inner: ffi::TerminalSelectWordBetweenOptions {
                start: start.inner,
                end: end.inner,
                ..ffi::sized!(ffi::TerminalSelectWordBetweenOptions)
            },
            _phan: PhantomData,
        }
    }

    /// Specify the word-boundary codepoints.
    pub fn with_boundary_codepoints(mut self, value: &[char]) -> Self {
        // Note: it's always safe to reinterpret char as a u32,
        // as long as no mutation occurs.
        self.inner.boundary_codepoints = value.as_ptr().cast();
        self.inner.boundary_codepoints_len = value.len();
        self
    }

    /// Finish selecting a word between two points.
    ///
    /// The returned selection is not installed as the terminal's current selection.
    pub fn finish(self) -> Result<Option<Selection<'t>>> {
        let mut value = ffi::Selection::default();

        let result = unsafe {
            ffi::ghostty_terminal_select_word_between(
                self.terminal,
                &raw const self.inner,
                &raw mut value,
            )
        };

        let sel = from_optional_result(result, value)?;
        Ok(sel.map(|v| unsafe {
            // SAFETY: Selection should be initialized and valid on success
            Selection::from_raw(self.terminal, v)
        }))
    }
}

/// Operation used to adjust a selection endpoint.
///
/// Adjustment mutates the selection's logical end endpoint, not whichever
/// endpoint is visually bottom/right. This preserves keyboard and drag behavior
/// for both forward and reversed selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum Adjustment {
    /// Move left to the previous non-empty cell, wrapping upward.
    Left = ffi::SelectionAdjust::LEFT,
    /// Move right to the next non-empty cell, wrapping downward.
    Right = ffi::SelectionAdjust::RIGHT,
    /// Move up one row at the current column,
    /// or to the beginning of the line if already at the top.
    Up = ffi::SelectionAdjust::UP,
    /// Move down to the next non-blank row at the current column,
    /// or to the end of the line if none exists.
    Down = ffi::SelectionAdjust::DOWN,
    /// Move to the top-left cell of the screen.
    Home = ffi::SelectionAdjust::HOME,
    /// Move to the right edge of the last non-blank row on the screen.
    End = ffi::SelectionAdjust::END,
    /// Move up by one terminal page height,
    /// or to home if that would move past the top.
    PageUp = ffi::SelectionAdjust::PAGE_UP,
    /// Move down by one terminal page height,
    /// or to end if that would move past the bottom.
    PageDown = ffi::SelectionAdjust::PAGE_DOWN,
    /// Move to the left edge of the current line.
    BeginningOfLine = ffi::SelectionAdjust::BEGINNING_OF_LINE,
    /// Move to the right edge of the current line.
    EndOfLine = ffi::SelectionAdjust::END_OF_LINE,
}

/// Ordering of a selection's endpoints in terminal coordinates.
///
/// Mirrored orders are only produced by rectangular selections whose start
/// and end endpoints are on opposite diagonal corners that are not simple
/// top-left-to-bottom-right or bottom-right-to-top-left orderings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum Order {
    /// Start is before end in top-left to bottom-right order.
    Forward = ffi::SelectionOrder::FORWARD,
    /// End is before start in top-left to bottom-right order.
    Reverse = ffi::SelectionOrder::REVERSE,
    /// Rectangular selection from top-right to bottom-left.
    MirroredForward = ffi::SelectionOrder::MIRRORED_FORWARD,
    /// Rectangular selection from bottom-left to top-right.
    MirroredReverse = ffi::SelectionOrder::MIRRORED_REVERSE,
}
