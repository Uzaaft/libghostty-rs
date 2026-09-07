//! These scenarios run against real Ghostty in normal tests. Under Miri, Rust
//! stubs model the native ownership contract; they do not execute Zig or validate
//! its parser. Keep native runs as an independent check of that model.
use libghostty_vt::{Error, Terminal, search::Search};

#[cfg(miri)]
mod native_model {
    use libghostty_vt::ffi;
    use std::{ffi::c_void, ptr};

    // Model the C ownership boundary, not the Rust wrappers. Native searches retain
    // terminal identity, free detaches registered searches, and exported selections
    // contain untracked terminal-owned page pointers. No parser emulation is needed:
    // the fixture contains one matching cell once its needle has been configured.
    struct NativeTerminal {
        node: *mut ffi::Cell,
        searches: Vec<*mut NativeSearch>,
    }
    struct NativeSearch {
        terminal: *mut NativeTerminal,
        has_needle: bool,
        selected: bool,
        last_selection: Option<ffi::Selection>,
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_terminal_new(
        _: *const ffi::Allocator,
        out: *mut ffi::Terminal,
        _: u16,
        _: u16,
    ) -> ffi::Result::Type {
        let node = Box::into_raw(Box::new(0));
        unsafe {
            *out = Box::into_raw(Box::new(NativeTerminal {
                node,
                searches: Vec::new(),
            }))
            .cast();
        }
        ffi::Result::SUCCESS
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_terminal_free(terminal: ffi::Terminal) {
        let state = unsafe { Box::from_raw(terminal.cast::<NativeTerminal>()) };
        // Match upstream TerminalWrapper.deinit: detach searches before freeing pages.
        for &search in &state.searches {
            unsafe {
                (*search).terminal = ptr::null_mut();
            }
        }
        unsafe {
            drop(Box::from_raw(state.node));
        }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_search_new(
        _: *const ffi::Allocator,
        out: *mut ffi::Search,
        terminal: ffi::Terminal,
    ) -> ffi::Result::Type {
        let terminal = terminal.cast::<NativeTerminal>();
        let search = Box::into_raw(Box::new(NativeSearch {
            terminal,
            has_needle: false,
            selected: false,
            last_selection: None,
        }));
        unsafe {
            (*terminal).searches.push(search);
            *out = search.cast();
        }
        ffi::Result::SUCCESS
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_search_free(search: ffi::Search) {
        let raw = search.cast::<NativeSearch>();
        let state = unsafe { Box::from_raw(raw) };
        if !state.terminal.is_null() {
            unsafe {
                (*state.terminal).searches.retain(|&search| search != raw);
            }
        }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_search_feed(search: ffi::Search) -> ffi::Result::Type {
        let state = unsafe { &mut *search.cast::<NativeSearch>() };
        if state.terminal.is_null() {
            return ffi::Result::INVALID_VALUE;
        }
        if state.has_needle {
            let node = unsafe { (*state.terminal).node };
            let grid_ref = ffi::GridRef {
                node: node.cast(),
                ..ffi::sized!(ffi::GridRef)
            };
            state.last_selection = Some(ffi::Selection {
                start: grid_ref,
                end: grid_ref,
                ..ffi::sized!(ffi::Selection)
            });
        }
        ffi::Result::SUCCESS
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_search_run(search: ffi::Search) -> ffi::Result::Type {
        unsafe { ghostty_search_feed(search) }
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_search_set(
        search: ffi::Search,
        option: ffi::SearchOption::Type,
        value: *const c_void,
    ) -> ffi::Result::Type {
        let state = unsafe { &mut *search.cast::<NativeSearch>() };
        if state.terminal.is_null() {
            return ffi::Result::INVALID_VALUE;
        }
        match option {
            ffi::SearchOption::NEEDLE => {
                state.has_needle =
                    !value.is_null() && unsafe { (*value.cast::<ffi::String>()).len > 0 };
                state.selected = false;
                state.last_selection = None;
            }
            ffi::SearchOption::SELECT_NEXT => {
                if !state.has_needle {
                    return ffi::Result::NO_VALUE;
                }
                state.selected = true;
            }
            _ => return ffi::Result::INVALID_VALUE,
        }
        ffi::Result::SUCCESS
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_search_get(
        search: ffi::Search,
        key: ffi::SearchData::Type,
        out: *mut c_void,
    ) -> ffi::Result::Type {
        let state = unsafe { &*search.cast::<NativeSearch>() };
        if matches!(
            key,
            ffi::SearchData::MATCHES | ffi::SearchData::VIEWPORT_MATCHES
        ) {
            let buffer = unsafe { &mut *out.cast::<ffi::SelectionBuffer>() };
            buffer.len = usize::from(state.last_selection.is_some());
            if buffer.cap < buffer.len {
                return ffi::Result::OUT_OF_SPACE;
            }
            if let Some(selection) = state.last_selection {
                unsafe {
                    *buffer.ptr = selection;
                }
            }
            return ffi::Result::SUCCESS;
        }
        assert_eq!(key, ffi::SearchData::SELECTED_MATCH);
        if !state.selected {
            return ffi::Result::NO_VALUE;
        }
        let Some(selection) = state.last_selection else {
            return ffi::Result::NO_VALUE;
        };
        unsafe {
            *out.cast::<ffi::Selection>() = selection;
        }
        ffi::Result::SUCCESS
    }
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_grid_ref_cell(
        grid_ref: *const ffi::GridRef,
        out: *mut ffi::Cell,
    ) -> ffi::Result::Type {
        // Like the native accessor, reading an untracked ref dereferences its page.
        unsafe {
            *out = *(*grid_ref).node.cast::<ffi::Cell>();
        }
        ffi::Result::SUCCESS
    }
    // This fixture always contains one matching cell. Parsing is tested by the
    // native run; the model only needs the allocation behind that cell.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_terminal_vt_write(_: ffi::Terminal, _: *const u8, _: usize) {}
}

fn fixture(terminal: &mut Terminal<'_, '_>) -> Search<'static> {
    terminal.vt_write(b"fixture");
    let mut search = Search::new(terminal).unwrap();
    search.set_needle(terminal, b"fixture").unwrap();
    search.run(terminal).unwrap();
    assert!(search.select_next(terminal).unwrap());
    search
}

fn rejects_wrong_terminal(search: &mut Search<'_>, terminal: &mut Terminal<'_, '_>) {
    assert!(matches!(search.feed(terminal), Err(Error::InvalidValue)));
    assert!(matches!(search.run(terminal), Err(Error::InvalidValue)));
    assert!(matches!(
        search.set_needle(terminal, b"other"),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        search.select_next(terminal),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        search.select_prev(terminal),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        search.snapshot(terminal),
        Err(Error::InvalidValue)
    ));
}

#[test]
fn search_can_be_dropped_before_terminal() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut search = fixture(&mut terminal);
    let snapshot = search.snapshot(&mut terminal).unwrap();
    snapshot
        .selected_match()
        .unwrap()
        .unwrap()
        .start()
        .cell()
        .unwrap();
    drop(search);
    terminal.vt_write(b"more");
}

#[test]
fn swapping_terminals_preserves_identity_and_snapshot_owner() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut other = Terminal::new(8, 2).unwrap();
    let mut search = fixture(&mut terminal);
    std::mem::swap(&mut terminal, &mut other);
    rejects_wrong_terminal(&mut search, &mut terminal);
    let snapshot = search.snapshot(&mut other).unwrap();
    let selected = snapshot.selected_match().unwrap().unwrap();
    // This is the replacement, not the owner of selected's pages.
    drop(terminal);
    selected.start().cell().unwrap();
    drop(search);
    drop(other);
}

#[test]
fn replacing_and_freeing_original_rejects_new_terminal() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut search = fixture(&mut terminal);
    let original = std::mem::replace(&mut terminal, Terminal::new(8, 2).unwrap());
    drop(original);
    rejects_wrong_terminal(&mut search, &mut terminal);
    // Also give the new terminal its own identity; two initialized identities
    // must remain distinct, not just Some versus None.
    let replacement_search = fixture(&mut terminal);
    rejects_wrong_terminal(&mut search, &mut terminal);
    drop(search);
    drop(replacement_search);
}

#[test]
fn multiple_searches_share_identity_and_detach_on_terminal_drop() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut first = fixture(&mut terminal);
    let mut second = Search::new(&mut terminal).unwrap();
    second.set_needle(&mut terminal, b"fixture").unwrap();
    second.run(&mut terminal).unwrap();
    assert!(second.select_next(&mut terminal).unwrap());
    first.feed(&mut terminal).unwrap();
    drop(terminal);
    drop(first);
    drop(second);
}

#[test]
fn match_storage_reuses_only_fresh_snapshots() {
    use libghostty_vt::search::MatchBuffer;
    let mut storage = MatchBuffer::new();
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut search = fixture(&mut terminal);
    {
        let snapshot = search.snapshot(&mut terminal).unwrap();
        let mut matches = snapshot.matches(&mut storage).unwrap();
        assert_eq!(matches.len(), 1);
        matches.next_back().unwrap().start().cell().unwrap();
        assert!(matches.next().is_none());
        assert!(matches.next_back().is_none());
    }
    search.set_needle(&mut terminal, b"").unwrap();
    assert_eq!(
        search
            .snapshot(&mut terminal)
            .unwrap()
            .matches(&mut storage)
            .unwrap()
            .len(),
        0
    );
    drop(terminal);
    drop(search);
    // Storage still contains raw values from the old terminal. A new read must
    // overwrite them before handing out references bounded by this new owner.
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut search = fixture(&mut terminal);
    let snapshot = search.snapshot(&mut terminal).unwrap();
    let selected = snapshot
        .viewport_matches(&mut storage)
        .unwrap()
        .next()
        .unwrap();
    selected.start().cell().unwrap();
}
