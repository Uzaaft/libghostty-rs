//! Read the images that programs send with the Kitty graphics protocol.
//!
//! [`Graphics`] is a handle to the image storage of the active screen,
//! where the terminal keeps every image a program sends with the
//! [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).
//!
//! Read it from a terminal with [`Terminal::kitty_graphics`]. Use it to read
//! pixels, play animations, and inspect what the storage holds. It doesn't tell
//! you where to draw. The render state does that.
//!
//! See [Images] for how to set up, decode PNGs, and draw images.
//!
#![doc = crate::guide_ref!("Images", "images")]
#![cfg(feature = "kitty-graphics")]

use std::{ffi::OsStr, mem::MaybeUninit, path::Path};

use crate::{
    Terminal,
    alloc::{Allocator, Object, Ref},
    error::{Error, Result, from_optional_result, from_optional_result_uninit, from_result},
    ffi,
    selection::Selection,
};

#[doc(inline)]
pub use ffi::KittyGraphicsPlacementRenderInfo as PlacementRenderInfo;

/// Opaque reference to a Kitty graphics image storage.
///
/// Obtained via [`Terminal::kitty_graphics`]. The reference is borrowed from
/// the terminal with lifetime `'t` and remains valid until the next mutating
/// terminal call (e.g. [`Terminal::vt_write`] or [`Terminal::reset`]).
#[derive(Debug)]
pub struct Graphics<'t> {
    inner: Ref<'t, ffi::KittyGraphicsImpl>,
    /// The `Terminal::id` of the terminal this storage belongs to.
    ///
    /// The lifetime only says that some terminal is borrowed, not which one,
    /// so [`PlacementIteration::rect`] compares this with the terminal it's
    /// given to make sure they're the same one.
    terminal: u64,
}

/// Opaque reference to a Kitty graphics image.
///
/// Obtained via [`Graphics::image`] with an image ID. The reference is
/// borrowed from the storage with lifetime `'g` and remains valid until
/// the next mutating terminal or graphics storage call
/// (e.g. [`Graphics::tick_animation`]).
#[derive(Debug)]
pub struct Image<'g> {
    inner: Ref<'g, ffi::KittyGraphicsImageImpl>,
}

/// Opaque reference to a Kitty graphics placement iterator.
#[derive(Debug)]
pub struct PlacementIterator<'alloc> {
    inner: Object<'alloc, ffi::KittyGraphicsPlacementIteratorImpl>,
}

/// Obtained via [`PlacementIterator::update`]. The reference is
/// borrowed from the storage with lifetime `'t` and remains valid until
/// the next mutating terminal call.
#[derive(Debug)]
pub struct PlacementIteration<'t, 'alloc> {
    iterator: &'t mut PlacementIterator<'alloc>,
    /// The `Terminal::id` of the terminal whose storage is being iterated,
    /// copied from [`Graphics`]. See [`PlacementIteration::rect`].
    terminal: u64,
}

/// Methods related to the [Kitty graphics protocol](crate::kitty::graphics).
impl Terminal<'_, '_> {
    /// The Kitty graphics image storage for the active screen.
    ///
    /// Returns a borrowed reference to the image storage.
    /// The pointer is valid until the next mutating terminal call (e.g.
    /// [`Terminal::vt_write`] or [`Terminal::reset`]).
    ///
    /// Returns [`Error::Unsupported`] when Kitty graphics are disabled at build time.
    pub fn kitty_graphics(&self) -> Result<Graphics<'_>> {
        let inner = self.get::<ffi::KittyGraphics>(ffi::TerminalData::KITTY_GRAPHICS)?;

        Ok(Graphics {
            inner: Ref::new(inner)?,
            terminal: self.id,
        })
    }

    /// The Kitty image storage limit in bytes for the active screen.
    ///
    /// A value of zero means the Kitty graphics protocol is disabled.
    ///
    /// Returns [`Error::Unsupported`] when Kitty graphics are disabled at build time.
    pub fn kitty_image_storage_limit(&self) -> Result<u64> {
        self.get(ffi::TerminalData::KITTY_IMAGE_STORAGE_LIMIT)
    }

    /// Whether the file medium is enabled for Kitty image loading on the
    /// active screen.
    ///
    /// Returns [`Error::Unsupported`] when Kitty graphics are disabled at build time.
    pub fn is_kitty_image_from_file_allowed(&self) -> Result<bool> {
        self.get(ffi::TerminalData::KITTY_IMAGE_MEDIUM_FILE)
    }

    /// The directory allowed for Kitty image loading via the temporary file
    /// medium on the active screen, or `None` when the medium is disabled.
    ///
    /// Returns [`Error::Unsupported`] when Kitty graphics are disabled at build time.
    ///
    /// Returns a borrowed path, valid until the next mutating terminal call
    /// (e.g. [`Terminal::vt_write`] or [`Terminal::reset`]).
    pub fn kitty_image_temp_file_dir(&self) -> Result<Option<&Path>> {
        let str = self.get::<ffi::String>(ffi::TerminalData::KITTY_IMAGE_MEDIUM_TEMP_FILE)?;

        // If not disabled at build time yet the string is empty,
        // then the medium itself is disabled. Return None in this case.
        if str.len == 0 {
            return Ok(None);
        }

        // SAFETY: We trust libghostty to return a valid borrowed string,
        // while we uphold that no mutation could happen during its lifetime.
        let bytes = unsafe { std::slice::from_raw_parts(str.ptr, str.len) };

        // SAFETY:
        // Whilst the `OsStr::from_encoded_bytes_unchecked` function states that the
        // exact OsStr encoding format is an implementation detail and no external bytes
        // should rely on which exact encoding is used... In reality this should always
        // work without issue, since both Zig and Rust basically share the same underlying
        // string format.
        //
        // On Unix-like platforms, both Zig and Rust agree that a file path is just an
        // arbitrary sequence of bytes without any NULs in between. The tricky part is
        // that Windows uses (noncompliant) UTF-16 file paths natively and it just so
        // happens both Zig and Rust use WTF-8 as a workaround against that whole can
        // of worms, though in Rust it's more of an implementation detail.
        // The Kitty graphics protocol itself is poorly defined on Windows, but for all
        // intents and purposes, the bytes are always valid `OsStr`s and `Path`s.
        let os_str = unsafe { OsStr::from_encoded_bytes_unchecked(bytes) };
        Ok(Some(Path::new(os_str)))
    }

    /// Whether the shared memory medium is enabled for Kitty image loading
    /// on the active screen.
    ///
    /// Returns [`Error::Unsupported`] when Kitty graphics are disabled at build time.
    pub fn is_kitty_image_from_shared_mem_allowed(&self) -> Result<bool> {
        self.get(ffi::TerminalData::KITTY_IMAGE_MEDIUM_SHARED_MEM)
    }
    /// Set the Kitty image storage limit in bytes.
    ///
    /// Applied to all initialized screens (primary and alternate).
    /// A value of zero disables the Kitty graphics protocol entirely,
    /// deleting all stored images and placements. The default is 10 MB
    /// (10,000,000 bytes), and a `None` value restores it. Values too large
    /// for the platform's address space are treated as the largest possible
    /// limit.
    ///
    /// Returns [`Error::Unsupported`] when Kitty graphics are disabled at build time.
    pub fn set_kitty_image_storage_limit(&mut self, limit: Option<u64>) -> Result<&mut Self> {
        self.set_optional(
            ffi::TerminalOption::KITTY_IMAGE_STORAGE_LIMIT,
            limit.as_ref(),
        )?;
        Ok(self)
    }
    /// Enable or disable Kitty image loading via the file medium.
    ///
    /// Has no effect when Kitty graphics are disabled at build time.
    pub fn set_kitty_image_from_file_allowed(&mut self, allowed: bool) -> Result<&mut Self> {
        self.set(ffi::TerminalOption::KITTY_IMAGE_MEDIUM_FILE, &allowed)?;
        Ok(self)
    }
    /// Enable Kitty image loading via the temporary file medium, restricted
    /// to the given directory. A `None` value disables the medium. The
    /// string is copied into the terminal.
    ///
    /// Has no effect when Kitty graphics are disabled at build time.
    pub fn set_kitty_image_temp_file_dir(&mut self, dir: Option<&Path>) -> Result<&mut Self> {
        let dir = dir.map(|dir| ffi::String::from(dir.as_os_str().as_encoded_bytes()));

        self.set_optional(
            ffi::TerminalOption::KITTY_IMAGE_MEDIUM_TEMP_FILE,
            dir.as_ref(),
        )?;
        Ok(self)
    }
    /// Enable or disable Kitty image loading via the shared memory medium.
    ///
    /// Has no effect when Kitty graphics are disabled at build time.
    pub fn set_kitty_image_from_shared_mem_allowed(&mut self, allowed: bool) -> Result<&mut Self> {
        self.set(ffi::TerminalOption::KITTY_IMAGE_MEDIUM_SHARED_MEM, &allowed)?;
        Ok(self)
    }

    /// Set the maximum bytes the APC handler will buffer for Kitty graphics
    /// protocol data.
    ///
    /// This prevents malicious input from causing unbounded memory allocation.
    /// A `None` value removes all overrides, reverting to the built-in defaults.
    pub fn set_apc_max_bytes_kitty(&mut self, max: Option<usize>) -> Result<&mut Self> {
        self.set_optional(ffi::TerminalOption::APC_MAX_BYTES_KITTY, max.as_ref())?;
        Ok(self)
    }
}

impl Graphics<'_> {
    fn get<T>(&self, tag: ffi::KittyGraphicsData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_result(result)?;
        Ok(unsafe { value.assume_init() })
    }

    /// Look up a Kitty graphics image by its image ID.
    ///
    /// Returns `None` if no image with the given ID exists.
    #[must_use]
    pub fn image(&self, id: u32) -> Option<Image<'_>> {
        let image = unsafe { ffi::ghostty_kitty_graphics_image(self.inner.as_raw(), id) };

        Some(Image {
            inner: Ref::new(image.cast_mut()).ok()?,
        })
    }

    /// A value that changes whenever a program sends, replaces, places or
    /// deletes an image.
    ///
    /// It is 0 if nothing has changed the storage yet, which means the storage
    /// is empty.
    ///
    /// If the value matches the last one you read, the storage holds the same
    /// images and placements as before. The placements may still have moved on
    /// screen, because scrolling and resizing move them without changing the
    /// storage. So this value can't tell you when to read the render state's
    /// image list again. For that, use
    /// [`Snapshot::image_placements_generation`](crate::render::Snapshot::image_placements_generation).
    /// See [Knowing what changed].
    #[doc = crate::guide_ref!("Knowing what changed", "images", "changes")]
    ///
    /// A value is never reused, not even by another screen, after a reset, or
    /// by another terminal. You can use it as a cache key on its own.
    pub fn generation(&self) -> Result<u64> {
        self.get(ffi::KittyGraphicsData::GENERATION)
    }

    /// Advance playing animations and get the time of the next frame.
    ///
    /// Every playing animation on the active screen moves to the frame that is
    /// due at `now_ms`.
    /// An animation moves forward at most one frame per call. If calls arrive
    /// late, playback slows down rather than skipping frames.
    ///
    /// Images that moved to a new frame get a new
    /// [generation](Image::generation), and their [`Image::data`] returns the
    /// new frame's pixels.
    ///
    /// Time is measured in milliseconds on a clock you choose, such as the
    /// time since your application started. The clock must only move forward,
    /// and you must keep using the same clock with the same starting point for
    /// as long as the terminal exists. The returned time uses the same clock.
    ///
    /// This call changes the image storage. Like [`Terminal::vt_write`], it
    /// invalidates any [`Image`] handles and image data you are holding, so
    /// look images up again afterwards.
    ///
    /// Call this once each time you prepare a frame to draw, not in a loop.
    /// The [Animation] section explains how to schedule redraws with the
    /// returned time and includes an example.
    ///
    /// Returns `Some` with the time the next frame is due, in milliseconds on
    /// the same clock as `now_ms`, if an animation is playing. Returns `None`
    /// if nothing is playing or Kitty graphics are disabled at build time.
    ///
    #[doc = crate::guide_ref!("Animation", "images", "animation")]
    pub fn tick_animation(&mut self, now_ms: u64) -> Result<Option<u64>> {
        let mut next_ms = MaybeUninit::<u64>::zeroed();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_animation_tick(
                self.inner.as_raw(),
                now_ms,
                next_ms.as_mut_ptr(),
            )
        };
        from_optional_result_uninit(result, next_ms)
    }
}

impl<'g> Image<'g> {
    fn get<T>(&self, tag: ffi::KittyGraphicsImageData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_image_get(
                self.inner.as_raw(),
                tag,
                value.as_mut_ptr().cast(),
            )
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }
    /// Like [`Image::get`], but for keys upstream documents as able to
    /// return `NO_VALUE` for a valid image. That code means "no value right
    /// now" rather than a failure, so it becomes `Ok(None)`. Every other
    /// error, such as `INVALID_VALUE` for a bad handle, stays an `Err`.
    fn get_optional<T>(&self, tag: ffi::KittyGraphicsImageData::Type) -> Result<Option<T>> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_image_get(
                self.inner.as_raw(),
                tag,
                value.as_mut_ptr().cast(),
            )
        };
        from_optional_result_uninit(result, value)
    }

    /// The image ID.
    pub fn id(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsImageData::ID)
    }
    /// The image number.
    pub fn number(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsImageData::NUMBER)
    }
    /// Image width in pixels.
    pub fn width(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsImageData::WIDTH)
    }
    /// Image height in pixels.
    pub fn height(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsImageData::HEIGHT)
    }

    /// A value that changes whenever this image's pixels may have changed.
    ///
    /// When a program sends the image or replaces it, and when an animation
    /// moves to a new frame. Use it as part of your texture cache key. If it
    /// matches the texture you have, you don't need to read the pixels again.
    /// The render state reports the same value for each of its entries as
    /// [`ImageIteration::image_generation`](crate::render::ImageIteration::image_generation).
    ///
    /// Don't use the image's size or data length to detect changes instead.
    /// A program can send new pixels of the same size under the same ID.
    ///
    /// A value is never reused, not even by another image, another screen, or
    /// another terminal. It is never 0 for a stored image, so you can use 0
    /// to mean "no texture yet".
    ///
    /// One case doesn't change this value. When a program finishes sending an
    /// image whose pixels were still on the way, the image keeps its value,
    /// and [`Image::data`] starts returning the pixels. [`Image::generation`]
    /// changes when that happens, so read [`Image::data`] again then.
    pub fn generation(&self) -> Result<u64> {
        self.get(ffi::KittyGraphicsImageData::GENERATION)
    }

    /// Pixel format of the image.
    pub fn format(&self) -> Result<ImageFormat> {
        self.get::<ffi::KittyImageFormat::Type>(ffi::KittyGraphicsImageData::FORMAT)
            .and_then(|v| v.try_into().map_err(|_| Error::Unsupported))
    }
    /// Compression of the image.
    pub fn compression(&self) -> Result<Compression> {
        self.get::<ffi::KittyImageCompression::Type>(ffi::KittyGraphicsImageData::COMPRESSION)
            .and_then(|v| v.try_into().map_err(|_| Error::Unsupported))
    }

    /// The image's pixel data.
    ///
    /// Valid as long as the underlying terminal and graphics storage
    /// are not mutated.
    ///
    /// Returns `None` if the image's pixels have not arrived yet. The expected
    /// size is still available through [`Image::expected_len`].
    ///
    /// The data is always fully decoded, uncompressed pixels in the format
    /// reported by [`Image::format`]: zlib payloads are inflated and PNG
    /// payloads are decoded to RGBA at transmission time, before the image
    /// is stored. Consumers can upload this directly to the GPU without any
    /// decode step.
    ///
    /// For an animated image (Kitty graphics animation, actions a=f/a=a) this
    /// is the pixel data of the current animation frame. The image's
    /// [generation](Image::generation) changes whenever the current frame
    /// changes, so generation-keyed caches remain coherent.
    pub fn data(&self) -> Result<Option<&'g [u8]>> {
        self.get_optional::<ffi::String>(ffi::KittyGraphicsImageData::PIXELS)
            .map(|pixels| {
                pixels.map(|pixels| {
                    // SAFETY: The pixels remain valid until the terminal is
                    // mutated, and `'t` borrows the terminal immutably.
                    unsafe { pixels.to_bytes() }
                })
            })
    }

    /// The size of the image's pixel data in bytes, which is
    /// `width * height * bytes per pixel` for the reported format.
    ///
    /// This is available even before the pixels arrive, when [`Image::data`]
    /// still returns `None`. In that case the image already counts this many
    /// bytes against the storage limit. Once the pixels arrive, this equals
    /// the length of [`Image::data`].
    pub fn expected_len(&self) -> Result<usize> {
        self.get(ffi::KittyGraphicsImageData::EXPECTED_LEN)
    }
}

impl<'alloc> PlacementIterator<'alloc> {
    /// Create a new placement iterator instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }
    /// Create a new placement iterator instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_in<'ctx: 'alloc>(alloc: &'alloc Allocator<'ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }
    unsafe fn new_inner(alloc: *const ffi::Allocator) -> Result<Self> {
        let mut inner: ffi::KittyGraphicsPlacementIterator = std::ptr::null_mut();
        let result =
            unsafe { ffi::ghostty_kitty_graphics_placement_iterator_new(alloc, &raw mut inner) };
        from_result(result)?;
        Ok(Self {
            inner: Object::new(inner)?,
        })
    }

    /// Update the placement iterator with the given graphics storage,
    /// returning a new placement iteration.
    ///
    /// The iteration walks the storage itself, so it borrows `graphics` (and
    /// through it the terminal) as well as this iterator:
    ///
    /// ```compile_fail,E0505
    /// # use libghostty_vt::{Terminal, kitty::graphics::PlacementIterator};
    /// let terminal = Terminal::new(80, 24).unwrap();
    /// let mut iterator = PlacementIterator::new().unwrap();
    /// let graphics = terminal.kitty_graphics().unwrap().unwrap();
    /// let mut placements = iterator.update(&graphics).unwrap();
    /// drop(terminal);
    /// placements.next();
    /// ```
    pub fn update<'t>(
        &'t mut self,
        graphics: &'t Graphics<'_>,
    ) -> Result<PlacementIteration<'t, 'alloc>> {
        let result = unsafe {
            ffi::ghostty_kitty_graphics_get(
                graphics.inner.as_raw(),
                ffi::KittyGraphicsData::PLACEMENT_ITERATOR,
                (&raw mut self.inner).cast(),
            )
        };
        from_result(result)?;
        Ok(PlacementIteration {
            iterator: self,
            terminal: graphics.terminal,
        })
    }
}

impl Drop for PlacementIterator<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::ghostty_kitty_graphics_placement_iterator_free(self.inner.as_raw());
        }
    }
}

impl<'t> PlacementIteration<'t, '_> {
    /// Advance the placement iterator to the next placement.
    ///
    /// If a layer filter has been set via [`PlacementIteration::set_layer`],
    /// only placements matching that layer are returned.
    #[expect(
        clippy::should_implement_trait,
        reason = "a lending iterator: each placement borrows the iteration"
    )]
    pub fn next(&mut self) -> Option<&Self> {
        if unsafe { ffi::ghostty_kitty_graphics_placement_next(self.iterator.inner.as_raw()) } {
            Some(self)
        } else {
            None
        }
    }

    fn set<T>(
        &self,
        tag: ffi::KittyGraphicsPlacementIteratorOption::Type,
        value: &T,
    ) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_iterator_set(
                self.iterator.inner.as_raw(),
                tag,
                std::ptr::from_ref(value).cast(),
            )
        };
        from_result(result)
    }
    fn get<T>(&self, tag: ffi::KittyGraphicsPlacementData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_get(
                self.iterator.inner.as_raw(),
                tag,
                value.as_mut_ptr().cast(),
            )
        };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// Only return placements in the selected layers. See [`Layer`].
    ///
    /// An empty mask disables the filter and visits every placement.
    /// Unknown bits return [`Error::InvalidValue`] and keep the current filter.
    pub fn set_layer(&mut self, layer: Layer) -> Result<&mut Self> {
        self.set::<u32>(
            ffi::KittyGraphicsPlacementIteratorOption::LAYER,
            &layer.bits(),
        )?;
        Ok(self)
    }

    /// Compute the rendered pixel size of the current placement.
    ///
    /// Takes into account the placement's source rectangle, specified
    /// columns/rows, and aspect ratio to calculate the final rendered pixel
    /// dimensions.
    pub fn pixel_size(
        &self,
        image: &Image<'t>,
        terminal: &'t Terminal<'_, '_>,
    ) -> Result<PixelSize> {
        let mut size = PixelSize::default();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_pixel_size(
                self.iterator.inner.as_raw(),
                image.inner.as_raw(),
                terminal.inner.as_raw(),
                &raw mut size.width,
                &raw mut size.height,
            )
        };
        from_result(result)?;
        Ok(size)
    }

    /// Compute the rendered pixel size of the current placement.
    ///
    /// Takes into account the placement's source rectangle, specified
    /// columns/rows, and aspect ratio to calculate the final rendered pixel
    /// dimensions.
    pub fn grid_size(&self, image: &Image<'t>, terminal: &'t Terminal<'_, '_>) -> Result<GridSize> {
        let mut size = GridSize::default();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_grid_size(
                self.iterator.inner.as_raw(),
                image.inner.as_raw(),
                terminal.inner.as_raw(),
                &raw mut size.cols,
                &raw mut size.rows,
            )
        };
        from_result(result)?;
        Ok(size)
    }

    /// Get the top-left cell of the current placement, relative to the
    /// viewport.
    ///
    /// This is for inspecting the storage. To draw, use the render state's
    /// image list ([`ImageIterator`](crate::render::ImageIterator)) instead.
    /// It positions every image on screen, including virtual placements and
    /// placements relative to them, and covers overscan rows. If you draw
    /// from both, images are drawn twice. See [Drawing images].
    #[doc = crate::guide_ref!("Drawing images", "images", "drawing")]
    ///
    /// The row is negative when the top of the placement scrolled above the
    /// viewport. For example, an image 4 rows tall that scrolled up by 2 rows
    /// has a row of -2. Its top 2 rows are above the viewport and its bottom
    /// 2 rows are still visible.
    ///
    /// Returns `None` if it is entirely outside the viewport, if it is a
    /// virtual placement or relative to one, or if Kitty graphics are
    /// disabled at build time.
    pub fn viewport_pos(
        &self,
        image: &Image<'t>,
        terminal: &'t Terminal<'_, '_>,
    ) -> Result<Option<ViewportPos>> {
        let mut pos = ViewportPos::default();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_viewport_pos(
                self.iterator.inner.as_raw(),
                image.inner.as_raw(),
                terminal.inner.as_raw(),
                &raw mut pos.col,
                &raw mut pos.row,
            )
        };
        from_optional_result_uninit(result, MaybeUninit::new(pos))
    }

    /// Get the resolved source rectangle for the current placement.
    ///
    /// Applies kitty protocol semantics: a width or height of 0 in the
    /// placement means "use the full image dimension", and the resulting
    /// rectangle is clamped to the actual image bounds. The returned values
    /// are in pixels and are ready to use for texture sampling.
    pub fn source_rect(&self, image: &Image<'t>) -> Result<SourceRect> {
        let mut rect = SourceRect::default();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_source_rect(
                self.iterator.inner.as_raw(),
                image.inner.as_raw(),
                &raw mut rect.x,
                &raw mut rect.y,
                &raw mut rect.width,
                &raw mut rect.height,
            )
        };
        from_result(result)?;
        Ok(rect)
    }

    /// Compute the grid rectangle occupied by the current placement.
    ///
    /// Uses the placement's pin, the image dimensions, and the terminal's
    /// cell/pixel geometry to calculate the bounding rectangle. Virtual
    /// placements (unicode placeholders) return `None`.
    ///
    /// Returns [`Error::InvalidValue`] if `terminal` isn't the terminal whose
    /// [`Graphics`] this placement came from.
    pub fn rect(
        &self,
        image: &Image<'t>,
        terminal: &'t Terminal<'_, '_>,
    ) -> Result<Option<Selection<'t>>> {
        // BUG: Upstream doesn't check if the given terminal is the same
        // terminal as the one that created the graphics storage...
        if self.terminal != terminal.id {
            return Err(Error::InvalidValue);
        }
        let mut sel = ffi::Selection::default();
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_rect(
                self.iterator.inner.as_raw(),
                image.inner.as_raw(),
                terminal.inner.as_raw(),
                &raw mut sel,
            )
        };
        // Upstream reports "no rectangle" as `NO_VALUE` rather than an
        // error, since a virtual placement has no position of its own:
        // only the placeholder cells know where it is.
        let sel = from_optional_result(result, sel)?;

        // SAFETY: Selection belongs to the correct terminal and should
        // share its lifetime with the given terminal.
        Ok(sel.map(|sel| unsafe { Selection::from_raw(terminal.inner.as_raw(), sel) }))
    }

    /// Get all of the geometry of the current placement in one call.
    ///
    /// This returns a [`PlacementRenderInfo`] with the pixel size, grid size,
    /// viewport position and source rectangle.
    ///
    /// This is for inspecting the storage. To draw, use the render state's
    /// image list ([`ImageIterator`](crate::render::ImageIterator)) instead.
    /// It gives the same values for every image on screen, including virtual
    /// placements and placements relative to them. If you draw from both,
    /// images are drawn twice. See [Drawing images].
    #[doc = crate::guide_ref!("Drawing images", "images", "drawing")]
    // FIXME: Use our own RenderInfo type instead or re-exporting
    pub fn render_info(
        &self,
        image: &Image<'t>,
        terminal: &'t Terminal<'_, '_>,
    ) -> Result<PlacementRenderInfo> {
        let mut info = ffi::sized!(PlacementRenderInfo);
        let result = unsafe {
            ffi::ghostty_kitty_graphics_placement_render_info(
                self.iterator.inner.as_raw(),
                image.inner.as_raw(),
                terminal.inner.as_raw(),
                &raw mut info,
            )
        };
        from_result(result)?;
        Ok(info)
    }

    /// The image ID this placement belongs to.
    pub fn image_id(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::IMAGE_ID)
    }
    /// The image ID this placement belongs to.
    pub fn placement_id(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::PLACEMENT_ID)
    }
    /// Whether this is a virtual placement, which a program shows with
    /// Unicode placeholder cells.
    ///
    /// A virtual placement appears wherever its placeholder cells are, so the
    /// storage has no position for it. The render state's image list
    /// positions it from its cells like any other placement, so a renderer
    /// doesn't need to check this. See [Placeholders].
    ///
    #[doc = crate::guide_ref!("Placeholders", "images", "placeholders")]
    pub fn is_virtual(&self) -> Result<bool> {
        self.get(ffi::KittyGraphicsPlacementData::IS_VIRTUAL)
    }
    /// Pixel offset from the left edge of the cell.
    pub fn x_offset(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::X_OFFSET)
    }
    /// Pixel offset from the top edge of the cell.
    pub fn y_offset(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::Y_OFFSET)
    }
    /// Source rectangle x origin in pixels.
    pub fn source_x(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::SOURCE_X)
    }
    /// Source rectangle y origin in pixels.
    pub fn source_y(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::SOURCE_Y)
    }
    /// Source rectangle width in pixels (0 = full image width).
    pub fn source_width(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::SOURCE_WIDTH)
    }
    /// Source rectangle height in pixels (0 = full image height).
    pub fn source_height(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::SOURCE_HEIGHT)
    }
    /// Number of columns this placement occupies.
    pub fn columns(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::COLUMNS)
    }
    /// Number of rows this placement occupies.
    pub fn rows(&self) -> Result<u32> {
        self.get(ffi::KittyGraphicsPlacementData::ROWS)
    }
    /// Z-index for this placement.
    pub fn z(&self) -> Result<i32> {
        self.get(ffi::KittyGraphicsPlacementData::Z)
    }
}

/// The size of an image in pixel coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PixelSize {
    /// The width in number of pixels.
    pub width: u32,
    /// The height in number of pixels.
    pub height: u32,
}

/// The size of an image in grid coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GridSize {
    /// The number of columns.
    pub cols: u32,
    /// The number of rows.
    pub rows: u32,
}

/// The top-left cell of a storage placement, relative to the viewport.
///
/// Returned by [`PlacementIteration::viewport_pos`], which is for inspecting
/// the storage. To draw, use the render state's image list
/// ([`ImageIterator`](crate::render::ImageIterator)) instead.
///
/// The row is negative when the top of the placement scrolled above the
/// viewport. For example, an image 4 rows tall that scrolled up by 2 rows has
/// a row of -2. Its top 2 rows are above the viewport and its bottom 2 rows
/// are still visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewportPos {
    /// The column index relative to the viewport.
    pub col: i32,
    /// The row index relative to the viewport.
    pub row: i32,
}

/// The pixel position and size of a source rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceRect {
    /// The x origin in pixels.
    pub x: u32,
    /// The y origin in pixels.
    pub y: u32,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
}

bitflags::bitflags! {
/// Layers to limit the storage placement iterator to, set with
/// [`PlacementIteration::set_layer`].
///
/// The Kitty graphics protocol picks a placement's layer from its z-index, as
/// each value below describes. This filter is only for the storage iterator.
/// To draw, use the layer the render state gives each entry
/// ([`ImageIteration::layer`](crate::render::ImageIteration::layer)). It
/// follows the same rules, and it also covers virtual placements, which are
/// always drawn below the text.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Layer: u32 {
        /// Placements drawn below the cell backgrounds: z < [`i32::MIN`] / 2.
        const BELOW_BACKGROUND = ffi::IMAGE_LAYER_MASK_BELOW_BACKGROUND;
        /// Placements drawn above the cell backgrounds and below the text:
        /// [`i32::MIN`] / 2 ≤ z < 0.
        const BELOW_TEXT = ffi::IMAGE_LAYER_MASK_BELOW_TEXT;
        /// Placements drawn above the text: z ≥ 0.
        const ABOVE_TEXT = ffi::IMAGE_LAYER_MASK_ABOVE_TEXT;
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::all()
    }
}

/// Pixel format of a Kitty graphics image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
#[repr(i32)]
#[expect(missing_docs, reason = "missing upstream docs")]
pub enum ImageFormat {
    #[default]
    Rgb = ffi::KittyImageFormat::RGB,
    Rgba = ffi::KittyImageFormat::RGBA,
    Png = ffi::KittyImageFormat::PNG,
    GrayAlpha = ffi::KittyImageFormat::GRAY_ALPHA,
    Gray = ffi::KittyImageFormat::GRAY,
}

/// Compression of a Kitty graphics image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
#[repr(i32)]
#[expect(missing_docs, reason = "missing upstream docs")]
pub enum Compression {
    #[default]
    None = ffi::KittyImageCompression::NONE,
    ZlibDeflate = ffi::KittyImageCompression::ZLIB_DEFLATE,
}

#[cfg(all(test, not(miri)))]
mod placement_tests {
    use std::cell::Cell;

    use super::{Layer, PlacementIterator};
    use crate::{
        Terminal,
        error::Error,
        point::{Coordinate, Space},
    };

    #[test]
    fn layer_masks_combine_clear_and_reject_unknown_bits() {
        let mut terminal = Terminal::new(10, 5).unwrap();
        terminal.resize(10, 5, 10, 20).unwrap();
        for (id, z) in [(1, -1_073_741_825), (2, -1), (3, 0)] {
            terminal
                .vt_write(format!("\x1b_Ga=T,f=24,s=1,v=1,i={id},z={z},q=2;////\x1b\\").as_bytes());
        }
        let graphics = terminal.kitty_graphics().unwrap();
        let mut iterator = PlacementIterator::new().unwrap();
        for (mask, expected) in [
            (Layer::BELOW_BACKGROUND | Layer::ABOVE_TEXT, vec![1, 3]),
            (Layer::BELOW_TEXT, vec![2]),
            (Layer::empty(), vec![1, 2, 3]),
        ] {
            let mut placements = iterator.update(&graphics).unwrap();
            placements.set_layer(mask).unwrap();
            assert!(matches!(
                placements.set_layer(Layer::from_bits_retain(1 << 31)),
                Err(Error::InvalidValue)
            ));
            let mut found = Vec::new();
            while let Some(placement) = placements.next() {
                found.push(placement.image_id().unwrap());
            }
            found.sort_unstable();
            assert_eq!(found, expected);
        }
    }

    #[test]
    fn rect_is_none_for_virtual_placements() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        // Same geometry as upstream's "placement_rect with transmit and
        // display" test, so the expected rectangle below matches theirs:
        // 80 cols * 10px = 800px, 24 rows * 20px = 480px.
        terminal.resize(80, 24, 10, 20).unwrap();
        terminal
            .set_kitty_image_storage_limit(Some(64 * 1024 * 1024))
            .unwrap();

        // Upstream's sequence: a 1x2 RGB image placed at the cursor (0,0),
        // 10 columns wide and 1 row tall.
        terminal.vt_write(b"\x1b_Ga=T,t=d,f=24,i=1,p=1,s=1,v=2,c=10,r=1;////////\x1b\\");
        // The same image data with a virtual placement (U=1), as in
        // upstream's render tests, under another image ID so it doesn't
        // replace the first. It has no position of its own, so upstream
        // returns NO_VALUE for its rect.
        terminal.vt_write(b"\x1b_Ga=T,t=d,f=24,i=2,p=1,s=1,v=2,U=1,c=2,r=1,q=2;////////\x1b\\");

        let graphics = terminal.kitty_graphics().unwrap();
        let mut iterator = PlacementIterator::new().unwrap();
        let mut placements = iterator.update(&graphics).unwrap();

        // Placements come out in storage order, which isn't specified, so
        // check each one by kind and count that we saw both.
        let (mut normal, mut virtual_) = (0, 0);
        while let Some(placement) = placements.next() {
            let image = graphics.image(placement.image_id().unwrap()).unwrap();
            let rect = placement.rect(&image, &terminal).unwrap();

            if placement.is_virtual().unwrap() {
                virtual_ += 1;
                assert!(rect.is_none());
            } else {
                normal += 1;
                let rect = rect.unwrap();
                let point = |grid_ref| {
                    terminal
                        .point_from_grid_ref(grid_ref, Space::Active)
                        .unwrap()
                        .unwrap()
                };
                // Starts at the cursor origin, and 10 columns by 1 row
                // means the inclusive bottom-right corner is (9, 0).
                assert_eq!(point(rect.start()), Coordinate { x: 0, y: 0 });
                assert_eq!(point(rect.end()), Coordinate { x: 9, y: 0 });
                assert!(rect.rectangle());
            }
        }
        assert_eq!((normal, virtual_), (1, 1));
    }

    #[test]
    fn rect_rejects_another_terminal() {
        let narrow = narrow();
        let mut wide = Terminal::new(80, 24).unwrap();
        wide.resize(80, 24, 10, 20).unwrap();

        let graphics = narrow.kitty_graphics().unwrap();
        let mut iterator = PlacementIterator::new().unwrap();
        let mut placements = iterator.update(&graphics).unwrap();
        let placement = placements.next().unwrap();
        let image = graphics.image(1).unwrap();

        // Don't touch the refs of a mismatched call: if the check is missing,
        // the end is at column 39 of a 10 column page. Matching on the
        // variant is enough.
        assert!(matches!(
            placement.rect(&image, &wide),
            Err(Error::InvalidValue)
        ));
        // With its own terminal, upstream clamps the end to column 9.
        assert!(matches!(placement.rect(&image, &narrow), Ok(Some(_))));
    }

    #[test]
    fn rect_rejects_another_terminal_in_callbacks() {
        // Same as above, but every terminal is a view passed to a callback,
        // and `wide`'s callback runs inside `narrow`'s. Views are built by
        // the callback trampolines rather than `Terminal::new`, so this
        // checks that they carry their own terminal's id too.
        // Declared first so they outlive `narrow`, whose callback borrows
        // them.
        let (same, other) = (Cell::new(false), Cell::new(false));
        let mut narrow = narrow();
        narrow
            .on_bell(|a| {
                let graphics = a.kitty_graphics().unwrap();
                let mut iterator = PlacementIterator::new().unwrap();
                let mut placements = iterator.update(&graphics).unwrap();
                let placement = placements.next().unwrap();
                let image = graphics.image(1).unwrap();
                same.set(matches!(placement.rect(&image, a), Ok(Some(_))));

                let mut wide = Terminal::new(80, 24).unwrap();
                wide.resize(80, 24, 10, 20).unwrap();
                wide.on_bell(|b| {
                    // Don't touch the refs, as above.
                    other.set(matches!(
                        placement.rect(&image, b),
                        Err(Error::InvalidValue)
                    ));
                })
                .unwrap();
                wide.vt_write(b"\x07");
            })
            .unwrap();
        narrow.vt_write(b"\x07");
        assert!(same.get());
        assert!(other.get());
    }

    /// A narrow terminal holding a placement wider than the terminal.
    ///
    /// Upstream clamps the end column of a rect to the width of the terminal
    /// it is given, so asking with a wider terminal would give a grid ref
    /// past the end of this terminal's page.
    fn narrow<'cb>() -> Terminal<'static, 'cb> {
        let mut narrow = Terminal::new(10, 24).unwrap();
        narrow.resize(10, 24, 10, 20).unwrap();
        narrow
            .set_kitty_image_storage_limit(Some(64 * 1024 * 1024))
            .unwrap();
        // A 1x2 RGB image at the cursor (0,0), 40 columns wide and 1 row
        // tall.
        narrow.vt_write(b"\x1b_Ga=T,t=d,f=24,i=1,p=1,s=1,v=2,c=40,r=1;////////\x1b\\");
        narrow
    }
}

#[cfg(all(test, not(miri)))]
mod animation_tests {
    use crate::Terminal;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn terminal() -> Terminal<'static, 'static> {
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal
            .set_kitty_image_storage_limit(Some(64 * 1024 * 1024))
            .unwrap();
        terminal
    }

    /// The first pixel and the generation of image 1.
    fn image(terminal: &Terminal<'_, '_>) -> ([u8; 4], u64) {
        let graphics = terminal.kitty_graphics().unwrap();
        let image = graphics.image(1).unwrap();
        let pixels = image.data().unwrap().unwrap();
        (pixels[..4].try_into().unwrap(), image.generation().unwrap())
    }

    #[test]
    fn nothing_playing_returns_none() {
        let mut terminal = terminal();
        let mut graphics = terminal.kitty_graphics().unwrap();

        // Empty storage.
        assert_eq!(graphics.tick_animation(0).unwrap(), None);

        // A placed static image.
        terminal.vt_write(b"\x1b_Ga=T,f=32,i=1,s=1,v=1,q=2;/wAA/w==\x1b\\");

        // Need to re-borrow since terminal state has been altered.
        let mut graphics = terminal.kitty_graphics().unwrap();
        assert_eq!(graphics.tick_animation(0).unwrap(), None);
        assert_eq!(graphics.tick_animation(1000).unwrap(), None);
    }

    #[test]
    fn advances_one_frame_per_tick_and_reports_the_next_frame_time() {
        let mut terminal = terminal();

        // A red root frame shown for 25 ms and a blue second frame shown for
        // 60 ms, playing on a loop.
        terminal.vt_write(
            b"\x1b_Ga=T,f=32,i=1,s=1,v=1,q=2;/wAA/w==\x1b\\\
              \x1b_Ga=f,f=32,i=1,s=1,v=1,z=60,q=2;AAD//w==\x1b\\\
              \x1b_Ga=a,i=1,r=1,z=25,q=2\x1b\\\
              \x1b_Ga=a,i=1,s=3,q=2\x1b\\",
        );
        let (pixel, g0) = image(&terminal);
        assert_eq!(pixel, RED);

        // The first tick starts the red frame's timer and changes nothing.
        let mut graphics = terminal.kitty_graphics().unwrap();
        assert_eq!(graphics.tick_animation(1000).unwrap(), Some(1025));
        assert_eq!(image(&terminal), (RED, g0));

        // An early tick reports the same absolute time.
        assert_eq!(graphics.tick_animation(1010).unwrap(), Some(1025));
        assert_eq!(image(&terminal), (RED, g0));

        // On time, the blue frame shows with a new generation.
        assert_eq!(graphics.tick_animation(1025).unwrap(), Some(1085));
        let (pixel, g1) = image(&terminal);
        assert_eq!(pixel, BLUE);
        assert_ne!(g1, g0);

        // A very late tick still moves on by exactly one frame, back to red,
        // and counts the red frame's gap from now. 5030 ms is chosen so that
        // catching up would look different: blue started at 1025 ms and the
        // loop lasts 85 ms, so 5030 ms is 10 ms into a blue frame on the
        // original schedule.
        assert_eq!(graphics.tick_animation(5030).unwrap(), Some(5055));
        let (pixel, g2) = image(&terminal);
        assert_eq!(pixel, RED);
        assert_ne!(g2, g1);

        // Stopped playback has nothing to schedule and keeps the frame.
        terminal.vt_write(b"\x1b_Ga=a,i=1,s=1,q=2\x1b\\");
        // Re-fetch graphics after mutating the terminal
        let mut graphics = terminal.kitty_graphics().unwrap();
        assert_eq!(graphics.tick_animation(6000).unwrap(), None);
        assert_eq!(image(&terminal), (RED, g2));
    }
}

#[cfg(all(test, not(miri), feature = "kitty-graphics", feature = "png"))]
mod png_decoder_tests {
    use super::*;
    use crate::alloc::testing::Capped;

    struct Png<'a> {
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        palette: Option<&'a [u8]>,
        trns: Option<&'a [u8]>,
        pixels: &'a [u8],
    }

    impl Png<'_> {
        fn encode(&self) -> Vec<u8> {
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
            encoder.set_color(self.color);
            encoder.set_depth(self.depth);
            if let Some(palette) = self.palette {
                encoder.set_palette(palette);
            }
            if let Some(trns) = self.trns {
                encoder.set_trns(trns);
            }
            encoder
                .write_header()
                .unwrap()
                .write_image_data(self.pixels)
                .unwrap();
            out
        }
    }

    fn png(color: png::ColorType, depth: png::BitDepth, pixels: &[u8]) -> Png<'_> {
        Png {
            width: 1,
            height: 1,
            color,
            depth,
            palette: None,
            trns: None,
            pixels,
        }
    }

    fn decode(encoded: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
        RustPngDecoder
            .decode_png(&Allocator::GLOBAL, encoded)
            .map(|image| (image.width, image.height, image.data.to_vec()))
    }

    #[test]
    fn decodes_every_color_type_to_rgba8() {
        use png::{BitDepth, ColorType};

        let cases: [(&str, Png<'_>, [u8; 4]); 8] = [
            (
                "rgba8",
                png(ColorType::Rgba, BitDepth::Eight, &[255, 0, 128, 7]),
                [255, 0, 128, 7],
            ),
            (
                "rgb8 gains opaque alpha",
                png(ColorType::Rgb, BitDepth::Eight, &[1, 2, 3]),
                [1, 2, 3, 255],
            ),
            (
                "rgba16 is stripped to 8 bits",
                png(
                    ColorType::Rgba,
                    BitDepth::Sixteen,
                    &[0xAB, 0xCD, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC],
                ),
                [0xAB, 0x12, 0x56, 0x9A],
            ),
            (
                "gray8",
                png(ColorType::Grayscale, BitDepth::Eight, &[128]),
                [128, 128, 128, 255],
            ),
            (
                "gray2 is expanded to 8 bits",
                png(ColorType::Grayscale, BitDepth::Two, &[0b1000_0000]),
                [0xAA, 0xAA, 0xAA, 255],
            ),
            (
                "gray+alpha8",
                png(ColorType::GrayscaleAlpha, BitDepth::Eight, &[64, 96]),
                [64, 64, 64, 96],
            ),
            (
                "palette",
                Png {
                    palette: Some(&[0, 0, 0, 10, 20, 30]),
                    ..png(ColorType::Indexed, BitDepth::Eight, &[1])
                },
                [10, 20, 30, 255],
            ),
            (
                "palette with tRNS",
                Png {
                    palette: Some(&[0, 0, 0, 10, 20, 30]),
                    trns: Some(&[255, 42]),
                    ..png(ColorType::Indexed, BitDepth::Eight, &[1])
                },
                [10, 20, 30, 42],
            ),
        ];

        for (name, image, expected) in cases {
            assert_eq!(
                decode(&image.encode()),
                Some((1, 1, expected.to_vec())),
                "{name}"
            );
        }
    }

    #[test]
    fn expands_multi_pixel_grayscale_alpha_in_place() {
        // Several pixels exercise the backwards in-place expansion; a single
        // pixel would not catch overlapping reads and writes.
        let pixels = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let image = Png {
            width: 3,
            height: 2,
            ..png(
                png::ColorType::GrayscaleAlpha,
                png::BitDepth::Eight,
                &pixels,
            )
        };
        let expected: Vec<u8> = pixels
            .chunks_exact(2)
            .flat_map(|ga| [ga[0], ga[0], ga[0], ga[1]])
            .collect();
        assert_eq!(decode(&image.encode()), Some((3, 2, expected)));
    }

    #[test]
    fn rejects_malformed_input() {
        let encoded = png(png::ColorType::Rgba, png::BitDepth::Eight, &[1, 2, 3, 4]).encode();
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(b"definitely not a png"), None);
        assert_eq!(decode(&encoded[..encoded.len() / 2]), None);
    }

    #[test]
    fn oversized_dimensions_are_rejected_by_the_provided_allocator() {
        // A tiny PNG whose header claims 16000x16000 RGBA8 pixels (~1 GB).
        // The decoder must ask the provided allocator for the output first,
        // so the cap rejects it before any work is done on the global heap.
        let mut encoded = png(png::ColorType::Rgba, png::BitDepth::Eight, &[0; 4]).encode();
        encoded[16..24].copy_from_slice(&[0, 0, 0x3E, 0x80, 0, 0, 0x3E, 0x80]);
        let crc = crc32(&encoded[12..29]);
        encoded[29..33].copy_from_slice(&crc.to_be_bytes());

        let capped = Capped::new(1024 * 1024);
        assert!(
            RustPngDecoder
                .decode_png(&capped.allocator(), &encoded)
                .is_none()
        );
        assert_eq!(capped.largest_request(), 16000 * 16000 * 4);
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 0 {
                    crc >> 1
                } else {
                    0xEDB8_8320 ^ (crc >> 1)
                };
            }
        }
        !crc
    }
}
