# Getting and Setting Values {#values}

libghostty-vt has hundreds of values you may want to read or change.
Unlike the C API, however, the Rust API provides separate methods for getting
or setting each value, since there are no concerns about ABI compatibility,
and type safety is much more significant instead.

## Newer Headers, Older Libraries {#values-older-libraries}

We add new configurable values over time. A program built with a newer header
can end up running against an older copy of the library, for example when the
library is installed separately as a shared library.

If you pass a key that the loaded library doesn't know, the call returns
[`Error::Unsupported`] and doesn't touch your output. This works the same way in
debug and release builds of the library.

That lets you use a newer key when it's there and fall back when it isn't. In
this example, imagine that [`render::Snapshot::cursor_blinking`] was added
in a later release than the library your program loads:

```rs
let blinking = match snapshot.cursor_blinking() {
  Ok(v) => v,
  // This library predates the key. Use a sensible default instead.
  Err(Error::Unsupported) => true,
  // A real error. Handle it as usual.
  Err(err) => return err,
};
```

Note that for functions that return `Result<Option<T>>`, the `Ok(None)` case
and the `Err(Error::Unsupported)` case mean different things. [`Error::Unsupported`]
means the library doesn't know the value at all. `None` means the library
knows the value, but something you passed with it is wrong.

The same check applies to enum values you pass as plain arguments or inside
option values, such as a [`key::Action`] or the variant of a [`terminal::Point`].
A value the library doesn't recognize returns [`Error::InvalidValue`] and leaves
the object unchanged.
