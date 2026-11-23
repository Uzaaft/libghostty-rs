//! Ownership scenarios between searches and their terminals, run against the
//! real library.
// These call into libghostty, which Miri cannot execute.
#![cfg(not(miri))]

use libghostty_vt::{Error, Terminal, search::Search};

fn fixture(terminal: &mut Terminal<'_, '_>) -> Search<'static> {
    terminal.vt_write(b"fixture");
    let mut search = Search::new(terminal).unwrap();
    search.set_needle(terminal, b"fixture").unwrap();
    search.run(terminal).unwrap();
    assert!(search.select_next(terminal).unwrap().is_some());
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
}

#[test]
fn search_can_be_dropped_before_terminal() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut search = fixture(&mut terminal);

    search
        .select_next(&terminal)
        .unwrap()
        .unwrap()
        .selection()
        .unwrap()
        .start()
        .cell()
        .unwrap();
    drop(search);
    terminal.vt_write(b"more");
}

#[test]
fn replacing_and_freeing_original_rejects_new_terminal() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut search = fixture(&mut terminal);
    let original = std::mem::replace(&mut terminal, Terminal::new(8, 2).unwrap());
    drop(original);
    rejects_wrong_terminal(&mut search, &mut terminal);
    assert!(matches!(search.tick(), Err(Error::InvalidValue)));
    // The original search stays rejected even once another search is bound
    // to the replacement.
    let replacement_search = fixture(&mut terminal);
    rejects_wrong_terminal(&mut search, &mut terminal);
    drop(search);
    drop(replacement_search);
}

#[test]
fn multiple_searches_share_identity_and_detach_on_terminal_drop() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    let mut first = fixture(&mut terminal);
    let mut second = Search::new(&terminal).unwrap();
    second.set_needle(&mut terminal, b"fixture").unwrap();
    second.run(&terminal).unwrap();
    assert!(second.select_next(&terminal).unwrap().is_some());
    first.feed(&terminal).unwrap();
    drop(terminal);
    // Ticking needs the terminal too, even though it doesn't borrow it.
    assert!(matches!(first.tick(), Err(Error::InvalidValue)));
    assert!(matches!(second.tick(), Err(Error::InvalidValue)));
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
        let mut matches = search.matches(&terminal, &mut storage).unwrap();
        assert_eq!(matches.len(), 1);
        matches.next_back().unwrap().start().cell().unwrap();
        assert!(matches.next().is_none());
        assert!(matches.next_back().is_none());
    }
    search.set_needle(&mut terminal, b"").unwrap();
    assert_eq!(search.matches(&terminal, &mut storage).unwrap().len(), 0);
    drop(terminal);
    drop(search);
    // Storage still contains raw values from the old terminal. A new read must
    // overwrite them before handing out references bounded by this new owner.
    let mut terminal = Terminal::new(8, 2).unwrap();
    let search = fixture(&mut terminal);
    let selected = search
        .viewport_matches(&terminal, &mut storage)
        .unwrap()
        .next()
        .unwrap();
    selected.start().cell().unwrap();
}
