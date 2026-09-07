//! Search active terminal content and scrollback with bounded or blocking work.
//!
//! A search holds exclusive access to its terminal. Use [`Search::terminal_mut`]
//! for writes, then feed or run again to refresh results. Match selections borrow
//! the search, preventing terminal mutation while those snapshots remain in use.

use crate::{
    Terminal,
    alloc::{Allocator, Object},
    error::{Error, Result, from_optional_result, from_result},
    ffi,
    selection::Selection,
};

/// A search bound to one terminal, using byte-exact, ASCII case-insensitive matching.
#[derive(Debug)]
pub struct Search<'t, 'alloc: 'cb, 'cb: 't> {
    inner: Object<'alloc, ffi::SearchImpl>,
    terminal: &'t mut Terminal<'alloc, 'cb>,
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

impl<'t, 'alloc: 'cb, 'cb: 't> Search<'t, 'alloc, 'cb> {
    /// Create an idle search with the default allocator.
    pub fn new(terminal: &'t mut Terminal<'alloc, 'cb>) -> Result<Self> {
        // SAFETY: NULL selects the default allocator.
        unsafe { Self::new_inner(terminal, std::ptr::null()) }
    }

    /// Create an idle search with a custom allocator that outlives it.
    pub fn new_with_alloc<'ctx: 'alloc>(
        terminal: &'t mut Terminal<'alloc, 'cb>,
        alloc: &'alloc Allocator<'ctx>,
    ) -> Result<Self> {
        // SAFETY: The allocator's lifetime is retained by Object.
        unsafe { Self::new_inner(terminal, alloc.to_raw()) }
    }

    unsafe fn new_inner(
        terminal: &'t mut Terminal<'alloc, 'cb>,
        alloc: *const ffi::Allocator,
    ) -> Result<Self> {
        let mut raw = std::ptr::null_mut();
        from_result(unsafe { ffi::ghostty_search_new(alloc, &mut raw, terminal.inner.as_raw()) })?;
        Ok(Self {
            inner: Object::new(raw)?,
            terminal,
        })
    }

    /// Access the terminal for reading and formatting current matches.
    pub fn terminal(&self) -> &Terminal<'alloc, 'cb> {
        self.terminal
    }
    /// Access the terminal for writes or resizing. Feed again to observe changes.
    pub fn terminal_mut(&mut self) -> &mut Terminal<'alloc, 'cb> {
        self.terminal
    }

    /// Set a copied needle. Empty clears it; an equivalent needle preserves results.
    pub fn set_needle(&mut self, needle: &[u8]) -> Result<&mut Self> {
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
    pub fn feed(&mut self) -> Result<()> {
        from_result(unsafe { ffi::ghostty_search_feed(self.inner.as_raw()) })
    }
    /// Make a bounded amount of progress on copied search data.
    pub fn tick(&mut self) -> Result<Status> {
        let mut status = 0;
        from_result(unsafe { ffi::ghostty_search_tick(self.inner.as_raw(), &mut status) })?;
        status.try_into().map_err(|_| Error::InvalidValue)
    }
    /// Feed and tick until caught up. Large scrollback searches can block.
    pub fn run(&mut self) -> Result<()> {
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
    fn select(&mut self, key: ffi::SearchOption::Type) -> Result<bool> {
        let code = unsafe { ffi::ghostty_search_set(self.inner.as_raw(), key, std::ptr::null()) };
        Ok(from_optional_result(code, ())?.is_some())
    }
    /// Select toward older content, wrapping around. False means no matches.
    pub fn select_next(&mut self) -> Result<bool> {
        self.select(ffi::SearchOption::SELECT_NEXT)
    }
    /// Select toward newer content, wrapping around. False means no matches.
    pub fn select_prev(&mut self) -> Result<bool> {
        self.select(ffi::SearchOption::SELECT_PREV)
    }

    /// Refresh matches and borrow a view that can also read their terminal.
    /// The view blocks writes and navigation until all match snapshots are dropped.
    pub fn snapshot(&mut self) -> Result<Snapshot<'_, 't, 'alloc, 'cb>> {
        self.feed()?;
        Ok(Snapshot { search: self })
    }
}

/// Refreshed search results and their terminal, borrowed together for safe formatting.
#[derive(Debug)]
pub struct Snapshot<'s, 't, 'alloc: 'cb, 'cb: 't> {
    search: &'s Search<'t, 'alloc, 'cb>,
}

impl<'alloc: 'cb, 'cb> Snapshot<'_, '_, 'alloc, 'cb> {
    /// The terminal that produced these matches.
    pub fn terminal(&self) -> &Terminal<'alloc, 'cb> {
        self.search.terminal()
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

impl Drop for Search<'_, '_, '_> {
    fn drop(&mut self) {
        // The terminal and allocator are still alive while tracked state is freed.
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
        assert!(!search.select_next().unwrap());
        search.set_needle(b"HELLO").unwrap();
        assert_eq!(search.needle().unwrap(), Some(&b"HELLO"[..]));
        search.run().unwrap();
        assert_eq!(search.total_matches().unwrap(), 2);
        assert_eq!(search.snapshot().unwrap().matches().unwrap().len(), 2);
        search.set_scroll(Scroll::None).unwrap();
        assert_eq!(search.scroll().unwrap(), Scroll::None);
        assert!(search.select_next().unwrap());
        assert!(search.selected_index().unwrap().is_some());
        assert!(
            search
                .snapshot()
                .unwrap()
                .selected_match()
                .unwrap()
                .is_some()
        );
        search.terminal_mut().vt_write(b"\r\nhello");
        search.run().unwrap();
        assert_eq!(search.total_matches().unwrap(), 3);
        assert_eq!(
            search.snapshot().unwrap().viewport_matches().unwrap().len(),
            3
        );
        search.set_needle(b"").unwrap();
        assert!(search.snapshot().unwrap().matches().unwrap().is_empty());
        assert_eq!(search.needle().unwrap(), None);
    }
}
