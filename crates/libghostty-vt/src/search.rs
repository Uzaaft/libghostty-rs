//! Search active terminal content and scrollback with bounded or blocking work.
//!
//! A search is bound to one terminal, but borrows it only during operations.
//! Mutate the terminal normally between feeds. Match snapshots borrow both the
//! search and its terminal so their untracked page references cannot be invalidated.

use std::rc::Rc;

use crate::{
    Terminal,
    alloc::{Allocator, Object},
    error::{Error, Result, from_optional_result, from_result},
    ffi,
    selection::Selection,
};

/// A search bound to one terminal, using byte-exact, ASCII case-insensitive matching.
///
/// Operations taking a terminal return [`Error::InvalidValue`] if it is not the
/// original terminal. Moving or swapping the original terminal is fine: identity
/// follows its native handle. A replacement terminal cannot be used with this search.
///
/// The terminal can be dropped before the search. Ghostty detaches the native
/// search during terminal destruction; dropping the search then frees only its
/// own storage. The search allocator must still outlive the search.
///
/// Match snapshots prevent writes while their selections are in use:
///
/// ```compile_fail,E0499
/// use libghostty_vt::{Terminal, search::Search};
/// let mut terminal = Terminal::new(8, 2).unwrap();
/// let mut search = Search::new(&mut terminal).unwrap();
/// let snapshot = search.snapshot(&mut terminal).unwrap();
/// let selected = snapshot.selected_match().unwrap().unwrap();
/// terminal.vt_write(b"\x1bc");
/// selected.start().cell().unwrap();
/// ```
///
/// They also prevent replacing and freeing the terminal:
///
/// ```compile_fail,E0499
/// use libghostty_vt::{Terminal, search::Search};
/// let mut terminal = Terminal::new(8, 2).unwrap();
/// let mut search = Search::new(&mut terminal).unwrap();
/// let snapshot = search.snapshot(&mut terminal).unwrap();
/// let selected = snapshot.selected_match().unwrap().unwrap();
/// let original = std::mem::replace(&mut terminal, Terminal::new(8, 2).unwrap());
/// drop(original);
/// selected.start().cell().unwrap();
/// ```
///
/// Other searches cannot mutate the same terminal while a snapshot is borrowed:
///
/// ```compile_fail,E0499
/// use libghostty_vt::{Terminal, search::Search};
/// let mut terminal = Terminal::new(8, 2).unwrap();
/// let mut first = Search::new(&mut terminal).unwrap();
/// let mut second = Search::new(&mut terminal).unwrap();
/// let snapshot = first.snapshot(&mut terminal).unwrap();
/// second.select_next(&mut terminal).unwrap();
/// snapshot.selected_match().unwrap();
/// ```
#[derive(Debug)]
pub struct Search<'alloc> {
    inner: Object<'alloc, ffi::SearchImpl>,
    terminal: Rc<()>,
}

/// Progress as of the last feed or tick; complete searches still need future feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
pub enum Status {
    /// A tick can make progress on already copied data.
    Running = ffi::SearchStatus::RUNNING,
    /// Feed terminal data before ticking again.
    FeedRequired = ffi::SearchStatus::FEED_REQUIRED,
    /// Caught up with the last feed, or idle without a needle.
    Complete = ffi::SearchStatus::COMPLETE,
}

/// Viewport scrolling when selecting a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
pub enum Scroll {
    /// Reveal selected matches when they are outside the viewport.
    IfNeeded = ffi::SearchScroll::IF_NEEDED,
    /// Leave the viewport unchanged.
    None = ffi::SearchScroll::NONE,
}

impl<'alloc> Search<'alloc> {
    /// Create an idle search with the default allocator.
    pub fn new(terminal: &mut Terminal<'_, '_>) -> Result<Self> {
        // SAFETY: NULL selects the default allocator.
        unsafe { Self::new_inner(terminal, std::ptr::null()) }
    }

    /// Create an idle search with a custom allocator that outlives it.
    pub fn new_with_alloc<'ctx: 'alloc>(
        terminal: &mut Terminal<'_, '_>,
        alloc: &'alloc Allocator<'ctx>,
    ) -> Result<Self> {
        // SAFETY: The allocator's lifetime is retained by Object.
        unsafe { Self::new_inner(terminal, alloc.to_raw()) }
    }

    unsafe fn new_inner(
        terminal: &mut Terminal<'_, '_>,
        alloc: *const ffi::Allocator,
    ) -> Result<Self> {
        let mut raw = std::ptr::null_mut();
        from_result(unsafe { ffi::ghostty_search_new(alloc, &mut raw, terminal.inner.as_raw()) })?;
        Ok(Self {
            inner: Object::new(raw)?,
            terminal: Rc::clone(terminal.search_identity.get_or_insert_with(|| Rc::new(()))),
        })
    }

    // Comparing native addresses is insufficient: a freed terminal's address may
    // be reused. The shared token keeps identity unique until its last search dies.
    fn check_terminal(&self, terminal: &Terminal<'_, '_>) -> Result<()> {
        match &terminal.search_identity {
            Some(identity) if Rc::ptr_eq(identity, &self.terminal) => Ok(()),
            _ => Err(Error::InvalidValue),
        }
    }

    /// Set a copied needle. Empty clears it; an equivalent needle preserves results.
    pub fn set_needle(
        &mut self,
        terminal: &mut Terminal<'_, '_>,
        needle: &[u8],
    ) -> Result<&mut Self> {
        self.check_terminal(terminal)?;
        let raw = ffi::String {
            ptr: needle.as_ptr(),
            len: needle.len(),
        };
        from_result(unsafe {
            ffi::ghostty_search_set(
                self.inner.as_raw(),
                ffi::SearchOption::NEEDLE,
                std::ptr::from_ref(&raw).cast(),
            )
        })?;
        Ok(self)
    }

    /// Borrow the current needle, or `None` when idle.
    pub fn needle(&self) -> Result<Option<&[u8]>> {
        let mut raw = ffi::String::default();
        let code = unsafe {
            ffi::ghostty_search_get(
                self.inner.as_raw(),
                ffi::SearchData::NEEDLE,
                std::ptr::from_mut(&mut raw).cast(),
            )
        };
        // SAFETY: The bytes belong to the search and cannot change during this borrow.
        Ok(from_optional_result(code, raw)?.map(|raw| unsafe { raw.to_bytes() }))
    }

    /// Copy a bounded amount of terminal data and reconcile terminal changes.
    pub fn feed(&mut self, terminal: &mut Terminal<'_, '_>) -> Result<()> {
        self.check_terminal(terminal)?;
        from_result(unsafe { ffi::ghostty_search_feed(self.inner.as_raw()) })
    }
    /// Make a bounded amount of progress on copied search data.
    pub fn tick(&mut self) -> Result<Status> {
        let mut status = 0;
        from_result(unsafe { ffi::ghostty_search_tick(self.inner.as_raw(), &mut status) })?;
        status.try_into().map_err(|_| Error::InvalidValue)
    }
    /// Feed and tick until caught up. Large scrollback searches can block.
    pub fn run(&mut self, terminal: &mut Terminal<'_, '_>) -> Result<()> {
        self.check_terminal(terminal)?;
        from_result(unsafe { ffi::ghostty_search_run(self.inner.as_raw()) })
    }

    // Only the explicitly typed accessors below may select an output type.
    fn get<T: Default>(&self, key: ffi::SearchData::Type) -> Result<T> {
        let mut value = T::default();
        from_result(unsafe {
            ffi::ghostty_search_get(
                self.inner.as_raw(),
                key,
                std::ptr::from_mut(&mut value).cast(),
            )
        })?;
        Ok(value)
    }
    /// Current progress state.
    pub fn status(&self) -> Result<Status> {
        self.get::<ffi::SearchStatus::Type>(ffi::SearchData::STATUS)?
            .try_into()
            .map_err(|_| Error::InvalidValue)
    }
    /// Matches found so far on the active screen as of the last feed.
    pub fn total_matches(&self) -> Result<usize> {
        self.get(ffi::SearchData::TOTAL_MATCHES)
    }
    /// Selected match index in newest-to-oldest order, as of the last feed.
    pub fn selected_index(&self) -> Result<Option<usize>> {
        let mut index = 0usize;
        let code = unsafe {
            ffi::ghostty_search_get(
                self.inner.as_raw(),
                ffi::SearchData::SELECTED_INDEX,
                std::ptr::from_mut(&mut index).cast(),
            )
        };
        from_optional_result(code, index)
    }
    /// Current viewport scrolling policy.
    pub fn scroll(&self) -> Result<Scroll> {
        self.get::<ffi::SearchScroll::Type>(ffi::SearchData::SELECT_SCROLL)?
            .try_into()
            .map_err(|_| Error::InvalidValue)
    }
    /// Choose whether selecting a match scrolls it into view.
    pub fn set_scroll(&mut self, scroll: Scroll) -> Result<&mut Self> {
        let raw: ffi::SearchScroll::Type = scroll.into();
        from_result(unsafe {
            ffi::ghostty_search_set(
                self.inner.as_raw(),
                ffi::SearchOption::SELECT_SCROLL,
                std::ptr::from_ref(&raw).cast(),
            )
        })?;
        Ok(self)
    }
    fn select(
        &mut self,
        terminal: &mut Terminal<'_, '_>,
        key: ffi::SearchOption::Type,
    ) -> Result<bool> {
        self.check_terminal(terminal)?;
        let code = unsafe { ffi::ghostty_search_set(self.inner.as_raw(), key, std::ptr::null()) };
        Ok(from_optional_result(code, ())?.is_some())
    }
    /// Select toward older content, wrapping around. False means no matches.
    pub fn select_next(&mut self, terminal: &mut Terminal<'_, '_>) -> Result<bool> {
        self.select(terminal, ffi::SearchOption::SELECT_NEXT)
    }
    /// Select toward newer content, wrapping around. False means no matches.
    pub fn select_prev(&mut self, terminal: &mut Terminal<'_, '_>) -> Result<bool> {
        self.select(terminal, ffi::SearchOption::SELECT_PREV)
    }

    /// Refresh matches and borrow a view that can also read their terminal.
    /// The view blocks writes and navigation until all match snapshots are dropped.
    pub fn snapshot<'s, 'ta: 'cb, 'cb>(
        &'s mut self,
        terminal: &'s mut Terminal<'ta, 'cb>,
    ) -> Result<Snapshot<'s, 'alloc, 'ta, 'cb>> {
        self.feed(terminal)?;
        Ok(Snapshot {
            search: self,
            terminal,
        })
    }
}

/// Refreshed search results and their terminal, borrowed together for safe formatting.
#[derive(Debug)]
pub struct Snapshot<'s, 'alloc, 'ta: 'cb, 'cb> {
    search: &'s Search<'alloc>,
    terminal: &'s Terminal<'ta, 'cb>,
}

impl<'ta: 'cb, 'cb> Snapshot<'_, '_, 'ta, 'cb> {
    /// The terminal that produced these matches.
    pub fn terminal(&self) -> &Terminal<'ta, 'cb> {
        self.terminal
    }
    /// Borrow the selected match, preventing terminal mutation.
    pub fn selected_match(&self) -> Result<Option<Selection<'_>>> {
        let mut raw = ffi::sized!(ffi::Selection);
        let code = unsafe {
            ffi::ghostty_search_get(
                self.search.inner.as_raw(),
                ffi::SearchData::SELECTED_MATCH,
                std::ptr::from_mut(&mut raw).cast(),
            )
        };
        Ok(from_optional_result(code, raw)?.map(|raw| unsafe { Selection::from_raw(raw) }))
    }

    fn read_matches(&self, key: ffi::SearchData::Type) -> Result<Vec<Selection<'_>>> {
        let mut buffer = ffi::SelectionBuffer::default();
        let code = unsafe {
            ffi::ghostty_search_get(
                self.search.inner.as_raw(),
                key,
                std::ptr::from_mut(&mut buffer).cast(),
            )
        };
        if code != ffi::Result::OUT_OF_SPACE {
            from_result(code)?;
        }
        // Initialize every sized output. The second call runs without terminal
        // mutation, so the required capacity cannot increase between calls.
        let mut values = vec![ffi::sized!(ffi::Selection); buffer.len];
        buffer.ptr = values.as_mut_ptr();
        buffer.cap = values.len();
        from_result(unsafe {
            ffi::ghostty_search_get(
                self.search.inner.as_raw(),
                key,
                std::ptr::from_mut(&mut buffer).cast(),
            )
        })?;
        values.truncate(buffer.len);
        Ok(values
            .into_iter()
            .map(|raw| unsafe { Selection::from_raw(raw) })
            .collect())
    }
    /// Borrow all matches in newest-to-oldest order.
    pub fn matches(&self) -> Result<Vec<Selection<'_>>> {
        self.read_matches(ffi::SearchData::MATCHES)
    }
    /// Borrow matches on pages covering the viewport.
    /// Matches sharing those pages can extend beyond the visible rows.
    pub fn viewport_matches(&self) -> Result<Vec<Selection<'_>>> {
        self.read_matches(ffi::SearchData::VIEWPORT_MATCHES)
    }
}

impl Drop for Search<'_> {
    fn drop(&mut self) {
        // Ghostty unregisters from a live terminal or frees detached search storage.
        // The allocator lifetime is retained even if the terminal was dropped first.
        unsafe { ffi::ghostty_search_free(self.inner.as_raw()) };
    }
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;

    #[test]
    fn search_refreshes_after_terminal_mutation() {
        let mut terminal = Terminal::new(30, 4).unwrap();
        terminal.vt_write(b"Hello hello\r\nother");
        let mut search = Search::new(&mut terminal).unwrap();
        assert_eq!(search.status().unwrap(), Status::Complete);
        assert!(!search.select_next(&mut terminal).unwrap());
        search.set_needle(&mut terminal, b"HELLO").unwrap();
        assert_eq!(search.needle().unwrap(), Some(&b"HELLO"[..]));
        search.run(&mut terminal).unwrap();
        assert_eq!(search.total_matches().unwrap(), 2);
        assert_eq!(
            search
                .snapshot(&mut terminal)
                .unwrap()
                .matches()
                .unwrap()
                .len(),
            2
        );
        search.set_scroll(Scroll::None).unwrap();
        assert_eq!(search.scroll().unwrap(), Scroll::None);
        assert!(search.select_next(&mut terminal).unwrap());
        assert!(search.selected_index().unwrap().is_some());
        assert!(
            search
                .snapshot(&mut terminal)
                .unwrap()
                .selected_match()
                .unwrap()
                .is_some()
        );
        terminal.vt_write(b"\r\nhello");
        search.run(&mut terminal).unwrap();
        assert_eq!(search.total_matches().unwrap(), 3);
        assert_eq!(
            search
                .snapshot(&mut terminal)
                .unwrap()
                .viewport_matches()
                .unwrap()
                .len(),
            3
        );
        terminal.resize(40, 6, 8, 16).unwrap();
        assert_eq!(terminal.cols().unwrap(), 40);
        assert_eq!(terminal.rows().unwrap(), 6);
        search.run(&mut terminal).unwrap();
        assert_eq!(search.total_matches().unwrap(), 3);
        let snapshot = search.snapshot(&mut terminal).unwrap();
        for selected in snapshot.matches().unwrap() {
            selected.start().cell().unwrap();
            selected.end().cell().unwrap();
        }
        search.set_needle(&mut terminal, b"").unwrap();
        assert!(
            search
                .snapshot(&mut terminal)
                .unwrap()
                .matches()
                .unwrap()
                .is_empty()
        );
        assert_eq!(search.needle().unwrap(), None);
    }
}
