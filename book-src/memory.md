# Memory {#memory}

libghostty-vt may allocate memory and fully supports custom allocators.

Most programs never need to think about this. Any function that allocates
either has variants with or without allocators (e.g. `new` vs `new_with_alloc`),
or takes an `Option<alloc::Allocator<'a>>` (`None` selects the default allocator).
Pass your own when you want memory to come from a specific place, such as an
arena, a pool, a tracking allocator, or a platform without a C library.

The library handles allocation failure. When an allocator returns NULL, the
function that needed the memory returns an error such as [`Error::OutOfMemory`].
It doesn't crash.

## The Default Allocator {#memory-default}

The default allocator uses:

- libc `malloc` and `free` when the library is linked against libc.
- A built-in allocator when it isn't. This is currently Zig's SMP allocator, or
  Zig's WebAssembly allocator on WebAssembly.
- An allocator that always fails on native freestanding targets, which have no
  heap. On these targets you must pass your own allocator to any function that
  allocates.

## Writing a Custom Allocator {#memory-custom}

Rust users have two ways to obtain a custom libghostty [`Allocator`](alloc::Allocator):

- [`Allocator::GLOBAL`](alloc::Allocator::GLOBAL), which uses Rust's own
  [global allocator](std::alloc::GlobalAlloc);
- Adapting a custom allocator via the soon-to-be stable (at the time of writing)
  [Allocator API](std::alloc::Allocator). We offer a `From` implementation that
  converts an Allocator API-compliant allocator into a libghostty allocator.

> [!NOTE]
>
> You may notice that libghostty's `Allocator` type comes with a lifetime parameter
> `'ctx`. This is because we might need to take a reference to the Rust-side
> allocator state when converting an existing allocator. If you use the default
> allocator or `Allocator::GLOBAL`, feel free to set `'ctx` to `'static`.

The library may call an allocator's functions from any thread that uses a handle
created with it. If handles that share one allocator are used on different
threads, its functions must be thread-safe. In other words, the allocator must
be `Sync`. The default allocator is.

## Alignment {#memory-alignment}

Every vtable function receives an `alignment` argument. It's an exponent, not a
byte count. The address you return must be a multiple of `1 << alignment` bytes.

| `alignment` | Address must be a multiple of |
| ----------- | ----------------------------- |
| 0           | 1 byte                        |
| 3           | 8 bytes                       |
| 4           | 16 bytes                      |
| 12          | 4096 bytes                    |

> [!WARNING]
> **You must translate the alignment value into bytes** when creating a Rust
> [`std::alloc::Layout`]. It's different from Zig's `std.mem.Alignment` in
> this regard, and failing to do so can render your allocator completely unusable.
> Other APIs like `aligned_alloc` or `posix_memalign` may use different conventions.

## Allocating and Freeing Buffers {#memory-buffers}

[`Bytes`] is the primary type for buffers allocated by or for libghostty.
Use them when:

- You need a buffer to hand to a libghostty-vt function, such as returning a
  [image](kitty::graphics::DecodedImage) decoded by the installed Kitty graphics
  [PNG decoder](kitty::graphics::DecodePng).
- You want libghostty to handle allocation for you while still having ownership
  over the returned data, such as in the case of [`Formatter::format_alloc()`].

Memory must only be freed by the same allocator that created it. On some
platforms the library and your program use different heaps. On Windows, for
example, Zig's libc and MSVC's C runtime each keep their own, so freeing memory
from one with the other is undefined behavior. The Rust bindings make sure
this will never happen normally through our `Drop` implementations.

## Allocators Passed to Callbacks {#memory-callbacks}

Some callbacks receive an allocator from the library, such as the PNG decoder
set with [`kitty::graphics::set_png_decoder`]. Use that allocator only through
allocating functions, such as [`Bytes::new_zeroed_with_alloc()`].
Its inner workings are implementation details that may silently change in future
releases.
