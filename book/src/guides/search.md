# Search {#guide-search}

A [`Search`] finds a string in a terminal. It covers the active area and
scrollback of both the primary and alternate screens.

Terminal search is harder than searching a string, because the text won't hold
still. Scrollback can be huge, and a program can keep writing while you search.
We handle that part. Results stay in sync with the live screens. They survive
primary and alternate screen switches, so opening and closing a fullscreen app
such as vim doesn't restart a scrollback search. And they recover from resize,
reflow, resets and scrollback pruning.

What's left to you is deciding when the work happens. If you want a one-shot
search, [`Search::run()`] does all of it, and you can go straight to the
example. If you're building a find bar for a terminal with a lot of scrollback,
read the first section.

## Driving a search {#guide-search-driving}

A search starts idle. Setting a needle with [`Search::set_needle()`] starts
it, changing the needle restarts it from scratch, and clearing it returns the
search to idle. Setting the needle it already has keeps the results, so a find
bar can resubmit freely.

Matching is byte-exact, except for ASCII letters, which compare
case-insensitively.

Searching a large scrollback takes time, so the work is split into small steps
you drive yourself. That gives you direct control over the performance:

| Function              | What it does                                                                                                      | Needs the terminal |
| --------------------- | ----------------------------------------------------------------------------------------------------------------- | ------------------ |
| [`Search::tick()`] | Makes a bounded amount of progress on data the search has already copied.                                         | No                 |
| [`Search::feed()`] | Reads the terminal to copy in more data and pick up changes, and returns a snapshot of the matched items.                                                      | Yes                |
| [`Search::run()`]  | Blocks, feeding and ticking until the search is caught up. Interactive programs should use the other two instead. | Yes                |

Tick while the status is [`search::Status::Running`], and feed when it
becomes [`search::Status::FeedRequired`]. That's also the status right after
a needle is set, because the search hasn't seen the terminal yet.

Feeding is the only way a search learns that the terminal changed. So keep
feeding periodically while the search is in use, even after it reports complete.

[`search::Status::Complete`] means the search is caught up with the terminal
as of the last feed. It never means finished forever, because later writes need
another feed to be seen.

## Matches are selections {#guide-search-matches}

Every match is a [`Selection`] snapshot with `rectangle` set to false. So
everything in [Selection](selection.md#guide-selection) works on matches:

- To copy the matched text, use a [`Formatter`] with
  [`Formatter::set_selection()`].
- To position highlight rectangles, use [`Terminal::point_from_grid_ref()`]
  with [`point::Space::Viewport`].
- To hit test, use [`Selection::contains()`].
- To make a match the terminal's selection, use [`Terminal::set_selection()`]. 

Matches follow the usual snapshot rule. They're only valid until the next
operation that changes the terminal, including [`Terminal::vt_write()`],
resize, reset and free. Read matches after a feed, use them before the terminal
changes again, and read them again rather than caching them.

We keep the selected match accurate across terminal changes. So the safe way to
follow a match is to read [`search::Snapshot::selected_match()`] again after
each feed.

## Lifetime {#guide-search-lifetime}

A search borrows the terminal it was created with and never owns it. Any number
of searches can share one terminal, alongside other readers such as formatters
and render states.

You can drop the search and its terminal in either order. Dropping the search
first releases the tracked state it holds in the terminal. If the terminal goes
first, the search detects that:

- Calls that need the terminal, and [`Search::tick()`], return
  [`Error::InvalidValue`].
- Reads return whatever the search last saw.
- The `Drop` implementation releases only the search's own memory.

A search can't be rebound. To search another terminal, create a new search.

## Threads {#guide-search-threads}

The library creates no threads. Calls on one [`Search`] aren't safe to make
concurrently with each other, so you have to serialize them.

The functions that touch the terminal also have to be serialized with all other
access to that terminal:

- [`Search::new()`] and the `Drop` implementation
- [`Search::feed()`] and [`Search::run()`]
- [`Search::select_next()`] and [`Search::select_prev()`]
- [`Search::set_needle()`]

Everything else, meaning [`Search::tick()`] and its various getters, only
touches memory the search owns. Those are safe to call while another thread
changes the terminal.

That split is how Ghostty runs search on a background thread: tick freely, and
take the terminal lock only to feed.

Reading a returned match value is always safe. Passing one to a function that
takes the terminal follows the terminal's serialization rule above.

## Example {#guide-search-example}

This creates a search, runs it, steps through the matches like a find bar, and
reads the viewport matches to draw highlights.

```rs
{{#include ../../../example/rust-vt-search/src/main.rs}}
```

Full program: [rust-vt-search/src/main.rs](/example/rust-vt-search/src/main.rs)
