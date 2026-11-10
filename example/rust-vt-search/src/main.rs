use libghostty_vt::{
    point::Space,
    search::{MatchBuffer, Search},
    Terminal,
};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // Create a terminal and fill it with some content to search.
    let mut terminal = Terminal::new(80, 24)?;
    terminal.vt_write(b"$ make test\r\n");
    terminal.vt_write(b"compiling module A... ok\r\n");
    terminal.vt_write(b"compiling module B... error: missing semicolon\r\n");
    terminal.vt_write(b"linking... error: undefined symbol\r\n");
    terminal.vt_write(b"$ grep -n ERROR build.log\r\n");

    // The user opened the find bar, so create a search bound to the
    // terminal. It starts idle until it has a needle.
    let mut search = Search::new(&terminal)?;

    // The user typed a query. Matching is byte-exact except ASCII
    // letters, which compare case-insensitively, so "error" also finds
    // "ERROR". Retyping just sets the needle again: a changed needle
    // restarts the search and an unchanged one keeps its results.
    search.set_needle(&mut terminal, b"error")?;

    // Drive the search. Interactive embedders interleave
    // `Search::tick` and `Search::feed` with their event
    // loop, but for a one-shot search we can just run it to completion.
    search.run(&mut terminal)?;

    // The total match count, for find bar text like "1 of 3".
    let total = search.total_matches()?;
    println!(r#"{total} matches for "error""#);

    // The user pressed Enter, so select the next match. Selection starts
    // at the newest match, moves toward older content, and wraps around.
    // This scrolls the viewport to the match if it isn't visible, per
    // the `Search::scroll` policy.
    while let Ok(Some(s)) = search.select_next(&mut terminal) {
        // Read the selection state. Index 0 is the newest
        // match, so a "k of n" find bar renders index + 1.
        let Some(selected_index) = s.selected_index()? else {
            continue;
        };
        println!("selected {} of {}", selected_index + 1, total);

        // Wrapped back around to the first match: stop.
        if selected_index + 1 == total {
            break;
        }
    }

    // Each frame while the find bar is open, feed to catch up with any
    // terminal changes and then read the viewport matches to draw
    // highlights. The list can include matches just past the viewport
    // when they share a page with it, so convert each endpoint to
    // viewport coordinates and skip matches outside the visible rows.
    let snapshot = search.feed(&mut terminal)?;

    let mut match_buffer = MatchBuffer::new();
    for m in snapshot.viewport_matches(&mut match_buffer)? {
        let Ok(Some(start)) = terminal.point_from_grid_ref(m.start(), Space::Viewport) else {
            continue;
        };
        let Ok(Some(end)) = terminal.point_from_grid_ref(m.end(), Space::Viewport) else {
            continue;
        };

        if start.y >= 24 || end.y >= 24 {
            continue;
        }

        // A real embedder draws a highlight rect from start to end here.
        println!(
            "highlight rows {}-{}, cols {}-{}",
            start.y, end.y, start.x, end.x
        );
    }

    Ok(())
}
