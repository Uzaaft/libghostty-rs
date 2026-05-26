# grid_ref_tracked_rs

Rust recreation of Ghostty's `c-vt-grid-ref-tracked` example.

This demonstrates the safe `libghostty-vt` tracked grid reference API: create a
tracked reference to a cell, let terminal mutations move that cell into
scrollback, detect when a reset discards the location, and reuse the same handle
for a new point.

## Usage

```sh
cargo run -p grid_ref_tracked_rs
```

When building with `link-dynamic`, set `DYLD_LIBRARY_PATH` on macOS or
`LD_LIBRARY_PATH` on Linux to the directory containing the generated
`libghostty-vt` shared library.
