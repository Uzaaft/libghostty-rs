//! Complete terminal emulator state and rendering.
//!
//! A [`Terminal`] is a whole terminal emulator: you write VT data into it, and
//! it keeps the screen, scrollback, cursor, styles, and modes.
//!
//! Nearly everything in this library starts here. All you need to make one is a
//! size. It doesn't draw and it doesn't talk to a pty. Both of those are yours.
//! If you only want one parser or encoder, the [Building Blocks](crate#building-blocks)
//! work without a terminal.
//!
//! # Example
//!
//! ```rust
//! use libghostty_vt::Terminal;
//!
//! let mut terminal = Terminal::new(80, 24).unwrap();
//! terminal.vt_write(b"Hello, \x1b[1;32mworld\x1b[0m!\r\n");
//! ```
//!
//! The guides cover the rest: [Effects] for what programs ask of the terminal,
//! [Colors], [Scrollback], and [Custom Sequences].
//!
#![doc = crate::guide_ref!("Effects", "effects")]
#![doc = crate::guide_ref!("Colors", "colors")]
#![doc = crate::guide_ref!("Scrollback", "scrollback")]
#![doc = crate::guide_ref!("Custom Sequences", "custom-sequences")]

use std::{any::Any, mem::MaybeUninit};

use crate::{
    alloc::{Allocator, Object},
    error::{Error, Result, from_optional_result, from_optional_result_uninit, from_result},
    ffi::{self, TerminalData as Data, TerminalOption as Opt},
    key, mouse,
    screen::Screen,
    style::Style,
};

#[doc(inline)]
pub use ffi::TerminalScrollbar as Scrollbar;

/// Opaque handle to a terminal instance.
#[derive(Debug)]
pub struct Terminal<'alloc> {
    pub(crate) inner: Object<'alloc, ffi::TerminalImpl>,
    pub(crate) effect_handler: Option<Box<dyn Any>>,
}

impl<'alloc> Terminal<'alloc> {
    /// Create a new terminal instance.
    ///
    /// The terminal starts with various reasonable defaults e.g. around
    /// scrollback limits. Use the `Terminal::set_*` family of methods
    /// to change any options prior to using the terminal.
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), cols, rows) }
    }

    /// Create a new terminal instance with a custom allocator.
    ///
    /// The terminal starts with various reasonable defaults e.g. around
    /// scrollback limits. Use the `Terminal::set_*` family of methods
    /// to change any options prior to using the terminal.
    ///
    /// See [Memory] regarding custom memory management and lifetimes.
    ///
    #[doc = crate::book_ref!("Memory", "memory")]
    pub fn new_in<'ctx: 'alloc>(
        alloc: &'alloc Allocator<'ctx>,
        cols: u16,
        rows: u16,
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), cols, rows) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator, cols: u16, rows: u16) -> Result<Self> {
        let mut raw: ffi::Terminal = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_terminal_new(alloc, &raw mut raw, cols, rows) };
        from_result(result)?;
        unsafe { Self::from_raw(raw) }
    }

    pub(crate) unsafe fn from_raw(raw: ffi::Terminal) -> Result<Self> {
        Ok(Self {
            inner: Object::new(raw)?,
            effect_handler: None,
        })
    }

    /// Write VT-encoded data to the terminal for processing.
    ///
    /// Feeds raw bytes through the terminal's VT stream parser, updating
    /// terminal state accordingly. By default, sequences that require output
    /// (queries, device status reports) are silently ignored.
    /// Use [`crate::effect::OnPtyWrite`] to install a callback that receives
    /// response data.
    ///
    /// This never fails. Any erroneous input or errors in processing the input
    /// are logged internally but do not cause this function to fail because
    /// this input is assumed to be untrusted and from an external source; so
    /// the primary goal is to keep the terminal state consistent and not allow
    /// malformed input to corrupt or crash.    
    pub fn vt_write(&mut self, data: &[u8]) {
        unsafe { ffi::ghostty_terminal_vt_write(self.inner.as_raw(), data.as_ptr(), data.len()) }
    }

    /// Write VT-encoded data to the terminal, stopping right after the first
    /// byte that reaches one of the requested boundaries.
    ///
    /// This works like [`Terminal::vt_write`], but it can pause at points you
    /// care about. It tells you exactly where a control string starts or ends,
    /// using the same parse that updates the terminal. For example, a program
    /// that sits between a shell and a terminal can use it to:
    ///
    /// - Find the exact bytes of each APC, DCS or OSC string, such as a Kitty
    ///   graphics command, a tmux control mode message or a clipboard write,
    ///   so it can forward, rewrite or drop that string
    ///
    /// - Wait until the stream is at ground before inserting its own sequences,
    ///   so it never splits a sequence from the program.
    ///
    /// Pass the boundaries to stop at as [`Boundary`] flags. The call consumes
    /// input up to and including the first byte that reaches one of them, and
    /// reports that point in out. The terminal only processes the consumed
    /// bytes, and callbacks only run for those bytes. Pass the remaining bytes
    /// in your next call.
    ///
    /// Every byte you pass is eventually consumed in order, so the terminal
    /// ends up in the same state as if you had called [`Terminal::vt_write`]
    /// with all of it.
    ///
    /// See [Stopping a write at a string boundary] for how to find string
    /// content, the cases that need care, and waiting for ground.
    #[doc = crate::guide_ref!(
        "Stopping a write at a string boundary",
        "custom-sequences",
        "write-until"
    )]
    pub fn vt_write_until(
        &mut self,
        data: &[u8],
        boundaries: Boundary,
    ) -> Result<Option<WriteStop>> {
        let mut stop = ffi::sized!(ffi::TerminalWriteStop);
        // SAFETY: `data` points to `data.len()` readable bytes (never NULL,
        // even when empty, and upstream only reads `len` bytes), and `stop`
        // is a live sized struct, which is what upstream requires of `out`.
        // Callbacks run synchronously in here but only get a `&Terminal`, so
        // they can't call back into `vt_write*` and break upstream's
        // no-reentrancy rule. Upstream rejects an empty or unknown
        // `boundaries` with INVALID_VALUE rather than misbehaving.
        let result = unsafe {
            ffi::ghostty_terminal_vt_write_until(
                self.inner.as_raw(),
                data.as_ptr(),
                data.len(),
                boundaries.bits(),
                &raw mut stop,
            )
        };
        // NO_VALUE means every byte was consumed without reaching a boundary,
        // so there is no stop to report.
        let Some(stop) = from_optional_result(result, stop)? else {
            return Ok(None);
        };

        let minimum_size =
            std::mem::offset_of!(ffi::TerminalWriteStop, byte) + std::mem::size_of::<u8>();
        if stop.size < minimum_size {
            return Err(Error::InvalidValue);
        }

        // Upstream zero-fills the fields that don't apply to a stop, and zero
        // is also a valid value for `kind` (APC), `introducer` (ESC) and
        // `end` (ST_ESC). Reading them blindly would make, say, a ground stop
        // look like an APC ended by ESC, so we only read the fields that
        // `reached` says apply and return `None` for the rest.
        let reached = Boundary::from_bits_retain(stop.reached);
        let started =
            reached.intersects(Boundary::APC_START | Boundary::DCS_START | Boundary::OSC_START);
        let ended = reached.intersects(Boundary::APC_END | Boundary::DCS_END | Boundary::OSC_END);
        let is_string = started || ended;

        Ok(Some(WriteStop {
            consumed: stop.consumed,
            reached,
            kind: is_string
                .then(|| stop.kind.try_into())
                .transpose()
                .map_err(|_| Error::Unsupported)?,
            introducer: started
                .then(|| stop.introducer.try_into())
                .transpose()
                .map_err(|_| Error::Unsupported)?,
            end: ended
                .then(|| stop.end.try_into())
                .transpose()
                .map_err(|_| Error::Unsupported)?,
            byte: is_string.then_some(stop.byte),
        }))
    }

    /// Whether VT processing is at ground.
    ///
    /// Ground is when the stream isn't in the middle of any type of sequence:
    /// UTF-8, ESC, CSI, OSC, etc. It is the stateless point of the stream.
    ///
    /// This is useful to know because it is a point at which you can safely
    /// insert out-of-band VT sequences. For example, while reading from a pty
    /// if you want to make your own changes, you can wait until the pty input
    /// reaches ground, then write yours.
    ///
    /// Use [`Self::vt_write_until`] with [`Boundary::GROUND`] to write input
    /// only until the stream reaches ground.
    pub fn is_vt_ground(&self) -> Result<bool> {
        self.get(Data::VT_GROUND)
    }

    /// Get the terminal width in cells.
    pub fn cols(&self) -> Result<u16> {
        self.get(Data::COLS)
    }

    /// Get the terminal height in cells.
    pub fn rows(&self) -> Result<u16> {
        self.get(Data::ROWS)
    }

    /// Get the total width of the terminal in pixels.
    ///
    /// This is `cols * cell_width_px` as set by [`Terminal::resize`].
    pub fn width_px(&self) -> Result<u32> {
        self.get(Data::WIDTH_PX)
    }

    /// Get the total height of the terminal in pixels.
    ///
    /// This is `rows * cell_height_px` as set by [`Terminal::resize`].
    pub fn height_px(&self) -> Result<u32> {
        self.get(Data::HEIGHT_PX)
    }

    /// Resize the terminal to the given dimensions.
    ///
    /// Changes the number of columns and rows in the terminal. The primary
    /// screen will reflow content if wraparound mode is enabled; the alternate
    /// screen does not reflow. If the dimensions are unchanged, the grid is
    /// left as is, but everything below still applies.
    ///
    /// This also updates the terminal's pixel dimensions (used for image
    /// protocols and size reports), disables synchronized output mode (allowed
    /// by the spec so that resize results are shown immediately), and sends an
    /// in-band size report if mode 2048 is enabled.
    ///
    /// If synchronized output was enabled, the [render hold](crate::effect::OnRenderHold)
    /// callback is invoked to report that the hold ended.
    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_terminal_resize(
                self.inner.as_raw(),
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            )
        };
        from_result(result)
    }

    /// Perform a full reset of the terminal (RIS).
    ///
    /// Resets all terminal state back to its initial configuration,
    /// including modes, scrollback, scrolling region, and screen contents.
    /// The terminal dimensions are preserved.
    ///
    /// If synchronized output was enabled, the [render hold](crate::effect::OnRenderHold)
    /// callback is invoked to report that the hold ended.
    ///
    /// This doesn't call the [reset](crate::effect::OnReset) callback.
    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_terminal_reset(self.inner.as_raw()) }
    }

    /// Scroll the terminal viewport.
    ///
    /// Scrolls the terminal's viewport according to the given behavior.
    /// See [`ScrollViewport::DeltaPrompt`] for the navigation behavior.
    pub fn scroll_viewport(&mut self, scroll: ScrollViewport) -> Result<()> {
        let result =
            unsafe { ffi::ghostty_terminal_scroll_viewport(self.inner.as_raw(), scroll.into()) };
        from_result(result)
    }

    /// Compress eligible terminal scrollback.
    ///
    /// Incremental mode performs bounded work suitable for an idle callback.
    /// A pending result means the application should invoke another step while
    /// the terminal remains idle. A complete result means no continuation is
    /// needed until [`Terminal::compression_activity`] changes. Full mode
    /// performs one synchronous scan and can stall on large scrollback buffers.
    ///
    /// Compression is opportunistic. Complete means the pass has finished,
    /// not that every page was compressed: pages may be unprofitable or
    /// encounter an allocation or reclamation failure. Compression changes
    /// only the terminal's storage representation and never its logical
    /// contents or scrollback limit. Accessing compressed history restores
    /// it transparently.
    ///
    /// This function is not thread-safe with other operations on the same
    /// terminal. The caller must serialize it with writes, rendering, searches,
    /// and other terminal access.
    pub fn compress(&mut self, mode: CompressionMode) -> Result<CompressionResult> {
        let mut value = ffi::TerminalCompressionResult::UNSUPPORTED;
        let result = unsafe {
            ffi::ghostty_terminal_compress(self.inner.as_raw(), mode.into(), &raw mut value)
        };
        from_result(result)?;
        value.try_into().map_err(|_| Error::Unsupported)
    }

    /// Return the current compression activity token.
    ///
    /// The token is opaque and only equality comparisons are meaningful.
    /// An embedding application should cache it and restart its compression
    /// idle delay whenever the value changes. The value may wrap and changes
    /// in either direction have the same meaning.
    ///
    /// This function only observes terminal state.
    /// It does not perform or schedule compression.
    pub fn compression_activity(&self) -> Result<CompressionActivity> {
        let mut value = 0;
        let result = unsafe {
            ffi::ghostty_terminal_compression_activity(self.inner.as_raw(), &raw mut value)
        };
        from_result(result)?;
        Ok(CompressionActivity(value))
    }

    /// How much memory the terminal holds. See [`MemoryUsage`] for what each
    /// field means.
    ///
    /// This never decompresses scrollback, but it does look at every page, so
    /// avoid reading it after every write.
    pub fn memory_usage(&self) -> Result<MemoryUsage> {
        // This can't go through `Self::get`: that zero-fills the output, so
        // `size` would be 0 and upstream rejects any size smaller than the
        // struct it knows with `INVALID_VALUE`. `sized!` stamps in the size
        // of the struct we were compiled against instead.
        let mut usage = ffi::sized!(ffi::TerminalMemoryUsage);
        // SAFETY: `usage` is a live, properly sized `TerminalMemoryUsage`,
        // which is exactly the output type upstream documents for this key.
        let result = unsafe {
            ffi::ghostty_terminal_get(
                self.inner.as_raw(),
                Data::MEMORY_USAGE,
                (&raw mut usage).cast(),
            )
        };
        from_result(result)?;
        MemoryUsage::from_raw(usage)
    }
    pub(crate) fn get<T>(&self, tag: ffi::TerminalData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_terminal_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }
    pub(crate) fn get_optional<T>(&self, tag: ffi::TerminalData::Type) -> Result<Option<T>> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_terminal_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_optional_result_uninit(result, value)
    }
    pub(crate) fn set<T>(&self, tag: ffi::TerminalOption::Type, v: &T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_terminal_set(self.inner.as_raw(), tag, std::ptr::from_ref(v).cast())
        };
        from_result(result)
    }
    /// Set an option whose ABI expects the pointer value itself, not a pointer
    /// to Rust storage containing that value.
    pub(crate) fn set_ptr(
        &self,
        tag: ffi::TerminalOption::Type,
        ptr: *const std::ffi::c_void,
    ) -> Result<()> {
        let result = unsafe { ffi::ghostty_terminal_set(self.inner.as_raw(), tag, ptr) };
        from_result(result)
    }
    pub(crate) fn set_optional<T>(
        &self,
        tag: ffi::TerminalOption::Type,
        v: Option<&T>,
    ) -> Result<()> {
        let ptr = if let Some(v) = v {
            std::ptr::from_ref(v)
        } else {
            std::ptr::null()
        };

        let result = unsafe { ffi::ghostty_terminal_set(self.inner.as_raw(), tag, ptr.cast()) };
        from_result(result)
    }
    /// The mouse pointer shape requested by the application through OSC 22.
    ///
    /// Initially [`mouse::Shape::Text`]. An empty OSC 22 resets it to that,
    /// and a name libghostty doesn't know leaves it unchanged. Excludes host
    /// hover overrides.
    pub fn mouse_shape(&self) -> Result<mouse::Shape> {
        self.get::<ffi::MouseShape::Type>(Data::MOUSE_SHAPE)?
            .try_into()
            .map_err(|_| Error::InvalidValue)
    }
    /// Get the current Kitty keyboard protocol flags.
    pub fn kitty_keyboard_flags(&self) -> Result<key::KittyKeyFlags> {
        self.get::<ffi::KittyKeyFlags>(Data::KITTY_KEYBOARD_FLAGS)
            .map(key::KittyKeyFlags::from_bits_retain)
    }

    /// Get the scrollbar state for the terminal viewport.
    ///
    /// This is amortized `O(1)`: the total is maintained incrementally as
    /// the terminal is modified and the viewport offset is cached. The
    /// first read after the viewport moves to an arbitrary position that
    /// isn't an absolute row (e.g. scrolling to a selection) may cost
    /// `O(pages)` to compute the offset, after which it is cached again.
    ///
    /// There is intentionally no change notification for scroll state.
    /// Callers building scrollbars should poll this once per frame or
    /// per write batch and diff the result to detect changes; this is
    /// what Ghostty's own renderer does.
    pub fn scrollbar(&self) -> Result<Scrollbar> {
        self.get(Data::SCROLLBAR)
    }
    /// Get the currently active screen.
    pub fn active_screen(&self) -> Result<Screen> {
        self.get::<ffi::TerminalScreen::Type>(Data::ACTIVE_SCREEN)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }
    /// Whether the viewport is currently pinned to the active area.
    ///
    /// This is true when the viewport is following the active terminal area,
    /// and false when the user has scrolled into history.
    pub fn viewport_active(&self) -> Result<bool> {
        self.get(Data::VIEWPORT_ACTIVE)
    }
    /// Get whether any mouse tracking mode is active.
    ///
    /// Returns true if any of the mouse tracking modes (X10, normal, button,
    /// or any-event) are enabled.
    pub fn is_mouse_tracking(&self) -> Result<bool> {
        self.get(Data::MOUSE_TRACKING)
    }
    /// Whether VT processing encountered a non-gracefully handled error that
    /// may have prevented a terminal-owned semantic update.
    ///
    /// Processing remains best-effort, and [`Terminal::reset`] does not clear
    /// this flag; it is purely informational. Gracefully handled protocol
    /// failures, configured limits, malformed or unsupported input, and
    /// failures limited to external effects or query responses do not set it.
    pub fn vt_processing_error(&self) -> Result<bool> {
        self.get(Data::VT_PROCESSING_ERROR)
    }
    /// The terminal title as set by escape sequences (e.g. OSC 0/2).
    ///
    /// Returns a borrowed string. It is valid until the next mutating terminal
    /// call (e.g. [`Terminal::vt_write`]). An empty string is returned when no
    /// title has been set.
    ///
    /// The bytes come from the program and are not guaranteed to be UTF-8.
    pub fn title(&self) -> Result<&[u8]> {
        let str = self.get::<ffi::String>(Data::TITLE)?;
        // SAFETY: libghostty documents the returned pointer as valid until the
        // next mutating terminal call. Not every mutating call takes
        // `&mut self` (`set_selection` doesn't), but libghostty only rewrites
        // the title and pwd storage when processing input, resetting, or
        // through the TITLE and PWD options. These bindings expose the first
        // two only through `&mut self` methods (`vt_write`,
        // `vt_write_until`, `reset`), and the options only through
        // `set_title` and `set_pwd`, which take `&mut self` too. No UTF-8 or
        // NUL-termination is assumed; we only read `len` bytes.
        Ok(unsafe { str.to_bytes() })
    }

    /// The terminal's current working directory as set by escape sequences
    /// (e.g. OSC 7).
    ///
    /// Returns a borrowed string. It is valid until the next mutating terminal
    /// call (e.g. [`Terminal::vt_write`]). An empty string is returned when no
    /// pwd has been set.
    ///
    /// The terminal stores whatever bytes the shell emitted, without parsing.
    /// That means for OSC 7 the value is the raw URI (typically `file://...`);
    /// for OSC 9/OSC 1337 it is typically a bare path. The bytes are not
    /// guaranteed to be UTF-8.
    pub fn pwd(&self) -> Result<&[u8]> {
        let str = self.get::<ffi::String>(Data::PWD)?;
        // SAFETY: Ditto (see `title`): only `&mut self` methods can rewrite
        // the pwd storage, and only `len` bytes are read.
        Ok(unsafe { str.to_bytes() })
    }

    /// Set the terminal title manually.
    ///
    /// The string data is copied into the terminal. An empty title clears
    /// it. Read it back with [`Terminal::title`].
    ///
    /// On `Err(Error::OutOfMemory)`, the previous title is left unchanged.
    ///
    /// ```
    /// use libghostty_vt::Terminal;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut terminal = Terminal::new(80, 24)?;
    ///
    /// // Most titles are plain strings, which pass straight in.
    /// terminal.set_title("vim — main.rs")?;
    /// assert_eq!(terminal.title()?, "vim — main.rs".as_bytes());
    ///
    /// // Titles are bytes, not UTF-8 strings: this one is Latin-1.
    /// terminal.set_title(b"caf\xe9")?;
    /// assert_eq!(terminal.title()?, b"caf\xe9");
    ///
    /// // So the bytes `title` returns can be set again unchanged.
    /// let mut other = Terminal::new(80, 24)?;
    /// other.set_title(terminal.title()?)?;
    /// assert_eq!(other.title()?, b"caf\xe9");
    ///
    /// // An empty title clears it.
    /// terminal.set_title("")?;
    /// assert!(terminal.title()?.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_title(&mut self, title: impl AsRef<[u8]>) -> Result<&mut Self> {
        // Take `AsRef<[u8]>` rather than `&str`: strings are the common
        // case, but upstream accepts arbitrary bytes and `title` returns
        // bytes, so any value read back must be settable again as-is.
        let title = title.as_ref();
        // The `From` impl on `ffi::String` only takes `str`, but upstream
        // accepts arbitrary bytes, so build it from the slice by hand.
        // Upstream copies the bytes before returning, so the pointer only
        // has to live for this call. An empty slice may carry a dangling
        // (non-NULL) pointer; that's fine, since upstream reads `len` bytes
        // and clears the title for a zero length.
        let raw = ffi::String {
            ptr: title.as_ptr(),
            len: title.len(),
        };
        self.set(Opt::TITLE, &raw)?;
        Ok(self)
    }

    /// Set the terminal working directory manually.
    ///
    /// The string data is copied into the terminal. An empty pwd clears it.
    /// Read it back with [`Terminal::pwd`].
    ///
    /// On `Err(Error::OutOfMemory)`, the previous pwd is left unchanged.
    ///
    /// ```
    /// use libghostty_vt::Terminal;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut terminal = Terminal::new(80, 24)?;
    ///
    /// // Most paths are plain strings, which pass straight in.
    /// terminal.set_pwd("/home/user/src")?;
    /// assert_eq!(terminal.pwd()?, "/home/user/src".as_bytes());
    ///
    /// // The pwd is stored as-is: neither parsed as a URI nor required to be
    /// // UTF-8.
    /// terminal.set_pwd(b"/home/caf\xe9")?;
    /// assert_eq!(terminal.pwd()?, b"/home/caf\xe9");
    ///
    /// // An empty pwd clears it.
    /// terminal.set_pwd("")?;
    /// assert!(terminal.pwd()?.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_pwd(&mut self, pwd: impl AsRef<[u8]>) -> Result<&mut Self> {
        // Ditto (see `set_title`): `AsRef<[u8]>` so values from `pwd`
        // round-trip, and upstream copies the bytes, so the slice only has
        // to outlive this call.
        let pwd = pwd.as_ref();
        let raw = ffi::String {
            ptr: pwd.as_ptr(),
            len: pwd.len(),
        };
        self.set(Opt::PWD, &raw)?;
        Ok(self)
    }

    /// Set the maximum bytes the APC handler will buffer for all protocols.
    ///
    /// This prevents malicious input from causing unbounded memory allocation.
    /// A `None` value removes all overrides, reverting to the built-in defaults.
    /// APC sequences no protocol implements are bounded separately, by
    /// [`set_unknown_max_bytes`](Self::set_unknown_max_bytes).
    pub fn set_apc_max_bytes(&mut self, max: Option<usize>) -> Result<&mut Self> {
        self.set_optional(Opt::APC_MAX_BYTES, max.as_ref())?;
        Ok(self)
    }

    /// Set the most bytes of each unsupported sequence to keep and pass to the
    /// [unknown sequence callback](crate::effect::OnUnknownSequence). The same limit
    /// applies to APC and OSC sequences.
    ///
    /// Zero, the default, turns unsupported sequence reporting off.
    ///
    /// A sequence longer than the limit is still reported. Its content holds
    /// the first bytes up to the limit, and `truncated` is true.
    ///
    /// Choose a limit that fits the largest sequence you expect. Unknown OSC
    /// sequences up to 2048 bytes are kept in a buffer the terminal already
    /// owns, so limits up to 2048 add no memory allocations for OSC. Larger
    /// limits allocate memory for each unknown OSC sequence. Unknown APC
    /// sequences are always kept in allocated memory.
    pub fn set_unknown_max_bytes(&mut self, max: usize) -> Result<&mut Self> {
        self.set(Opt::UNKNOWN_MAX_BYTES, &max)?;
        Ok(self)
    }

    /// Set the name of the terminfo entry this terminal runs as, reported in
    /// response to an XTGETTCAP query for `TN` (e.g. `xterm-256color`).
    ///
    /// The name is copied into the terminal. An empty name clears it. A name
    /// longer than 128 bytes returns `Err(Error::InvalidValue)` and leaves the
    /// previous name unchanged.
    ///
    /// If this is unset then nothing is reported for an XTGETTCAP `TN` query,
    /// because libghostty doesn't know what the embedding terminal advertises
    /// itself as.
    pub fn set_terminfo_name(&mut self, name: impl AsRef<[u8]>) -> Result<&mut Self> {
        // Ditto (see `set_title`): `AsRef<[u8]>` since upstream takes the
        // name as arbitrary bytes with no UTF-8 promise, and upstream copies
        // the bytes, so the slice only has to outlive this call.
        let name = name.as_ref();
        let raw = ffi::String {
            ptr: name.as_ptr(),
            len: name.len(),
        };
        self.set(Opt::TERMINFO_NAME, &raw)?;
        Ok(self)
    }

    /// The configured maximum decoded bytes per Kitty clipboard protocol
    /// (OSC 5522) write transaction.
    ///
    /// See [`Self::set_clipboard_write_max_bytes`].
    pub fn clipboard_write_max_bytes(&self) -> Result<usize> {
        self.get(Data::CLIPBOARD_WRITE_MAX_BYTES)
    }

    /// Set the maximum total decoded bytes a single Kitty clipboard protocol
    /// (OSC 5522) write transaction may accumulate. The limit is captured
    /// when a transaction begins; an in-flight transaction keeps the limit it
    /// started with.
    ///
    /// Data beyond the limit fails the whole transaction with EFBIG. The
    /// transaction is discarded, later write-related packets are ignored
    /// until a new write begins, and nothing reaches the
    /// [clipboard write callback](crate::effect::OnClipboardWrite).
    ///
    /// Transactions are buffered in memory, so this limit bounds how much
    /// memory a single write can make the terminal allocate. Pass
    /// `Some(usize::MAX)` to remove the limit. `None` reverts to the built-in
    /// default of 64 MiB, the minimum required by the protocol.
    ///
    /// This limit doesn't apply to OSC 52 writes, which are bounded by the
    /// maximum length of an escape sequence instead.
    pub fn set_clipboard_write_max_bytes(&mut self, limit: Option<usize>) -> Result<&mut Self> {
        self.set_optional(Opt::CLIPBOARD_WRITE_MAX_BYTES, limit.as_ref())?;
        Ok(self)
    }

    /// Enable or disable Glyph Protocol APC handling.
    ///
    /// Disabling the protocol makes the terminal ignore Glyph Protocol APC
    /// sequences and clears the session's glyph glossary.
    pub fn set_glyph_protocol_enabled(&mut self, enabled: bool) -> Result<&mut Self> {
        self.set(Opt::GLYPH_PROTOCOL, &enabled)?;
        Ok(self)
    }

    /// Enable window title reports in response to `CSI 21 t`.
    ///
    /// This is disabled by default because a running program can set a title
    /// and query it back into the pty input stream, potentially injecting
    /// commands that execute after user interaction.
    ///
    /// Passing `false` disables title reporting.
    pub fn set_title_report_enabled(&mut self, enabled: bool) -> Result<&mut Self> {
        self.set(Opt::TITLE_REPORT, &enabled)?;
        Ok(self)
    }

    /// Enable checksum reports in response to DECRQCRA
    /// (`CSI Pi ; Pg ; Pt ; Pl ; Pb ; Pr * y`).
    ///
    /// This is disabled by default because a running program can checksum the
    /// screen one cell at a time and so read back everything on it, including
    /// output from other programs. Passing `false` disables checksum
    /// reporting.
    ///
    /// While this is disabled, XTCHECKSUM (`CSI Ps # y`), which changes how
    /// the checksum is calculated, is ignored as well. See
    /// [`Self::set_xt_checksum_extension`].
    pub fn set_xt_checksum_report_enabled(&mut self, enabled: bool) -> Result<&mut Self> {
        self.set(Opt::XT_CHECKSUM_REPORT, &enabled)?;
        Ok(self)
    }

    /// Set how the DECRQCRA checksum is calculated after a reset (RIS or
    /// DECSTR). This also changes the current calculation.
    ///
    /// The value holds the same bits as XTCHECKSUM (`CSI Ps # y`) and xterm's
    /// `checksumExtension` resource, which a running program can still use to
    /// change the calculation until the next reset. See
    /// [`XtChecksumExtension`] for the bits.
    ///
    /// [`XtChecksumExtension::empty()`] is the calculation of a real DEC
    /// terminal. Values above 31, i.e. any bits outside
    /// [`XtChecksumExtension::all()`] kept by
    /// [`XtChecksumExtension::from_bits_retain`], return
    /// `Err(Error::InvalidValue)` and leave both the current and the reset
    /// calculation unchanged.
    ///
    /// Checksums are only reported once enabled with
    /// [`Self::set_xt_checksum_report_enabled`].
    pub fn set_xt_checksum_extension(&mut self, ext: XtChecksumExtension) -> Result<&mut Self> {
        // Upstream treats NULL and 0 the same, so there's no need for an
        // `Option` here: the empty set already means "the DEC default".
        self.set(Opt::XT_CHECKSUM_EXTENSION, &ext.bits())?;
        Ok(self)
    }
}

/// Methods related to history and scrollback.
impl Terminal<'_> {
    /// The total number of rows in the active screen including scrollback.
    pub fn total_rows(&self) -> Result<usize> {
        self.get(Data::TOTAL_ROWS)
    }

    /// The number of scrollback rows (total rows minus viewport rows).
    pub fn scrollback_rows(&self) -> Result<usize> {
        self.get(Data::SCROLLBACK_ROWS)
    }

    /// The configured maximum scrollback allocation in bytes.
    ///
    /// This always reports the primary screen's configured value, including
    /// while an alternate screen is active.
    ///
    /// Returns `None` when the configured byte limit is unlimited.
    pub fn scrollback_max_bytes(&self) -> Result<Option<usize>> {
        self.get_optional(Data::SCROLLBACK_MAX_BYTES)
    }

    /// Set the maximum scrollback allocation in bytes.
    ///
    /// This is an estimate. Internally, libghostty only prunes bytes up
    /// to a "page"-granularity. A page is the minimum allocated unit of
    /// grid space within Ghostty. A page at the time of writing these docs
    /// is about 400KB, so the byte limit will be within this delta.
    ///
    /// This works alongside the line limit configuration. If both are set,
    /// the first-reached limit is used first. Both limits are dependent
    /// on external state (byte limit can be reached with less lines if
    /// more styles are used for example, line limit can be reached with
    /// a narrower terminal viewport). So, they are useful together.
    ///
    /// Lowering the limit immediately removes eligible complete historical
    /// pages. A value of zero disables scrollback and erases retained history.
    /// `None` restores the default of 10,000 bytes, rounded up to a page.
    /// Pass `Some(usize::MAX)` for no byte limit.
    pub fn set_scrollback_max_bytes(&mut self, v: Option<usize>) -> Result<&mut Self> {
        self.set_optional(Opt::SCROLLBACK_MAX_BYTES, v.as_ref())?;
        Ok(self)
    }

    /// The configured maximum number of physical scrollback lines.
    ///
    /// This always reports the primary screen's configured value, including
    /// while an alternate screen is active.
    ///
    /// Returns `None` when the configured line limit is unlimited.
    pub fn scrollback_max_lines(&self) -> Result<Option<usize>> {
        self.get_optional(Data::SCROLLBACK_MAX_LINES)
    }

    /// Set the maximum number of physical lines retained in scrollback.
    ///
    /// This is an estimate. Internally, libghostty only prunes lines up
    /// to a "page"-granularity. A page is the minimum allocated unit of
    /// grid space within Ghostty. As a result, the actual available scrollback
    /// lines will almost always be higher than configured. The magnitude
    /// of the difference depends on the number of used styles, graphemes, etc.
    /// since the row-count in a page is dynamic based on that. In general,
    /// it ranges from dozens to a hundred or so lines.
    ///
    /// This works alongside the byte limit configuration. If both are set,
    /// the first-reached limit is used first. Both limits are dependent
    /// on external state (byte limit can be reached with less lines if
    /// more styles are used for example, line limit can be reached with
    /// a narrower terminal viewport). So, they are useful together.
    ///
    /// Lowering the limit immediately removes eligible complete historical
    /// pages. A `None` value pointer removes the line limit.
    pub fn set_scrollback_max_lines(&mut self, v: Option<usize>) -> Result<&mut Self> {
        self.set_optional(Opt::SCROLLBACK_MAX_LINES, v.as_ref())?;
        Ok(self)
    }

    /// Set whether a resize may pull rows out of scrollback back into the
    /// active area.
    ///
    /// When true, growing rows reveals scrollback if the cursor is on the
    /// bottom row, and a column reflow that needs fewer rows reveals
    /// scrollback as well. When false, growing rows always appends blank rows
    /// at the bottom and a column reflow keeps the top of the active area on
    /// the same content, so a line that is fully in scrollback stays there. A
    /// soft-wrapped line with at least one row still in the active area may
    /// still unwrap back into view.
    ///
    /// Set this to false when the pty keeps its own screen buffer without
    /// scrollback, since it cannot pull rows back and will otherwise disagree
    /// with the terminal about the screen contents after a resize. Windows
    /// `ConPTY` is the motivating case.
    ///
    /// This is preserved across a full reset (RIS).
    pub fn set_resize_pull_scrollback(&mut self, v: bool) -> Result<&mut Self> {
        // NOTE: There's no point in accepting an Option<bool> here
        // since the default is explicitly just `true`.
        self.set_optional(Opt::RESIZE_PULL_SCROLLBACK, Some(&v))?;
        Ok(self)
    }
}

/// Methods related to the cursor.
impl Terminal<'_> {
    /// Get the cursor column position (0-indexed).
    pub fn cursor_x(&self) -> Result<u16> {
        self.get(Data::CURSOR_X)
    }
    /// Get the cursor row position within the active area (0-indexed).
    pub fn cursor_y(&self) -> Result<u16> {
        self.get(Data::CURSOR_Y)
    }
    /// Whether the cursor is currently at a semantic shell prompt or input
    /// area.
    ///
    /// This depends on semantic prompt markers such as OSC 133. Returns false
    /// when semantic prompt information is unavailable or the alternate
    /// screen is active.
    pub fn is_cursor_at_prompt(&self) -> Result<bool> {
        self.get(Data::CURSOR_AT_PROMPT)
    }
    /// Get whether the cursor has a pending wrap (next print will soft-wrap).
    pub fn is_cursor_pending_wrap(&self) -> Result<bool> {
        self.get(Data::CURSOR_PENDING_WRAP)
    }
    /// Get whether the cursor is visible (DEC mode 25).
    pub fn is_cursor_visible(&self) -> Result<bool> {
        self.get(Data::CURSOR_VISIBLE)
    }
    /// Get the current SGR style of the cursor.
    ///
    /// This is the style that will be applied to newly printed characters.
    pub fn cursor_style(&self) -> Result<Style> {
        self.get::<ffi::Style>(Data::CURSOR_SGR_STYLE)
            .and_then(std::convert::TryInto::try_into)
    }

    /// Set the default cursor style used by DECSCUSR reset (CSI 0 q).
    ///
    /// Passing `None` resets to libghostty's built-in block cursor default.
    pub fn set_default_cursor_style(&mut self, v: Option<CursorStyle>) -> Result<&mut Self> {
        self.set_optional(Opt::DEFAULT_CURSOR_STYLE, v.as_ref())?;
        Ok(self)
    }

    /// Set whether the default cursor blinks when reset by DECSCUSR (CSI 0 q).
    ///
    /// Passing `None` resets to libghostty's built-in non-blinking default.
    pub fn set_default_cursor_blink(&mut self, v: Option<bool>) -> Result<&mut Self> {
        self.set_optional(Opt::DEFAULT_CURSOR_BLINK, v.as_ref())?;
        Ok(self)
    }
}

impl Drop for Terminal<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.inner.as_raw()) }
        // SAFETY: This terminal owns the allocation from Box::into_raw.
        // The native handle can no longer use its userdata. Borrowed callback
        // views are ManuallyDrop and never reach this destructor.
        // unsafe { drop(Box::from_raw(self.vtable)) }
    }
}

/// The shape of the cursor.
///
/// Programs change the cursor's shape with the DECSCUSR escape sequence
/// (CSI Ps SP q). Set the shape that DECSCUSR 0 restores with
/// [`Terminal::set_default_cursor_style`]. Read the shape to draw with
/// [`crate::render::Snapshot::cursor_visual_style`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum CursorStyle {
    /// Bar cursor (DECSCUSR 5, 6).
    Bar = ffi::CursorStyle::BAR,
    /// Block cursor (DECSCUSR 1, 2).
    Block = ffi::CursorStyle::BLOCK,
    /// Underline cursor (DECSCUSR 3, 4).
    Underline = ffi::CursorStyle::UNDERLINE,
    /// Hollow block cursor.
    BlockHollow = ffi::CursorStyle::BLOCK_HOLLOW,
}

/// Scroll viewport behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScrollViewport {
    /// Scroll to the top of the scrollback.
    Top,
    /// Scroll to the bottom (active area).
    Bottom,
    /// Scroll by a delta amount (up is negative).
    Delta(isize),
    /// Scroll to an absolute row offset from the top of the scrollback.
    Row(usize),
    /// Scroll by a number of prompts relative to the viewport top.
    ///
    /// Negative values move backward and positive values move forward.
    /// Requires semantic prompt markers from the shell. Zero or no matching
    /// prompt is a no-op. If fewer prompts remain than requested, move to
    /// the last matching prompt, clamped to the active area.
    ///
    /// Moving forward through a final prompt continuation can snap to the
    /// active area even when there is no newer prompt.
    DeltaPrompt(isize),
}
impl From<ScrollViewport> for ffi::TerminalScrollViewport {
    fn from(value: ScrollViewport) -> Self {
        match value {
            ScrollViewport::Top => Self {
                tag: ffi::TerminalScrollViewportTag::TOP,
                value: ffi::TerminalScrollViewportValue::default(),
            },
            ScrollViewport::Bottom => Self {
                tag: ffi::TerminalScrollViewportTag::BOTTOM,
                value: ffi::TerminalScrollViewportValue::default(),
            },
            ScrollViewport::Delta(delta) => Self {
                tag: ffi::TerminalScrollViewportTag::DELTA,
                value: {
                    let mut v = ffi::TerminalScrollViewportValue::default();
                    v.delta = delta;
                    v
                },
            },
            ScrollViewport::Row(row) => Self {
                tag: ffi::TerminalScrollViewportTag::ROW,
                value: {
                    let mut v = ffi::TerminalScrollViewportValue::default();
                    v.row = row;
                    v
                },
            },
            ScrollViewport::DeltaPrompt(delta_prompt) => Self {
                tag: ffi::TerminalScrollViewportTag::DELTA_PROMPT,
                value: {
                    let mut v = ffi::TerminalScrollViewportValue::default();
                    v.delta_prompt = delta_prompt;
                    v
                },
            },
        }
    }
}

bitflags::bitflags! {
    /// How the DECRQCRA checksum is calculated.
    ///
    /// These are the same bits as XTCHECKSUM (`CSI Ps # y`) and xterm's
    /// `checksumExtension` resource. The empty set is the calculation of a
    /// real DEC terminal.
    ///
    /// See [`Terminal::set_xt_checksum_extension`].
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct XtChecksumExtension: u8 {
        /// Don't negate the result.
        const NO_NEGATE = 1;
        /// Don't add the video attributes of each cell.
        const NO_ATTRIBUTES = 2;
        /// Don't omit blanks.
        const NO_OMIT_BLANKS = 4;
        /// Count cells that were never written to as spaces.
        const UNWRITTEN_AS_SPACES = 8;
        /// Use full codepoints instead of the DEC 8-bit values.
        const FULL_CODEPOINTS = 16;
    }
}

// bindgen turns `GhosttyTerminalBoundary` into `c_int` consts, while the
// call and `GhosttyTerminalWriteStop.reached` take the flags as `uint32_t`.
// Every flag is a small positive power of two, so the casts are lossless.
bitflags::bitflags! {
    /// A point in the VT stream where [`Terminal::vt_write_until`] can stop.
    ///
    /// The values are bit flags. Combine them with `|` to stop at any of
    /// several boundaries.
    ///
    /// Most boundaries mark the first or last byte of a control string: an
    /// APC, DCS or OSC sequence, made of an introducer, some content, and a
    /// terminator. [`GROUND`](Self::GROUND) is different. It marks the state
    /// between sequences rather than a byte inside one.
    ///
    /// Terminal protocols spell control bytes two ways, and both are
    /// recognized. A 7-bit introducer is ESC followed by one byte, such as
    /// `ESC ]` for OSC. The 8-bit form is a single byte in the range 0x80 to
    /// 0x9F, such as 0x9D for OSC. The string terminator ST is `ESC \` or the
    /// single byte 0x9C.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Boundary: u32 {
        /// The stream is not inside any escape sequence or partial UTF-8
        /// character. This is a safe point to insert your own sequences.
        ///
        /// Ground is a state, not an event. If the stream is already at
        /// ground, the call stops immediately and consumes nothing.
        const GROUND = ffi::TerminalBoundary::GROUND as u32;
        /// The byte that started an APC string: `_` after ESC, or 0x9F.
        ///
        /// SOS (`X` or 0x98) and PM (`^` or 0x9E) strings also report this
        /// boundary, because the parser handles all three the same way.
        /// Check [`WriteStop::byte`] to tell them apart.
        const APC_START = ffi::TerminalBoundary::APC_START as u32;
        /// The byte that ended an APC, SOS or PM string.
        const APC_END = ffi::TerminalBoundary::APC_END as u32;
        /// The byte that started a DCS string: `P` after ESC, or 0x90.
        ///
        /// A DCS has a header before its payload: parameters, intermediates
        /// and a final byte, in the same shape as a CSI sequence. The stop
        /// lands on the introducer, so the header follows it. A 0x90 inside
        /// the header restarts the header without leaving the string, so it
        /// does not stop.
        const DCS_START = ffi::TerminalBoundary::DCS_START as u32;
        /// The byte that ended a DCS string, whether it was still in its
        /// header or already in its payload. A DCS cut short in its header
        /// reports this without ever having had a payload.
        const DCS_END = ffi::TerminalBoundary::DCS_END as u32;
        /// The byte that started an OSC string: `]` after ESC, or 0x9D.
        const OSC_START = ffi::TerminalBoundary::OSC_START as u32;
        /// The byte that ended an OSC string. This is reported for every OSC,
        /// including ones the terminal does not understand.
        const OSC_END = ffi::TerminalBoundary::OSC_END as u32;
    }
}

/// The kind of control string that started or ended.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum StringKind {
    /// An APC, SOS or PM string. [`WriteStop::byte`] at the start of the
    /// string tells you which one it is:
    ///
    /// | Byte               | String |
    /// |--------------------|--------|
    /// | `_` (0x5F) or 0x9F | APC    |
    /// | `X` (0x58) or 0x98 | SOS    |
    /// | `^` (0x5E) or 0x9E | PM     |
    Apc = ffi::TerminalStringKind::APC,
    /// A DCS string, started by `P` (0x50) after ESC or by 0x90.
    Dcs = ffi::TerminalStringKind::DCS,
    /// An OSC string, started by `]` (0x5D) after ESC or by 0x9D.
    Osc = ffi::TerminalStringKind::OSC,
}

/// How a control string started.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum StringIntroducer {
    /// ESC followed by `_`, `X`, `^`, `P` or `]`. The stop's
    /// [`consumed`](WriteStop::consumed) count includes only the final byte,
    /// because the ESC may have arrived in an earlier call.
    Esc = ffi::TerminalStringIntroducer::ESC,
    /// A single 8-bit byte: 0x9F (APC), 0x98 (SOS), 0x9E (PM), 0x90 (DCS) or
    /// 0x9D (OSC).
    ///
    /// When the stream is at ground, these bytes are decoded as UTF-8 and do
    /// not start a string. They only start a string when they arrive while
    /// the stream is already inside another escape sequence.
    C1 = ffi::TerminalStringIntroducer::C1,
}

/// How a control string ended.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum StringEnd {
    /// ESC, the first byte of the string terminator `ESC \`.
    ///
    /// The ESC is consumed but the `\` after it is not. Pass the `\` as the
    /// first byte of your next write, where it completes the terminator. If
    /// you forward the stream elsewhere, forward the ESC along with the
    /// string.
    StEsc = ffi::TerminalStringEnd::ST_ESC,
    /// 0x9C, the 8-bit string terminator. The stream is now at ground.
    ///
    /// Inside an OSC, and inside a DCS once its header is complete, 0x9C is
    /// content rather than a terminator. So this is only reported for an APC
    /// or for a DCS still in its header.
    StC1 = ffi::TerminalStringEnd::ST_C1,
    /// CAN (0x18) or SUB (0x1A) cancelled the string. The stream is now at
    /// ground.
    ///
    /// The terminal discards a cancelled APC or OSC. Its command does not
    /// run and it is not passed to the unknown sequence callback. For
    /// example, `ESC _ G a=t,i=7;AAAA` followed by CAN stores no image.
    Cancelled = ffi::TerminalStringEnd::CANCELLED,
    /// Another 8-bit control byte cut the string short. Most of these return
    /// the stream to ground. CSI (0x9B) starts a CSI sequence instead. The
    /// introducers DCS (0x90), OSC (0x9D), APC (0x9F), SOS (0x98) and PM
    /// (0x9E) start a new string on the same byte, which the stop reports
    /// with a START flag as well. See [`WriteStop::reached`].
    ///
    /// Inside an OSC, and inside a DCS once its header is complete, 8-bit
    /// bytes are content. So this is only reported for an APC or for a DCS
    /// still in its header.
    ///
    /// The terminal discards an aborted APC, just like a cancelled one.
    Aborted = ffi::TerminalStringEnd::ABORTED,
    /// BEL (0x07) terminated the string. Only an OSC ends this way.
    Bel = ffi::TerminalStringEnd::BEL,
}

/// Where [`Terminal::vt_write_until`] stopped, and why.
///
/// Upstream leaves the fields that don't apply to a stop zeroed, and zero is
/// also a valid value for `kind`, `introducer` and `end`. Those fields are
/// therefore `Option`s here, and are `None` whenever [`reached`](Self::reached)
/// says they don't apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WriteStop {
    /// Number of input bytes consumed, including the byte that reached the
    /// boundary. This is 0 when the stream was already at ground and you
    /// asked for [`Boundary::GROUND`].
    pub consumed: usize,
    /// The [`Boundary`] flags that the stopping byte reached. Only flags you
    /// asked for are set.
    ///
    /// This is usually one flag. It holds two when a single byte does two
    /// things:
    ///
    /// - It ends a string and returns to ground. BEL, CAN, SUB, 0x9C and the
    ///   other 8-bit control bytes all do this. So if you asked for both
    ///   [`Boundary::OSC_END`] and [`Boundary::GROUND`], an `OSC_END` stop
    ///   can include `GROUND` too.
    /// - It ends one string and starts another. An 8-bit introducer of a
    ///   different kind does this inside an APC, or inside a DCS header: 0x90
    ///   or 0x9D in an APC ends it and starts a DCS or OSC, and 0x9F, 0x98,
    ///   0x9E or 0x9D in a DCS header ends it and starts an APC or OSC. The
    ///   stop has an END flag and a START flag, [`end`](Self::end) is
    ///   [`StringEnd::Aborted`] and [`introducer`](Self::introducer) is
    ///   [`StringIntroducer::C1`]. See [`Terminal::vt_write_until`] for an
    ///   example.
    ///
    /// A START flag and an END flag of the same kind are never set together.
    pub reached: Boundary,
    /// The kind of string. `Some` when [`reached`](Self::reached) has a
    /// START or END flag.
    ///
    /// When one byte both ends a string and starts another, this is the kind
    /// of the string that ended. The START flag in `reached` names the kind
    /// that started, since `APC_START`, `DCS_START` and `OSC_START` each
    /// belong to one kind.
    pub kind: Option<StringKind>,
    /// How the string started. `Some` when [`reached`](Self::reached) has a
    /// START flag.
    pub introducer: Option<StringIntroducer>,
    /// How the string ended. `Some` when [`reached`](Self::reached) has an
    /// END flag.
    pub end: Option<StringEnd>,
    /// The byte that started or ended the string, such as `]`, 0x90, ESC or
    /// BEL. `Some` when [`reached`](Self::reached) has a START or END flag.
    pub byte: Option<u8>,
}

/// Amount of compression work to perform before returning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum CompressionMode {
    /// Perform one bounded compression step suitable for idle scheduling.
    Incremental = ffi::TerminalCompressionMode::INCREMENTAL,
    /// Synchronously inspect every currently eligible page.
    Full = ffi::TerminalCompressionMode::FULL,
}

/// Scheduling result from terminal compression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum CompressionResult {
    /// Retained-mapping reclamation is unavailable on this target.
    Unsupported = ffi::TerminalCompressionResult::UNSUPPORTED,
    /// More incremental compression work remains.
    Pending = ffi::TerminalCompressionResult::PENDING,
    /// The pass has no continuation to schedule.
    Complete = ffi::TerminalCompressionResult::COMPLETE,
}

/// Opaque token representing a terminal's current compression activity.
///
/// The token is opaque and only equality comparisons are meaningful.
/// An embedding application should cache it and restart its compression idle
/// delay whenever the value changes. The value may wrap and changes in either
/// direction have the same meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressionActivity(u64);

/// Memory held by a terminal, read with [`Terminal::memory_usage`].
///
/// Each screen has its own set of fields in the form of [`ScreenMemoryUsage`].
/// Add the two sets together for the terminal's total. The alternate fields
/// are all zero until a program first switches to that screen.
///
/// See [Memory use] for what these figures cover and which one to budget against.
#[doc = crate::guide_ref!("Memory use", "scrollback", "memory")]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct MemoryUsage {
    /// Whether compressing scrollback can free memory on this platform.
    ///
    /// When false, [`Terminal::compress`] reports [`CompressionResult::Unsupported`]
    /// and the compressed fields are always zero. To reduce a terminal's memory
    /// on such a platform, you have to do something else, such as closing it.
    pub compression_supported: bool,
    /// Memory usage of the primary screen.
    pub primary: ScreenMemoryUsage,
    /// Memory usage of the alternate screen.
    pub alternate: ScreenMemoryUsage,
}

impl MemoryUsage {
    fn from_raw(raw: ffi::TerminalMemoryUsage) -> Result<Self> {
        let minimum_size = std::mem::offset_of!(ffi::TerminalMemoryUsage, alternate_image_bytes)
            + std::mem::size_of::<u64>();
        if raw.size < minimum_size {
            return Err(Error::InvalidValue);
        }

        Ok(Self {
            compression_supported: raw.compression_supported,
            primary: ScreenMemoryUsage {
                pages: raw.primary_pages,
                virtual_bytes: raw.primary_virtual_bytes,
                resident_bytes: raw.primary_resident_bytes,
                compressed_pages: raw.primary_compressed_pages,
                compressed_bytes: raw.primary_compressed_bytes,
                image_bytes: raw.primary_image_bytes,
            },
            alternate: ScreenMemoryUsage {
                pages: raw.alternate_pages,
                virtual_bytes: raw.alternate_virtual_bytes,
                resident_bytes: raw.alternate_resident_bytes,
                compressed_pages: raw.alternate_compressed_pages,
                compressed_bytes: raw.alternate_compressed_bytes,
                image_bytes: raw.alternate_image_bytes,
            },
        })
    }
}

/// Memory held by a single screen, either primary or alternate.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ScreenMemoryUsage {
    /// Number of pages in the screen, including compressed pages.
    pub pages: u64,

    /// Bytes of address space reserved for the primary screen's pages.
    ///
    /// This includes compressed pages and spare pages kept ready for reuse.
    /// Always at least [`Self::resident_bytes`].
    pub virtual_bytes: u64,

    /// Bytes of physical memory used by the primary screen's pages.
    ///
    /// A compressed page counts only its compressed size. Use this figure for
    /// memory budgets.
    pub resident_bytes: u64,

    /// Number of the primary screen's pages that are compressed.
    pub compressed_pages: u64,

    /// Bytes of compressed data held for the primary screen's compressed pages.
    ///
    /// This is already included in [`Self::resident_bytes`].
    pub compressed_bytes: u64,

    /// Bytes of image data stored for the primary screen through the Kitty graphics protocol. This is not included in primary_resident_bytes. Always zero when libghostty-vt is built without Kitty graphics.
    pub image_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderState;
    use crate::effect::MockPty;
    use crate::grid_ref::TrackedGridRef;
    use crate::point::{self, Point};
    use crate::terminal::CursorStyle;

    #[test]
    fn scroll_viewport_delta_prompt() {
        // Same transcript as upstream's `scroll_viewport delta_prompt` test:
        // prompts start on rows 0, 3, 6 and 9 of a 10-row history, and with
        // 2 visible rows the active area starts at row 8.
        let mut terminal = Terminal::new(80, 2).expect("terminal should initialize");
        terminal.vt_write(
            b"\x1b]133;P;k=i\x07p1\r\n\
              \x1b]133;B\x07cmd1\r\n\
              \x1b]133;C\x07out1\r\n\
              \x1b]133;P;k=i\x07p2\r\n\
              \x1b]133;B\x07cmd2\r\n\
              \x1b]133;C\x07out2\r\n\
              \x1b]133;P;k=i\x07p3\r\n\
              \x1b]133;B\x07cmd3\r\n\
              \x1b]133;C\x07out3\r\n\
              \x1b]133;P;k=i\x07p4",
        );
        let mut scroll = |scroll| {
            terminal
                .scroll_viewport(scroll)
                .expect("scroll should work");
            terminal.scrollbar().expect("scrollbar should exist").offset
        };

        // First, upstream's exact sequence.
        //
        // Counting is relative to the viewport top (row 8), so going backward
        // from the active area lands on p3, then p2. Forward returns to p3.
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(-1)), 6);
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(-1)), 3);
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(1)), 6);

        // Over-asking stops at the last matching prompt: p1 going backward
        // (from p3, so this walks past p2), and p4 going forward, which lies
        // in the active area and so is clamped to it. Zero is then a no-op.
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(isize::MIN)), 0);
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(isize::MAX)), 8);
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(0)), 8);

        // Extra steps beyond upstream's test: step back from the active area
        // to p3, check zero is a no-op away from the active area too, then
        // over-ask with ordinary counts in both directions.
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(-1)), 6);
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(0)), 6);
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(-100)), 0);
        assert_eq!(scroll(ScrollViewport::DeltaPrompt(100)), 8);
    }

    /// Feed `input` to a terminal configured by `setup`, returning what it
    /// wrote back to the pty.
    fn pty_output(setup: impl FnOnce(&mut Terminal<'_>), input: &[u8]) -> Vec<u8> {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .register_effects(MockPty::default())
            .expect("effects should be registered")
            .with_on_pty_write()
            .expect("pty_write should be registered");
        setup(&mut terminal);
        terminal.vt_write(input);
        terminal.effect_handler::<MockPty>().unwrap().pty.clone()
    }

    /// A ground stop carries no string, so every string field is `None`.
    fn ground_stop(consumed: usize) -> WriteStop {
        WriteStop {
            consumed,
            reached: Boundary::GROUND,
            kind: None,
            introducer: None,
            end: None,
            byte: None,
        }
    }

    #[test]
    fn vt_write_until_stops_at_ground() {
        let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
        // Already at ground: ground is a state, so the call stops right away
        // and consumes nothing. Upstream zero-fills `kind`, `introducer` and
        // `end` here, which must not be read as APC / ESC / ST_ESC.
        assert!(terminal.is_vt_ground().unwrap());
        assert_eq!(
            terminal.vt_write_until(b"hello", Boundary::GROUND).unwrap(),
            Some(ground_stop(0))
        );
        assert_eq!(terminal.cursor_x().unwrap(), 0);

        // An incomplete CSI sequence is only finished, not followed.
        terminal.vt_write(b"\x1b[");
        assert!(!terminal.is_vt_ground().unwrap());
        assert_eq!(
            terminal.vt_write_until(b"31", Boundary::GROUND).unwrap(),
            None
        );
        // The count includes the byte that reaches ground.
        assert_eq!(
            terminal
                .vt_write_until(b"mhello", Boundary::GROUND)
                .unwrap(),
            Some(ground_stop(1))
        );
        assert!(terminal.is_vt_ground().unwrap());
        assert_eq!(terminal.cursor_x().unwrap(), 0);

        // The same applies to an incomplete UTF-8 sequence ("€" is E2 82 AC).
        terminal.vt_write(b"\xe2");
        assert!(!terminal.is_vt_ground().unwrap());
        assert_eq!(
            terminal
                .vt_write_until(b"\x82\xac!", Boundary::GROUND)
                .unwrap(),
            Some(ground_stop(2))
        );
        assert_eq!(terminal.cursor_x().unwrap(), 1);
    }

    #[test]
    fn vt_write_until_finds_esc_apc() {
        // Mirrors upstream's "vt_write_until kitty apc start and end" test,
        // with plain text around the APC.
        let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
        let input = b"ab\x1b_Gi=1\x1b\\cd";
        let want = Boundary::APC_START | Boundary::APC_END;

        // The start stop covers `a`, `b`, ESC and `_`.
        let start = terminal.vt_write_until(input, want).unwrap();
        assert_eq!(
            start,
            Some(WriteStop {
                consumed: 4,
                reached: Boundary::APC_START,
                kind: Some(StringKind::Apc),
                introducer: Some(StringIntroducer::Esc),
                end: None,
                byte: Some(b'_'),
            })
        );
        let mut off = 4;
        assert_eq!(input[off], b'G');

        // The end stop consumes the content and the ESC of `ESC \`, but not
        // the backslash.
        let end = terminal.vt_write_until(&input[off..], want).unwrap();
        assert_eq!(
            end,
            Some(WriteStop {
                consumed: b"Gi=1".len() + 1,
                reached: Boundary::APC_END,
                kind: Some(StringKind::Apc),
                introducer: None,
                end: Some(StringEnd::StEsc),
                byte: Some(0x1b),
            })
        );
        off += b"Gi=1".len() + 1;
        assert_eq!(&input[off..], b"\\cd");

        // The backslash completes the terminator and the rest is plain text,
        // so no boundary is left to reach.
        assert_eq!(terminal.vt_write_until(&input[off..], want).unwrap(), None);
        assert!(terminal.is_vt_ground().unwrap());
        // Only `ab` and `cd` were printed.
        assert_eq!(terminal.cursor_x().unwrap(), 4);
    }

    /// Mirrors the header's DCS example: the start stop lands on `P`, so the
    /// `1;2|` header follows it, and the payload runs up to the ESC of ST.
    #[test]
    fn vt_write_until_finds_esc_dcs() {
        let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
        let input = b"\x1bP1;2|data\x1b\\";
        let want = Boundary::DCS_START | Boundary::DCS_END;

        let start = terminal.vt_write_until(input, want).unwrap();
        assert_eq!(
            start,
            Some(WriteStop {
                consumed: 2,
                reached: Boundary::DCS_START,
                kind: Some(StringKind::Dcs),
                introducer: Some(StringIntroducer::Esc),
                end: None,
                byte: Some(b'P'),
            })
        );

        // The header and payload, then the ESC of `ESC \`.
        let end = terminal.vt_write_until(&input[2..], want).unwrap();
        assert_eq!(
            end,
            Some(WriteStop {
                consumed: b"1;2|data".len() + 1,
                reached: Boundary::DCS_END,
                kind: Some(StringKind::Dcs),
                introducer: None,
                end: Some(StringEnd::StEsc),
                byte: Some(0x1b),
            })
        );
    }

    #[test]
    fn vt_write_until_osc_bel_end_reaches_ground() {
        // Mirrors the BEL half of upstream's "vt_write_until osc start and
        // bel end" test.
        let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
        terminal.vt_write(b"\x1b]");

        // BEL ends the string and reaches ground on the same byte, so both
        // requested flags are set.
        let stop = terminal
            .vt_write_until(b"2;title\x07x", Boundary::OSC_END | Boundary::GROUND)
            .unwrap();
        assert_eq!(
            stop,
            Some(WriteStop {
                consumed: b"2;title".len() + 1,
                reached: Boundary::OSC_END | Boundary::GROUND,
                kind: Some(StringKind::Osc),
                introducer: None,
                end: Some(StringEnd::Bel),
                byte: Some(0x07),
            })
        );
        assert!(terminal.is_vt_ground().unwrap());
        assert_eq!(terminal.title().unwrap(), b"title");
    }

    #[test]
    fn vt_write_until_c1_ends_apc_and_starts_osc() {
        // The same byte ends an APC and starts an OSC. Only requested
        // metadata is exposed; with both flags, the ended string's kind wins.
        let input = b"\x1b_a\x9d2;t\x07";
        for (boundaries, kind, introducer, end) in [
            (
                Boundary::APC_END | Boundary::OSC_START,
                StringKind::Apc,
                Some(StringIntroducer::C1),
                Some(StringEnd::Aborted),
            ),
            (
                Boundary::OSC_START,
                StringKind::Osc,
                Some(StringIntroducer::C1),
                None,
            ),
            (
                Boundary::APC_END,
                StringKind::Apc,
                None,
                Some(StringEnd::Aborted),
            ),
        ] {
            let mut terminal = Terminal::new(8, 2).unwrap();
            assert_eq!(
                terminal.vt_write_until(input, boundaries).unwrap(),
                Some(WriteStop {
                    consumed: 4,
                    reached: boundaries,
                    kind: Some(kind),
                    introducer,
                    end,
                    byte: Some(0x9d),
                }),
                "{boundaries:?}"
            );
            // BEL ends the OSC, but asking only for GROUND must not expose
            // string metadata, even though the C enum fields contain zeros.
            assert_eq!(
                terminal
                    .vt_write_until(&input[4..], Boundary::GROUND)
                    .unwrap(),
                Some(WriteStop {
                    consumed: 4,
                    reached: Boundary::GROUND,
                    kind: None,
                    introducer: None,
                    end: None,
                    byte: None,
                })
            );
            assert_eq!(terminal.title().unwrap(), b"t");
        }
    }

    #[test]
    fn cursor_at_prompt_follows_semantic_prompts() {
        let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
        assert!(!terminal.is_cursor_at_prompt().unwrap());
        terminal.vt_write(b"\x1b]133;A\x1b\\");
        assert!(terminal.is_cursor_at_prompt().unwrap());
        // The alternate screen never counts as a prompt.
        terminal.vt_write(b"\x1b[?1049h");
        assert!(!terminal.is_cursor_at_prompt().unwrap());
    }

    #[test]
    fn memory_usage_reports_each_screen() {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");

        // Upstream: the alternate screen's fields "are all zero until a
        // program first switches to it".
        let fresh = terminal.memory_usage().expect("memory usage should read");
        assert_eq!(fresh.alternate.pages, 0);
        assert_eq!(fresh.alternate.virtual_bytes, 0);
        assert_eq!(fresh.alternate.resident_bytes, 0);
        assert_eq!(fresh.alternate.compressed_pages, 0);
        assert_eq!(fresh.alternate.compressed_bytes, 0);
        assert_eq!(fresh.alternate.image_bytes, 0);

        // The primary screen always has its visible rows in at least one page.
        assert!(fresh.primary.pages >= 1);
        assert!(fresh.primary.resident_bytes > 0);
        // Upstream: virtual bytes are "always at least
        // `primary_resident_bytes`".
        assert!(fresh.primary.virtual_bytes >= fresh.primary.resident_bytes);
        // Upstream: compressed bytes are "already included in
        // `primary_resident_bytes`". That holds whether or not this platform
        // supports compression (the compressed fields are zero if it doesn't).
        assert!(fresh.primary.compressed_bytes <= fresh.primary.resident_bytes);

        // Once a program switches to the alternate screen, it gets pages.
        terminal.vt_write(b"\x1b[?1049h");
        let alternate = terminal.memory_usage().expect("memory usage should read");
        assert!(alternate.alternate.pages >= 1);
    }

    #[test]
    fn terminfo_name_answers_xtgettcap() {
        // XTGETTCAP query for "TN" (hex 544e).
        let query = b"\x1bP+q544e\x1b\\";
        // Unset names are not reported at all.
        assert_eq!(pty_output(|_| {}, query), b"");
        assert_eq!(
            pty_output(
                |terminal| {
                    terminal.set_terminfo_name("xterm-256color").unwrap();
                },
                query
            ),
            // "xterm-256color" in hex.
            b"\x1bP1+r544E=787465726D2D323536636F6C6F72\x1b\\"
        );
        // Upstream doesn't require UTF-8: the bytes are hex-encoded as-is.
        assert_eq!(
            pty_output(
                |terminal| {
                    terminal.set_terminfo_name(b"x\xff").unwrap();
                },
                query
            ),
            b"\x1bP1+r544E=78FF\x1b\\"
        );
        // An empty name clears it again.
        assert_eq!(
            pty_output(
                |terminal| {
                    terminal.set_terminfo_name("xterm-256color").unwrap();
                    terminal.set_terminfo_name("").unwrap();
                },
                query
            ),
            b""
        );

        let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
        assert!(matches!(
            terminal.set_terminfo_name("x".repeat(129)),
            Err(Error::InvalidValue)
        ));
    }

    #[test]
    fn cursor_style_preserves_independent_attributes() {
        use crate::{
            color::{PaletteIndex, Rgb},
            style::{Style, StyleColor, Underline},
        };

        let mut terminal = Terminal::new(80, 24).unwrap();
        let default = Style::default();
        assert!(default.is_default());

        // Complementary flag sets catch swapped bits; a curly underline
        // exercises more than one bit of the packed underline field.
        terminal.vt_write(b"\x1b[1;3;7;9;4:3;38;2;17;83;201;48;5;42m");
        let expected = Style {
            bold: true,
            italic: true,
            inverse: true,
            strikethrough: true,
            underline: Underline::Curly,
            fg_color: StyleColor::Rgb(Rgb {
                r: 17,
                g: 83,
                b: 201,
            }),
            bg_color: StyleColor::Palette(PaletteIndex(42)),
            ..default
        };
        assert_eq!(terminal.cursor_style().unwrap(), expected);
        assert!(!expected.is_default());

        terminal.vt_write(b"\x1b[0;2;5;8;53;4:5m");
        assert_eq!(
            terminal.cursor_style().unwrap(),
            Style {
                faint: true,
                blink: true,
                invisible: true,
                overline: true,
                underline: Underline::Dashed,
                ..default
            }
        );
        terminal.vt_write(b"\x1b[0m");
        assert_eq!(terminal.cursor_style().unwrap(), default);
    }

    #[test]
    fn default_cursor_reset_uses_configured_style_and_blink() {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        let mut render_state = RenderState::new().expect("render state should initialize");

        terminal
            .set_default_cursor_style(Some(CursorStyle::Underline))
            .expect("default cursor style should update")
            .set_default_cursor_blink(Some(true))
            .expect("default cursor blink should update");

        terminal.vt_write(b"\x1b[0 q");
        let snapshot = render_state
            .update(&terminal)
            .expect("render state should update");

        assert_eq!(
            snapshot
                .cursor_visual_style()
                .expect("cursor style should be readable"),
            CursorStyle::Underline
        );
        assert!(
            snapshot
                .cursor_blinking()
                .expect("cursor blink should be readable")
        );
    }

    #[test]
    fn glyph_protocol_enabled_setting_updates() {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");

        terminal
            .set_glyph_protocol_enabled(false)
            .expect("glyph protocol should disable")
            .set_glyph_protocol_enabled(true)
            .expect("glyph protocol should enable");
    }
    // The next test reaches simdutf, which libghostty-vt DLLs on Windows
    // used to crash in, since their C++ global constructors never ran. Pure
    // ASCII never reaches it.
    #[test]
    fn multibyte_utf8_is_decoded() {
        use crate::point::{Coordinate, Point};
        let mut terminal = Terminal::new(8, 3).unwrap();
        // "é" is 0xC3 0xA9. Both bytes must arrive in one write: a character
        // split across writes is finished by the scalar decoder, and never
        // reaches simdutf.
        terminal.vt_write(b"\xc3\xa9");
        let codepoint = terminal
            .grid_ref(Point::Active(Coordinate { x: 0, y: 0 }))
            .unwrap()
            .cell()
            .unwrap()
            .codepoint()
            .unwrap();
        assert_eq!(codepoint, '\u{e9}');
    }

    #[test]
    fn mouse_shape_follows_osc_22() {
        let mut terminal = Terminal::new(8, 3).unwrap();
        assert_eq!(terminal.mouse_shape().unwrap(), mouse::Shape::Text);
        // OSC 22 names the shape with its W3C cursor name.
        terminal.vt_write(b"\x1b]22;pointer\x07");
        assert_eq!(terminal.mouse_shape().unwrap(), mouse::Shape::Pointer);
        terminal.vt_write(b"\x1b]22;nwse-resize\x1b\\");
        assert_eq!(terminal.mouse_shape().unwrap(), mouse::Shape::NwseResize);
        // A name libghostty doesn't know leaves the shape alone.
        terminal.vt_write(b"\x1b]22;not-a-shape\x07");
        assert_eq!(terminal.mouse_shape().unwrap(), mouse::Shape::NwseResize);
        // An empty name gives the pointer back.
        terminal.vt_write(b"\x1b]22;\x1b\\");
        assert_eq!(terminal.mouse_shape().unwrap(), mouse::Shape::Text);
    }

    fn tiny_terminal() -> Terminal<'static> {
        Terminal::new(8, 3).expect("terminal should initialize")
    }

    fn codepoint_at_tracked_ref(terminal: &Terminal<'_>, tracked: &TrackedGridRef) -> char {
        let snapshot = tracked
            .snapshot(terminal)
            .expect("tracked snapshot should not fail")
            .expect("tracked ref should have a value");
        snapshot
            .cell()
            .expect("tracked snapshot should resolve to a cell")
            .codepoint()
            .expect("tracked snapshot cell should expose a codepoint")
    }

    #[test]
    fn tracked_grid_ref_follows_scroll() {
        let mut terminal = tiny_terminal();
        terminal.vt_write(b"alpha\r\nbravo\r\ncharlie");

        let tracked = terminal
            .track_grid_ref(Point::Active(point::Coordinate { x: 0, y: 0 }))
            .expect("tracked grid ref should initialize");

        terminal.vt_write(b"\r\ndelta");

        assert!(tracked.has_value());
        assert_eq!(codepoint_at_tracked_ref(&terminal, &tracked), 'a');
        assert_eq!(
            tracked
                .point(point::Space::Screen)
                .expect("tracked point should resolve")
                .expect("tracked point should have a value")
                .x,
            0
        );
    }

    #[test]
    fn tracked_grid_ref_reports_loss_and_can_set_point() {
        let mut terminal = tiny_terminal();
        terminal.vt_write(b"alpha\r\nbravo\r\ncharlie");

        let mut tracked = terminal
            .track_grid_ref(Point::Active(point::Coordinate { x: 0, y: 0 }))
            .expect("tracked grid ref should initialize");

        terminal.reset();

        assert!(!tracked.has_value());
        assert!(
            tracked
                .snapshot(&terminal)
                .expect("missing tracked snapshot should not fail")
                .is_none()
        );
        assert!(
            tracked
                .point(point::Space::Screen)
                .expect("missing tracked point should not fail")
                .is_none()
        );

        terminal.vt_write(b"echo");
        tracked
            .set(
                &mut terminal,
                Point::Active(point::Coordinate { x: 0, y: 0 }),
            )
            .expect("tracked grid ref should set to a new point");

        assert!(tracked.has_value());
        assert_eq!(codepoint_at_tracked_ref(&terminal, &tracked), 'e');
    }

    #[test]
    fn tracked_grid_ref_survives_terminal_drop() {
        let tracked = {
            let mut terminal = tiny_terminal();
            terminal.vt_write(b"alpha");
            terminal
                .track_grid_ref(Point::Active(point::Coordinate { x: 0, y: 0 }))
                .expect("tracked grid ref should initialize")
        };

        assert!(!tracked.has_value());
        assert!(
            tracked
                .point(point::Space::Screen)
                .expect("detached tracked point should not fail")
                .is_none()
        );
    }

    #[test]
    fn tracked_grid_ref_rejects_different_terminal() {
        let mut first = tiny_terminal();
        first.vt_write(b"alpha");
        let mut second = tiny_terminal();
        second.vt_write(b"bravo");

        let mut tracked = first
            .track_grid_ref(Point::Active(point::Coordinate { x: 0, y: 0 }))
            .expect("tracked grid ref should initialize");

        assert!(matches!(
            tracked.snapshot(&second),
            Err(Error::InvalidValue)
        ));
        assert!(matches!(
            tracked.set(&mut second, Point::Active(point::Coordinate { x: 0, y: 0 })),
            Err(Error::InvalidValue)
        ));
    }

    #[test]
    fn grid_ref_converts_back_to_point() {
        let mut terminal = tiny_terminal();
        terminal.vt_write(b"alpha");

        let original = point::Coordinate { x: 1, y: 0 };
        let grid_ref = terminal
            .grid_ref(Point::Active(original))
            .expect("grid ref should resolve");

        assert_eq!(
            terminal
                .point_from_grid_ref(grid_ref, point::Space::Active)
                .expect("grid ref point conversion should not fail")
                .expect("grid ref should be representable in active space"),
            original
        );
    }

    /// Bold `A`, then `B`, a space and `C` on the first row.
    ///
    /// Expected checksums, worked out from upstream's `xt_checksum.compute`:
    /// bold `A` is 0x41 + 0x80 (bold attribute) = 0xC1, `B` is 0x42 and `C`
    /// is 0x43. The DEC calculation omits the space because it isn't the
    /// first cell, so the sum is 0x146, negated to 0xFEBA.
    const CHECKSUM_SCREEN: &[u8] = b"\x1b[1mA\x1b[0mB C";

    /// DECRQCRA with id 7 over row 1, columns 1 through 4.
    const DECRQCRA: &[u8] = b"\x1b[7;1;1;1;1;4*y";

    /// Write a DECRQCRA and return whatever the terminal replied.
    fn request_checksum(terminal: &mut Terminal<'_>) -> Vec<u8> {
        let handler = terminal.effect_handler_mut::<MockPty>().unwrap();
        handler.pty.clear();
        terminal.vt_write(DECRQCRA);

        let handler = terminal.effect_handler::<MockPty>().unwrap();
        handler.pty.clone()
    }

    #[test]
    fn xt_checksum_report_requires_opt_in() {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .register_effects(MockPty::default())
            .expect("effects should be registered")
            .with_on_pty_write()
            .expect("pty_write should be registered");
        terminal.vt_write(CHECKSUM_SCREEN);

        // Disabled by default.
        assert_eq!(request_checksum(&mut terminal), b"");

        terminal
            .set_xt_checksum_report_enabled(true)
            .expect("enabling checksum reports should succeed");
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~FEBA\x1b\\");

        terminal
            .set_xt_checksum_report_enabled(false)
            .expect("disabling checksum reports should succeed");
        assert_eq!(request_checksum(&mut terminal), b"");
    }

    #[test]
    fn xt_checksum_xtchecksum_ignored_while_disabled() {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .register_effects(MockPty::default())
            .expect("effects should be registered")
            .with_on_pty_write()
            .expect("pty_write should be registered");
        terminal.vt_write(CHECKSUM_SCREEN);

        // XTCHECKSUM 1 (don't negate) while reporting is off is dropped, so
        // the reply after enabling is still the negated DEC checksum.
        terminal.vt_write(b"\x1b[1#y");
        terminal
            .set_xt_checksum_report_enabled(true)
            .expect("enabling checksum reports should succeed");
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~FEBA\x1b\\");

        // Once enabled, the same XTCHECKSUM takes effect: 0x146 unnegated.
        terminal.vt_write(b"\x1b[1#y");
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~0146\x1b\\");
    }

    #[test]
    fn xt_checksum_extension_changes_calculation() {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .register_effects(MockPty::default())
            .expect("effects should be registered")
            .with_on_pty_write()
            .expect("pty_write should be registered");
        terminal.vt_write(CHECKSUM_SCREEN);
        terminal
            .set_xt_checksum_report_enabled(true)
            .expect("enabling checksum reports should succeed");

        // Without attributes: 0x41 + 0x42 + 0x43 = 0xC6, negated to 0xFF3A.
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::NO_ATTRIBUTES)
            .expect("valid extension should be accepted");
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~FF3A\x1b\\");

        // Keeping blanks adds the space: 0x146 + 0x20 = 0x166, negated to
        // 0xFE9A.
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::NO_OMIT_BLANKS)
            .expect("valid extension should be accepted");
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~FE9A\x1b\\");

        // The extension also applies after a full reset (RIS), which clears
        // the screen, so rewrite it.
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::NO_NEGATE)
            .expect("valid extension should be accepted");
        terminal.vt_write(b"\x1bc");
        terminal.vt_write(CHECKSUM_SCREEN);
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~0146\x1b\\");

        // The empty set restores the DEC calculation.
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::empty())
            .expect("empty extension should be accepted");
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~FEBA\x1b\\");

        // Bits above 31 are rejected and leave the calculation unchanged.
        let result = terminal.set_xt_checksum_extension(XtChecksumExtension::from_bits_retain(32));
        assert!(matches!(result, Err(Error::InvalidValue)));
        assert_eq!(request_checksum(&mut terminal), b"\x1bP7!~FEBA\x1b\\");
    }

    #[test]
    fn xt_checksum_extension_unwritten_and_full_codepoints() {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .register_effects(MockPty::default())
            .expect("effects should be registered")
            .with_on_pty_write()
            .expect("pty_write should be registered");
        terminal
            .set_xt_checksum_report_enabled(true)
            .expect("enabling checksum reports should succeed");

        // Row 2: column 1 is U+1D400 MATHEMATICAL BOLD CAPITAL A, written
        // without attributes.
        // Row 1: column 1 is never written, column 2 is a bold `X`. Bold is
        // left on so we'd notice it leaking into the unwritten cell.
        terminal.vt_write("\x1b[2;1H\u{1D400}\x1b[1;2H\x1b[1mX".as_bytes());
        let request = |terminal: &mut Terminal<'_>, req: &[u8]| {
            let handler = terminal.effect_handler_mut::<MockPty>().unwrap();
            handler.pty.clear();
            terminal.vt_write(req);
            let handler = terminal.effect_handler::<MockPty>().unwrap();
            handler.pty.clone()
        };

        // Row 1, columns 1 through 2. Bold `X` is 0x58 + 0x80 = 0xD8.
        // DEC: the unwritten cell is skipped entirely, so `X` is the only
        // cell: 0xD8, negated to 0xFF28.
        let row1 = b"\x1b[7;1;1;1;1;2*y";
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::empty())
            .expect("valid extension should be accepted");
        assert_eq!(request(&mut terminal, row1), b"\x1bP7!~FF28\x1b\\");
        // Unwritten as spaces: the unwritten cell is a space with no style
        // (bold only applies to cells that get printed) and, being the first
        // cell, isn't trimmed: 0x20 + 0xD8 = 0xF8, negated to 0xFF08.
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::UNWRITTEN_AS_SPACES)
            .expect("valid extension should be accepted");
        assert_eq!(request(&mut terminal, row1), b"\x1bP7!~FF08\x1b\\");

        // Row 2, column 1.
        // DEC: anything outside the 8-bit ranges counts as ESC: 0x1B, negated
        // to 0xFFE5.
        let row2 = b"\x1b[7;1;2;1;2;1*y";
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::empty())
            .expect("valid extension should be accepted");
        assert_eq!(request(&mut terminal, row2), b"\x1bP7!~FFE5\x1b\\");
        // Full codepoints: 0x1D400 truncated to 16 bits is 0xD400, negated to
        // 0x2C00.
        terminal
            .set_xt_checksum_extension(XtChecksumExtension::FULL_CODEPOINTS)
            .expect("valid extension should be accepted");
        assert_eq!(request(&mut terminal, row2), b"\x1bP7!~2C00\x1b\\");
    }
}

/// Soundness regression tests for
/// <https://github.com/Uzaaft/libghostty-rs/issues/74>.
///
/// These tests are gated on `cfg(miri)` because they construct the exact
/// shapes the C API produces and feed them into the safe wrappers, which was
/// UB before the wrappers stopped building slices and `&str` from unvalidated
/// FFI input. The module also holds tests that need the mock C API to produce
/// shapes real libghostty can't yet (e.g. unknown upstream tags). Run with:
///
/// ```sh
/// cargo +nightly miri test -p libghostty-vt miri_soundness
/// ```
#[cfg(all(test, miri))]
mod miri_soundness {
    use super::*;

    // Miri cannot run Zig. These shims implement only native handle ownership
    // and synchronous callback dispatch; the constructor, registration,
    // trampolines and destructor under test are the actual Rust wrapper.
    struct MockTerminal {
        userdata: *mut std::ffi::c_void,
        write: ffi::TerminalWritePtyFn,
        read: ffi::TerminalClipboardReadFn,
        resize: ffi::TerminalResizeWindowFn,
        unknown: ffi::TerminalUnknownSequenceFn,
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_terminal_new(
        _: *const ffi::Allocator,
        out: *mut ffi::Terminal,
        _: u16,
        _: u16,
    ) -> ffi::Result::Type {
        let terminal = Box::new(MockTerminal {
            userdata: std::ptr::null_mut(),
            write: None,
            read: None,
            resize: None,
            unknown: None,
        });
        // SAFETY: The constructor supplies a writable out parameter.
        unsafe { *out = Box::into_raw(terminal).cast() };
        ffi::Result::SUCCESS
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_terminal_free(terminal: ffi::Terminal) {
        // SAFETY: The wrapper frees the handle from new exactly once.
        unsafe { drop(Box::from_raw(terminal.cast::<MockTerminal>())) };
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_terminal_set(
        terminal: ffi::Terminal,
        option: ffi::TerminalOption::Type,
        value: *const std::ffi::c_void,
    ) -> ffi::Result::Type {
        // SAFETY: Registration supplies a live handle with exclusive access.
        let terminal = unsafe { &mut *terminal.cast::<MockTerminal>() };
        match option {
            ffi::TerminalOption::USERDATA => terminal.userdata = value.cast_mut(),
            ffi::TerminalOption::WRITE_PTY => {
                // SAFETY: The wrapper supplies the matching callback ABI.
                terminal.write = Some(unsafe { std::mem::transmute(value) });
            }
            ffi::TerminalOption::CLIPBOARD_READ => {
                // SAFETY: The wrapper supplies the matching callback ABI.
                terminal.read = Some(unsafe { std::mem::transmute(value) });
            }
            ffi::TerminalOption::RESIZE_WINDOW => {
                // SAFETY: The wrapper supplies the matching callback ABI.
                terminal.resize = Some(unsafe { std::mem::transmute(value) });
            }
            ffi::TerminalOption::UNKNOWN_SEQUENCE => {
                // SAFETY: The wrapper supplies the matching callback ABI.
                terminal.unknown = Some(unsafe { std::mem::transmute(value) });
            }
            _ => return ffi::Result::INVALID_VALUE,
        }
        ffi::Result::SUCCESS
    }

    unsafe extern "C" fn reply_to_read(
        read: *const ffi::ClipboardRead,
        _: *const ffi::ClipboardReadReply,
    ) {
        // SAFETY: The synchronous request stores the live mock handle in ctx.
        let terminal = unsafe { (*read).ctx.cast_mut().cast::<ffi::TerminalImpl>() };
        // SAFETY: This models a clipboard reply's nested pty write dispatch.
        unsafe { ghostty_terminal_vt_write(terminal, b"reply".as_ptr(), 5) };
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn ghostty_terminal_vt_write(
        terminal: ffi::Terminal,
        data: *const u8,
        len: usize,
    ) {
        let (userdata, write, read, resize, unknown) = {
            // SAFETY: The wrapper supplies a live handle. End this borrow
            // before dispatch, since a clipboard reply can dispatch again.
            let mock = unsafe { &*terminal.cast::<MockTerminal>() };
            (
                mock.userdata,
                mock.write,
                mock.read,
                mock.resize,
                mock.unknown,
            )
        };
        // SAFETY: vt_write supplies initialized bytes for this call.
        let bytes = unsafe { std::slice::from_raw_parts(data, len) };
        if let Some(kind) = bytes.strip_prefix(b"resize:") {
            // The libghostty these bindings build against only sends tags
            // they know, so a kind added upstream later can only be faked
            // here. The input picks the tag.
            let known = kind == b"grid";
            // Write only the tag and the active field, leaving the rest of the
            // union (and struct padding) uninitialized, so miri flags any
            // read of bytes upstream never wrote. An unknown kind has no
            // field we know, so its whole union stays uninitialized and miri
            // also flags a grid read before the tag is checked.
            let mut request = MaybeUninit::<ffi::TerminalResizeWindow>::uninit();
            let p = request.as_mut_ptr();
            // SAFETY: `p` points to writable, aligned storage for the struct,
            // and `&raw mut` makes no reference to the uninitialized memory.
            // This matches `lib.TaggedUnion.cval`, whose `@unionInit` leaves
            // the rest of the union undefined.
            unsafe {
                if known {
                    (&raw mut (*p).tag).write(ffi::TerminalResizeWindowTag::GRID);
                    (&raw mut (*p).value.grid).write(ffi::TerminalResizeWindowGrid {
                        rows: 30,
                        columns: 100,
                    });
                } else {
                    (&raw mut (*p).tag).write(ffi::TerminalResizeWindowTag::GRID + 1);
                }
            }
            // SAFETY: Registration provided this callback and its userdata;
            // the request remains live throughout synchronous dispatch.
            unsafe { resize.unwrap()(terminal, userdata, p.cast_const()) };
        } else if let Some(kind) = bytes.strip_prefix(b"unknown:") {
            // Like `resize:` above, the input picks the shape and only the tag
            // and the active field are written. `future` is a kind added
            // upstream later, so its whole union stays uninitialized;
            // `osc-badterm` is an OSC with a terminator added upstream later.
            let content = b"payload";
            let content = ffi::String {
                ptr: content.as_ptr(),
                len: content.len(),
            };
            let mut sequence = MaybeUninit::<ffi::TerminalUnknownSequence>::uninit();
            let p = sequence.as_mut_ptr();
            // SAFETY: As for `resize:` above.
            unsafe {
                match kind {
                    b"apc" => {
                        (&raw mut (*p).tag).write(ffi::TerminalUnknownSequenceTag::APC);
                        (&raw mut (*p).value.apc).write(ffi::TerminalUnknownStringSequence {
                            truncated: true,
                            content,
                        });
                    }
                    b"osc" | b"osc-badterm" => {
                        let terminator = if kind == b"osc" {
                            ffi::OscTerminator::BEL
                        } else {
                            ffi::OscTerminator::BEL + 1
                        };
                        (&raw mut (*p).tag).write(ffi::TerminalUnknownSequenceTag::OSC);
                        (&raw mut (*p).value.osc).write(ffi::TerminalUnknownOscSequence {
                            truncated: false,
                            content,
                            terminator,
                        });
                    }
                    _ => (&raw mut (*p).tag).write(ffi::TerminalUnknownSequenceTag::OSC + 1),
                }
            }
            // SAFETY: Registration provided this callback and its userdata;
            // the sequence and its static content remain live throughout
            // synchronous dispatch.
            unsafe { unknown.unwrap()(terminal, userdata, p.cast_const()) };
        } else if bytes == b"clipboard" {
            let request = ffi::ClipboardRead {
                ctx: terminal.cast(),
                reply: Some(reply_to_read),
                ..ffi::ClipboardRead::default()
            };
            // SAFETY: Registration provided this callback and its userdata;
            // the request remains live throughout synchronous dispatch.
            unsafe { read.unwrap()(terminal, userdata, &raw const request) };
        } else {
            // SAFETY: Registration provided this callback and its userdata.
            unsafe { write.unwrap()(terminal, userdata, data, len) };
        }
    }

    #[test]
    fn moved_terminal_callbacks_and_destruction() {
        use std::cell::Cell;
        struct CountDrop<'a>(&'a Cell<usize>);
        impl Drop for CountDrop<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        fn dispatch<'alloc>(mut terminal: Terminal<'alloc>) -> Terminal<'alloc> {
            terminal.vt_write(b"reply");
            terminal
        }

        let drops = Cell::new(0);
        let calls = Cell::new(0);
        let mut terminal = Terminal::new(8, 2).unwrap();
        let first = CountDrop(&drops);
        let observed = &calls;
        terminal
            .on_pty_write(move |_, bytes| {
                let _guard = &first;
                assert_eq!(bytes, b"reply");
                observed.set(observed.get() + 1);
            })
            .unwrap();
        let mut terminal = dispatch(terminal);
        assert_eq!(calls.get(), 1);
        assert_eq!(drops.get(), 0);

        let second = CountDrop(&drops);
        terminal
            .on_pty_write(move |_, bytes| {
                let _guard = &second;
                assert_eq!(bytes, b"reply");
                observed.set(observed.get() + 10);
            })
            .unwrap();
        assert_eq!(drops.get(), 1);
        drop(dispatch(terminal));
        assert_eq!(calls.get(), 11);
        assert_eq!(drops.get(), 2);
        drop(Terminal::new(8, 2).unwrap());
    }

    #[test]
    fn moved_terminal_nested_clipboard_reply() {
        let order = std::cell::RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal
            .on_pty_write(|_, bytes| {
                assert_eq!(bytes, b"reply");
                order.borrow_mut().push(2);
            })
            .unwrap();
        terminal
            .on_clipboard_read(|_, request| {
                order.borrow_mut().push(1);
                request.reply(Ok(&[])).finish();
                order.borrow_mut().push(3);
            })
            .unwrap();
        fn dispatch(mut terminal: Terminal<'_>) {
            terminal.vt_write(b"clipboard");
        }
        dispatch(terminal);
        assert_eq!(*order.borrow(), [1, 2, 3]);
    }
}
