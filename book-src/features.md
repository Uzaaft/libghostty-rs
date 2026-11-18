# Optional Features {#features}

> [!WARNING]
>
> This is not yet implemented for the Rust bindings. We are hard at work
> implementing it and it will be available in the near future.

Some parts of libghostty-vt are optional. A program that only draws a terminal
doesn't need search or snapshots, for example, so the library can be built
without them to make it smaller. You choose features when you build the library
with default features disabled and with vendoring enabled. We translate the
Cargo features you selected into Zig build flags when vendoring the library.

Rust binding symbols are gated behind Cargo feature flags, as is usual for Rust
libraries. If at runtime however the linked libghostty-vt does not support a
given feature, then the function does nothing and returns [`Error::Unsupported`].

| Feature               | Types, functions, traits                                                                                                                                                               |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `color`              | All types and functions within [`color`]                                                                                                                                                       |
| `formatter`          | All types and functions within [`fmt`]                                                                                                                                                   |
| `glyph_protocol`     | None. It lets applications register custom glyphs with Ghostty's glyph protocol.                                                                                        |
| `grid_introspection` | All types and functions within [`grid_ref`], as well as [`screen::Cell`], [`screen::Row`], [`Terminal::track_grid_ref`]                                           |
| `input_encode`       | All types and functions within [`key`], [`mouse`], [`focus`] and [`paste`], as well as [`Terminal::paste`] |
| `kitty_graphics`     | All types and functions within [`kitty::graphics`], and the Kitty setters and getters on [`Terminal`]                                                         |
| `render_state`       | All types and functions within [`render`]                                                                                                                                                |
| `search`             | All types and functions within [`search`]                                                                                                                                                      |
| `selection`          | All types and functions within [`selection`], and all `select_*` methods on [`Terminal`]                                                                             |
| `snapshot`           | All types and functions within [`snapshot`]                                                                                                                                                    |

A few functions depend on two features:

- [`Terminal::grid_ref()`] and [`Terminal::point_from_grid_ref()`] need
  either `grid-introspection` or `selection`.
- [`TrackedGridRef::snapshot()`] needs both `grid-introspection` and
  `snapshot`.

Kitty graphics need a clock, so builds for targets without an operating system,
such as wasm32-freestanding, never include `kitty-graphics`.

## Checking for a Feature {#features-check}

To find out up front, for example to hide a menu item, call the corresponding
`supports_*` function in [`build_info`].

```rust,ignore
let has_search = build_info::supports_search().unwrap_or(false);
if !has_search {
    // Hide the find bar.
}
```

You can also just call the function and check its result.

```rust,ignore
let search = match Search::new(&terminal) {
    Ok(search) => search,
    Err(Error::Unsupported) => {
      // This library was built without search.
    }
    Err(e) => { /* Some other issue, such as allocation failure */ }
};
```

## Functions Without a Result {#features-no-result}

A function that doesn't return a `Result` can't report [`Error::Unsupported`].
When its feature is left out, it returns a fixed value instead:

- Functions that return `bool`, such as iterator `_next` methods, return
  false.
- Functions that return a number, an enum or a struct return zero or a zeroed
  struct. [`key::Event::utf8`] returns an empty string.
- [`color::Palette::default()`] and [`color::Palette::generate()`] fill their
  outputs with zeros.
- [`color::X11ColorNames`] is empty.
- [`kitty::Graphics::image()`] returns NULL.

In practice these values rarely matter. Most of these functions take a handle
that could only come from a `_new` function of the same feature, and that
function returned [`Error::Unsupported`].
