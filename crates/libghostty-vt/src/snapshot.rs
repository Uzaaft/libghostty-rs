//! Encode and restore the complete state of a terminal via a binary format.
//!
//! A snapshot is an ordered, CRC-protected record stream. Its READY marker
//! follows enough state to render and resume the terminal, including any
//! unfinished VT parser input. Older scrollback pages follow READY and the
//! FINISH marker terminates the complete snapshot.
//!
//! End-of-file before an operation's required READY or FINISH marker is
//! malformed, truncated snapshot data and returns [`Error::InvalidValue`].
//! [`Error::IoError`] is reserved for reader errors.
//!
//! Decoding is done with the dedicated [`Decoder`] struct; encoding, meanwhile,
//! is supported by methods on [`Terminal`] like [`Terminal::encode_snapshot`].
//!
//! # Format
//!
//! Every integer is unsigned and little-endian.
//! The stream begins with this fixed ten-byte envelope:
//!
//! ```text
//! byte  0               8       10
//!       +---------------+--------+
//!       | "GHOSTSNP"    | version|
//!       | 8-byte magic  | u16    |
//!       +---------------+--------+
//! ```
//!
//! The envelope is followed by independently checksummed records. A record's
//! CRC32C covers its encoded tag and payload length followed by its payload;
//! it does not cover the CRC field itself.
//!
//! ```text
//! byte  0       2             6          10             10 + payload_len
//!       +-------+-------------+-----------+----------------+
//!       | tag   | payload_len | CRC32C    | payload        |
//!       | u16   | u32         | u32       | payload_len B  |
//!       +-------+-------------+-----------+----------------+
//!       \____________________/             \______________/
//!          CRC prefix                         CRC suffix
//! ```
//!
//! Record groups occur in this strict order. SCREEN and HISTORY groups contain
//! one entry for each screen declared by TERMINAL. Each manifest is followed by
//! the number of PAGE records it declares. Active SCREEN pages make the terminal
//! renderable; HISTORY pages are older scrollback ordered newest to oldest so
//! an incremental decoder can prepend them as they arrive.
//!
//! ```text
//!
//! +---------------- TERMINAL ----------------+
//! | terminal-wide state and screen count     |
//! +----------------- SCREEN -----------------+  repeated per screen
//! | active-screen manifest                   |
//! +------------------ PAGE ------------------+  repeated per manifest
//! | active screen rows                       |
//! +---------- KITTY_IMAGE_STORAGE -----------+  optional, per screen
//! | images, placements, in-progress load     |
//! +------------- CONTINUATION ---------------+
//! | unfinished VT/UTF-8 input, or ground     |
//! +------------------ READY -----------------+
//! | empty renderable-state marker            |  ready() returns here
//! +---------- KITTY_IMAGE_PAYLOAD -----------+  optional, anywhere until FINISH,
//! | one image's or frame's pixels            |  next() consumes one
//! +----------------- HISTORY ----------------+  repeated per screen
//! | scrollback manifest                      |
//! +------------------ PAGE ------------------+  next() consumes one page
//! | older screen rows                        |
//! +------ KITTY_IMAGE_PAGE_PLACEMENTS -------+  optional, after a PAGE
//! | placements pinned to that page           |
//! +------------------ FINISH ----------------+
//! | empty end-of-snapshot marker             |  next() returns NO_VALUE
//! +------------------------------------------+
//! | trailing transport bytes (not consumed) |
//! +------------------------------------------+
//! ```
//!
//! READY separates the renderable prefix through CONTINUATION from history.
//! FINISH terminates the record sequence. Both are empty records protected by
//! CRC32C, like every other record. Declared record counts, tags, and strict
//! decoding enforce the stream's ordering and completeness.
//!
//! A decoder skips any record whose tag it does not know, wherever it
//! appears. This lets a newer library add optional kinds of records that
//! older libraries ignore, without changing the snapshot version. Skipped
//! records are still checked, so a damaged record fails decoding even when
//! its tag is unknown.
//!
//! The Kitty graphics records are optional in this way. A library built
//! without Kitty graphics skips them and restores the same terminal without
//! its images, and a terminal without images writes none of them.
//!
//! Images are restored at READY with their pixels pending, and the pixels
//! arrive later through [`IncrementalDecoder::next`].
// The Kitty graphics items only exist with the `kitty-graphics` feature, so
// only link to them then; otherwise the links would be broken.
#![cfg_attr(
    feature = "kitty-graphics",
    doc = "A pending image can be placed and queried, but \
           [`Image::data`](crate::kitty::graphics::Image::data) returns \
           `Ok(None)` for it until its pixels arrive. When they do, the image \
           keeps its [`Image::generation`](crate::kitty::graphics::Image::generation), \
           but [`Graphics::generation`](crate::kitty::graphics::Graphics::generation) \
           changes, so a renderer that watches that value picks up the new \
           pixels on its next frame."
)]
#![cfg_attr(
    not(feature = "kitty-graphics"),
    doc = "A pending image can be placed and queried, but `Image::data` \
           returns `Ok(None)` for it until its pixels arrive. When they do, \
           the image keeps its `Image::generation`, but `Graphics::generation` \
           changes, so a renderer that watches that value picks up the new \
           pixels on its next frame."
)]
//!
//! Snapshot format version 1 is a work in progress and does not yet carry a
//! binary-compatibility guarantee.
//!
//! ## See also
//!
//! [Snapshot format and Zig codec documentation](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/snapshot/main.zig)
use std::{
    io::{Read, Write},
    marker::PhantomData,
    mem::MaybeUninit,
};

use crate::{
    alloc::{Allocator, Bytes, Object},
    error::{
        Error, Result, from_optional_result, from_optional_result_uninit,
        from_optional_result_with_len, from_result,
    },
    ffi::{self, SnapshotDecoderData as Data, SnapshotDecoderOption as Opt},
    screen::Screen,
    terminal::Terminal,
};

/// Snapshot-related methods.
impl Terminal<'_, '_> {
    /// Encode a complete terminal snapshot to a writer.
    ///
    /// The terminal's persistent VT stream supplies the continuation bytes
    /// needed to reconstruct unfinished parser state. The caller must prevent
    /// concurrent writes or other terminal mutation for the duration of this
    /// call. The writer callback must not call terminal APIs with the same
    /// terminal handle. A terminal can be encoded with tracking disabled when
    /// its VT parser and UTF-8 decoder are both at ground. If either is
    /// unfinished, tracking must have been enabled before the input that
    /// produced that state was written; otherwise this returns
    /// [`Error::InvalidValue`].
    ///
    /// Encoding begins at the writer's current position. If an error occurs,
    /// the writer may contain a partial snapshot without a valid FINISH
    /// marker. Calls to the writer are synchronous; this function does not
    /// flush or make the caller's destination durable.
    ///
    /// # Errors
    ///
    /// This function returns [`Error::IoError`] if the writer rejects output,
    /// [`Error::LimitExceeded`] if output accounting overflows, or another
    /// error code on failure.
    pub fn encode_snapshot<W: Write>(&mut self, writer: &mut W) -> Result<()> {
        let writer = crate::io::to_writer(writer);
        let result = unsafe { ffi::ghostty_snapshot_encode(self.inner.as_raw(), writer) };
        from_result(result)
    }

    /// Encode a complete terminal snapshot to an allocated buffer.
    ///
    /// The returned buffer is allocated with allocator, or the default
    /// allocator when allocator is `None`.
    ///
    /// A terminal can be encoded with tracking disabled when its VT parser
    /// and UTF-8 decoder are both at ground. If either is unfinished, tracking
    /// must have been enabled before the input that produced that state was
    /// written; otherwise this returns [`Error::InvalidValue`].
    pub fn encode_snapshot_alloc<'a, 'ctx: 'a>(
        &self,
        alloc: Option<&'a Allocator<'ctx>>,
    ) -> Result<Option<Bytes<'a>>> {
        let mut out = std::ptr::null_mut();
        let mut out_len = 0usize;
        let alloc = alloc.map_or(std::ptr::null(), super::alloc::Allocator::to_raw);

        let result = unsafe {
            ffi::ghostty_snapshot_encode_alloc(
                self.inner.as_raw(),
                alloc,
                &raw mut out,
                &raw mut out_len,
            )
        };

        let out = from_optional_result(result, out)?;
        // SAFETY: On success, libghostty hands over `out_len` bytes allocated
        // with `alloc`, or NULL for empty output.
        Ok(out.map(|ptr| unsafe { Bytes::from_raw_parts(ptr, out_len, alloc) }))
    }

    /// Encode a complete terminal snapshot to a caller-provided buffer.
    ///
    /// Pass an empty `buf` to query the required size. A size query returns
    /// [`Error::OutOfSpace`] with the required size, including zero when the
    /// stream is at ground. If a non-empty buffer is too small, the function
    /// has the same result and reports the full required size.
    ///
    /// A terminal can be encoded with tracking disabled when its VT parser
    /// and UTF-8 decoder are both at ground. If either is unfinished, tracking
    /// must have been enabled before the input that produced that state was
    /// written; otherwise this returns [`Error::InvalidValue`].
    pub fn encode_snapshot_buf(&self, buf: &mut [u8]) -> Result<Option<usize>> {
        let mut written = 0usize;

        let result = unsafe {
            ffi::ghostty_snapshot_encode_buf(
                self.inner.as_raw(),
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut written,
            )
        };

        from_optional_result_with_len(result, written)
    }
}

/// Opaque handle to a terminal snapshot decoder.
#[derive(Debug)]
pub struct Decoder<'alloc, 'r> {
    inner: Object<'alloc, ffi::SnapshotDecoderImpl>,
    _phan: PhantomData<&'r mut ffi::Reader>,
}

impl<'alloc, 'r> Decoder<'alloc, 'r> {
    /// Create a snapshot decoder that reads from a caller-provided reader.
    ///
    /// Reads are synchronous and occur only during ready, next, or decode calls.
    /// A zero-byte successful read is permanent end-of-file, not temporary
    /// starvation; nonblocking sources must wait outside the decoder or block
    /// in their callback. Reading zero bytes before a required marker
    /// reports truncated snapshot data as [`Error::InvalidValue`].
    pub fn new<R: Read>(r: &'r mut R) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), r) }
    }

    /// Create a new snapshot decoder that reads from a caller-provided reader
    /// with a custom allocator.
    ///
    /// Reads are synchronous and occur only during ready, next, or decode calls.
    /// A zero-byte successful read is permanent end-of-file, not temporary
    /// starvation; nonblocking sources must wait outside the decoder or block
    /// in their callback. The read callback must not call APIs on or drop the
    /// decoder that owns it. Reading zero bytes before a required marker
    /// reports truncated snapshot data as [`Error::InvalidValue`].
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, R: Read>(
        alloc: &'alloc Allocator<'ctx>,
        r: &'r mut R,
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), r) }
    }

    unsafe fn new_inner<R: Read>(alloc: *const ffi::Allocator, r: &'r mut R) -> Result<Self> {
        let reader = crate::io::to_reader(r);
        let mut raw: ffi::SnapshotDecoder = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_snapshot_decoder_new(alloc, &raw mut raw, reader) };
        from_result(result)?;
        Ok(Self {
            inner: Object::new(raw)?,
            _phan: PhantomData,
        })
    }

    /// Create a snapshot decoder over a borrowed byte buffer.
    ///
    /// The bytes are not copied. Bytes after FINISH are not consumed;
    /// query [`Decoder::source_offset`] to locate them. To do so, decode with
    /// [`Decoder::ready`] and read it through [`IncrementalDecoder::decoder`]
    /// after FINISH.
    pub fn new_buf(buf: &'r [u8]) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_buf_inner(std::ptr::null(), buf) }
    }

    /// Create a new snapshot decoder over a borrowed byte buffer
    /// with a custom allocator.
    ///
    /// The bytes are not copied. Bytes after FINISH are not consumed;
    /// query [`Decoder::source_offset`] to locate them. To do so, decode with
    /// [`Decoder::ready`] and read it through [`IncrementalDecoder::decoder`]
    /// after FINISH.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_buf_with_alloc<'ctx: 'alloc>(
        alloc: &'alloc Allocator<'ctx>,
        buf: &'r [u8],
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_buf_inner(alloc.to_raw(), buf) }
    }

    unsafe fn new_buf_inner(alloc: *const ffi::Allocator, buf: &[u8]) -> Result<Self> {
        let mut raw: ffi::SnapshotDecoder = std::ptr::null_mut();
        let result = unsafe {
            ffi::ghostty_snapshot_decoder_new_buf(alloc, &raw mut raw, buf.as_ptr(), buf.len())
        };
        from_result(result)?;
        Ok(Self {
            inner: Object::new(raw)?,
            _phan: PhantomData,
        })
    }

    /// Decode and validate one complete snapshot.
    ///
    /// This is the one-shot form of READY followed by all history pages
    /// through FINISH. It may only be called before decoding starts. Bytes
    /// following FINISH are left unread. On success this returns a
    /// caller-owned terminal with its persistent VT stream restored.
    /// Continuation tracking on the returned terminal is disabled by default.
    /// When [`Self::set_retain_continuation`] is enabled, the decoder's
    /// maximum continuation size is applied to the terminal, and the terminal
    /// continuation APIs export the exact current continuation when that limit
    /// is nonzero. Tracking remains enabled even if the exported continuation
    /// is empty. Callers that do not need ongoing tracking must call
    /// [`Terminal::set_continuation_max_bytes`] with zero after export and
    /// before writing any post-snapshot bytes, because later input may change
    /// it.
    ///
    /// A decoding, I/O, or allocation error after input consumption begins
    /// poisons the decoder, after which it must be dropped. An invalid
    /// argument or lifecycle error detected before the operation consumes
    /// input does not poison it.    
    pub fn decode<'cb>(self) -> Result<Terminal<'alloc, 'cb>> {
        let mut raw: ffi::Terminal = std::ptr::null_mut();
        let result =
            unsafe { ffi::ghostty_snapshot_decoder_decode(self.inner.as_raw(), &raw mut raw) };
        from_result(result)?;
        unsafe { Terminal::from_raw(raw) }
    }

    /// Decode and validate the renderable snapshot prefix through READY.
    ///
    /// On success, terminal receives a caller-owned terminal with its
    /// persistent VT stream already restored from the snapshot continuation.
    /// The terminal is immediately usable for rendering and live input.
    /// Older scrollback remains to be restored with [`IncrementalDecoder::next`].
    ///
    /// The restored parser state may be unfinished. By default, terminal
    /// continuation tracking is disabled and
    /// [`Terminal::continuation_max_bytes`] returns zero.
    /// When [`Self::set_retain_continuation`] is enabled, the decoder's
    /// maximum continuation size is applied to the terminal, and the terminal
    /// continuation APIs export the exact current continuation when that limit
    /// is nonzero. Tracking remains enabled even if the exported continuation
    /// is empty. Callers that do not need ongoing tracking must call
    /// [`Terminal::set_continuation_max_bytes`] with zero after export and
    /// before writing any post-snapshot bytes, because later input may change
    /// it.
    ///
    /// A decoding, I/O, or allocation error after input consumption begins
    /// poisons the decoder, after which it must be dropped. An invalid
    /// argument or lifecycle error detected before the operation consumes
    /// input does not poison it.    
    pub fn ready<'cb>(self) -> Result<IncrementalDecoder<'alloc, 'r, 'cb>> {
        let mut raw: ffi::Terminal = std::ptr::null_mut();
        let result =
            unsafe { ffi::ghostty_snapshot_decoder_ready(self.inner.as_raw(), &raw mut raw) };
        from_result(result)?;
        let terminal = unsafe { Terminal::from_raw(raw)? };
        Ok(IncrementalDecoder {
            decoder: self,
            ready_terminal: terminal.id,
            finished: false,
            terminal,
        })
    }

    fn get<T>(&self, tag: Data::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_snapshot_decoder_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }
    fn get_optional<T>(&self, tag: Data::Type) -> Result<Option<T>> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_snapshot_decoder_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_optional_result_uninit(result, value)
    }
    fn set<T>(&self, tag: Opt::Type, v: &T) -> Result<()> {
        let result = unsafe {
            ffi::ghostty_snapshot_decoder_set(
                self.inner.as_raw(),
                tag,
                std::ptr::from_ref(v).cast(),
            )
        };
        from_result(result)
    }

    /// Current maximum accepted continuation size.
    ///
    /// This value is available in every non-failed decoder state.
    pub fn max_continuation_bytes(&self) -> Result<usize> {
        self.get(Data::MAX_CONTINUATION_BYTES)
    }

    /// Largest non-ground continuation the decoder will accept.
    ///
    /// A value of zero accepts only snapshots whose VT parser is in the ground
    /// state. The decoder default matches the largest built-in APC protocol
    /// buffer limit, currently 65 MiB.
    ///
    /// This is primarily an input validation limit. When
    /// [`Self::set_retain_continuation`] is enabled, the same value also
    /// becomes the continuation tracking limit on the returned terminal.
    pub fn set_max_continuation_bytes(&mut self, v: usize) -> Result<&mut Self> {
        self.set(Opt::MAX_CONTINUATION_BYTES, &v)?;
        Ok(self)
    }

    /// Whether decoded continuation tracking is retained on returned
    /// terminals.
    ///
    /// This value is available in every non-failed decoder state.
    pub fn retain_continuation(&self) -> Result<bool> {
        self.get(Data::RETAIN_CONTINUATION)
    }

    /// Retain the decoded continuation on the returned terminal.
    ///
    /// When true, terminals returned by [`Self::ready`] and [`Self::decode`]
    /// use [`Self::max_continuation_bytes`] as their continuation tracking
    /// limit. The existing continuation APIs such as
    /// [`Terminal::continuation_buf`] can then export the exact unfinished VT
    /// or UTF-8 input restored from the snapshot.
    ///
    /// This is false by default. A maximum continuation size of zero leaves
    /// tracking disabled. With a nonzero maximum, tracking remains enabled
    /// even when the decoded continuation is empty. Exporting an empty
    /// continuation does not disable it. Callers that do not need ongoing
    /// tracking must still call [`Terminal::set_continuation_max_bytes`] with
    /// zero after export and before writing post-snapshot input.
    pub fn set_retain_continuation(&mut self, value: bool) -> Result<&mut Self> {
        self.set(Opt::RETAIN_CONTINUATION, &value)?;
        Ok(self)
    }

    /// Whether history is compressed while it is restored.
    ///
    /// See [`Self::set_compress_history_enabled`]. This value is available in
    /// every non-failed decoder state.
    pub fn is_compress_history_enabled(&self) -> Result<bool> {
        self.get(Data::COMPRESS_HISTORY)
    }

    /// Compress scrollback history while it is restored.
    ///
    /// By default, restoring a snapshot leaves all of its scrollback history
    /// uncompressed, even if the terminal that produced the snapshot had
    /// compressed it. The history stays that size until the application calls
    /// [`Terminal::compress`]. For a terminal with a lot of scrollback, that
    /// can be many times more memory than the terminal needed before.
    ///
    /// When this option is true, the decoder compresses each history page
    /// right after restoring it. The restored terminal starts out compressed,
    /// and the decode never holds more than one uncompressed history page at a
    /// time. The result is the same as decoding normally and then calling
    /// [`Terminal::compress`] with
    /// [`CompressionMode::Full`](crate::terminal::CompressionMode::Full),
    /// without the memory spike in between.
    ///
    /// A history page that is on screen when it is restored stays
    /// uncompressed. This only happens if the viewport is scrolled to the top
    /// of the scrollback during an incremental decode. Compressed history is
    /// uncompressed automatically when it is accessed later, for example by
    /// scrolling or searching.
    ///
    /// This only changes how the restored terminal stores its history in
    /// memory. The snapshot format is unchanged, so it works with any
    /// snapshot. On platforms that do not support scrollback compression, this
    /// option is accepted and has no effect.
    ///
    /// This is false by default.
    pub fn set_compress_history_enabled(&mut self, enabled: bool) -> Result<&mut Self> {
        self.set(Opt::COMPRESS_HISTORY, &enabled)?;
        Ok(self)
    }

    // The storage limit pair is gated on `kitty-graphics` like its terminal
    // counterparts in `kitty::graphics`, which the docs below link to. The
    // -sys keys exist either way, but without the feature there's no
    // `Terminal` API to read or change a restored terminal's limit, so the
    // decoder option would be the odd one out.

    /// Kitty graphics image storage limit applied to each restored screen.
    ///
    /// See [`Self::set_kitty_image_storage_limit`]. This value is available
    /// in every non-failed decoder state.
    #[cfg(feature = "kitty-graphics")]
    pub fn kitty_image_storage_limit(&self) -> Result<u64> {
        self.get(Data::KITTY_IMAGE_STORAGE_LIMIT)
    }

    /// Kitty graphics image storage limit, in bytes, for each restored
    /// screen.
    ///
    /// The limit of the terminal that wrote the snapshot is not part of the
    /// snapshot, so the decoder always applies this one. Images are restored
    /// oldest first, and an image that does not fit evicts older images,
    /// exactly as a live transmission would. The newest images are kept, and
    /// the placements of evicted images are dropped with them. Zero disables
    /// Kitty graphics on the restored terminal.
    ///
    /// The default is the storage limit of a new terminal. Change the limit
    /// of a restored terminal later with
    /// [`Terminal::set_kitty_image_storage_limit`].
    #[cfg(feature = "kitty-graphics")]
    pub fn set_kitty_image_storage_limit(&mut self, limit: u64) -> Result<&mut Self> {
        self.set(Opt::KITTY_IMAGE_STORAGE_LIMIT, &limit)?;
        Ok(self)
    }

    /// Number of snapshot source bytes consumed so far.
    ///
    /// At FINISH this identifies the first byte after the snapshot. Trailing
    /// bytes are not consumed. This value is unavailable after a decoding
    /// error, because the decoder can no longer guarantee its source position.
    pub fn source_offset(&self) -> Result<usize> {
        self.get(Data::SOURCE_OFFSET)
    }
    /// Advisory complete logical history extent for the primary screen.
    ///
    /// The value counts rows before the active area, including any resident
    /// overlap carried before READY. It becomes available after READY validates.
    pub fn history_rows_primary(&self) -> Result<u64> {
        self.get(Data::HISTORY_ROWS_PRIMARY)
    }
    /// Advisory complete logical history extent for the alternate screen.
    ///
    /// The value has the same semantics and lifetime as [`Decoder::history_rows_primary`]
    /// Querying it returns `Ok(None)` when the snapshot does not declare an
    /// alternate screen.
    pub fn history_rows_alternate(&self) -> Result<Option<u64>> {
        self.get_optional(Data::HISTORY_ROWS_ALTERNATE)
    }
}

impl Drop for Decoder<'_, '_> {
    fn drop(&mut self) {
        unsafe {
            ffi::ghostty_snapshot_decoder_free(self.inner.as_raw());
        }
    }
}

/// A [`Decoder`] that incrementally decodes history pages, and the pixels of
/// Kitty graphics images, into the terminal, obtained by calling
/// [`Decoder::ready`].
///
/// Call [`IncrementalDecoder::next`] repeatedly until `Ok(None)` is returned
/// to keep decoding history and image pixels from the snapshot.
///
/// The terminal is accessible for use during the decode process via methods
/// like [`IncrementalDecoder::terminal`] and [`IncrementalDecoder::terminal_mut`],
/// while obtaining ownership of the terminal requires halting the decode
/// process via [`IncrementalDecoder::into_terminal`].
#[derive(Debug)]
pub struct IncrementalDecoder<'alloc, 'r, 'cb> {
    // Drop order is significant here.
    // First drop the decoder, then the terminal.
    decoder: Decoder<'alloc, 'r>,
    terminal: Terminal<'alloc, 'cb>,
    // The identity of the terminal returned by READY. libghostty retains its
    // handle inside the decoder and writes history into it on every `next`
    // call, but `terminal_mut` lets safe code swap `terminal` for another one
    // (e.g. via `std::mem::replace`) and drop the original. `next` checks this
    // against `terminal` so it only lets libghostty touch the handle while we
    // own it.
    //
    // This compares `Terminal::id` rather than the handle: a replacement
    // terminal can be allocated at the freed original's address, and would
    // then receive the rest of the history.
    ready_terminal: u64,
    // Whether FINISH was reached. libghostty doesn't touch the terminal after
    // that, so the check above no longer applies.
    finished: bool,
}

impl<'alloc, 'r, 'cb> IncrementalDecoder<'alloc, 'r, 'cb> {
    /// Decode one history page, or the pixels of one Kitty graphics image,
    /// into the terminal returned by READY.
    ///
    /// Each `Ok(Some(progress))` result consumes and validates one record: a
    /// history page together with the Kitty graphics placements pinned to
    /// it, or the pixels of one image or animation frame. Pixels may arrive
    /// at any point, even between history pages. Query [`Progress::kind`]
    /// and the progress values for that kind on the returned `progress`
    /// before calling [`IncrementalDecoder::next`] again. `Ok(None)` means
    /// FINISH was validated; repeated calls after FINISH also return
    /// `Ok(None)`.
    ///
    /// ```rust
    /// use libghostty_vt::{
    ///     Terminal,
    ///     screen::Screen,
    ///     snapshot::{Decoder, ProgressKind},
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let mut source = Terminal::new(80, 24)?;
    /// # for i in 0..5000 {
    /// #     source.vt_write(format!("line {i}\r\n").as_bytes());
    /// # }
    /// # // A 2x1 RGBA image: 2 * 1 * 4 = 8 bytes of pixels.
    /// # source.vt_write(b"\x1b_Ga=T,f=32,s=2,v=1,i=7,q=2;AAAA/wAAAP8=\x1b\\");
    /// # let mut snapshot = Vec::new();
    /// # source.encode_snapshot(&mut snapshot)?;
    /// let mut incremental = Decoder::new_buf(&snapshot)?.ready()?;
    ///
    /// // How many scrollback rows were restored, and how many image bytes the
    /// // primary screen received, to show progress while they arrive.
    /// let mut restored = 0;
    /// let mut received = 0;
    ///
    /// while let Some(progress) = incremental.next()? {
    ///     match progress.kind()? {
    ///         // Scrollback grew by this many rows.
    ///         ProgressKind::Page { rows, .. } => restored += rows,
    ///         // An image's pixels arrived.
    ///         ProgressKind::Image { bytes, .. } => {
    ///             if progress.screen()? == Screen::Primary {
    ///                 received += bytes;
    ///             }
    ///         }
    ///         // `ProgressKind` is non-exhaustive, so a later version of
    ///         // this crate may decode more kinds of records.
    ///         _ => {}
    ///     }
    /// }
    ///
    /// // `Ok(None)` means the snapshot is complete. Dropped pixels report
    /// // zero bytes, so `received` can end up below what was sent.
    /// # assert!(restored > 0);
    /// # // A library built without Kitty graphics skips the image.
    /// # #[cfg(feature = "kitty-graphics")]
    /// # assert_eq!(received, 8);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The terminal may be rendered, resized, and fed live PTY input between
    /// calls. If a history page can no longer be applied safely, it is still
    /// consumed and validated and progress reports zero rows. Image pixels
    /// work the same way: if live input deleted the image, or sent a new
    /// image with the same ID, the pixels are consumed and validated and
    /// progress reports zero bytes. The decoder applies history and pixels to
    /// the terminal produced by its READY operation.
    ///
    /// If [`Decoder::set_compress_history_enabled`] is enabled, the page is
    /// compressed before this returns, unless it is visible in the terminal's
    /// viewport.
    ///
    /// If that terminal has been replaced through
    /// [`IncrementalDecoder::terminal_mut`] (e.g. with [`std::mem::replace`]),
    /// this returns [`Error::InvalidValue`] without consuming input. Unlike a
    /// decoding error, this doesn't invalidate the decoder: putting the READY
    /// terminal back allows decoding to continue. After FINISH, the terminal
    /// may be replaced freely.
    ///
    /// A decoding error invalidates the decoder's source position. The terminal
    /// remains usable with its already-restored history, but the decoder can
    /// only be dropped.
    pub fn next<'d>(&'d mut self) -> Result<Option<Progress<'alloc, 'r, 'd>>> {
        if self.finished {
            return Ok(None);
        }
        // libghostty applies history to the handle it retained at READY, not to
        // whatever terminal we hold now. If we no longer hold the READY
        // terminal, it may already have been freed, so calling into
        // libghostty would be a use-after-free.
        if self.terminal.id != self.ready_terminal {
            return Err(Error::InvalidValue);
        }
        let result = unsafe { ffi::ghostty_snapshot_decoder_next(self.decoder.inner.as_raw()) };
        self.finished = result == ffi::Result::NO_VALUE;
        from_optional_result(
            result,
            Progress {
                decoder: &self.decoder,
            },
        )
    }

    /// Get a reference to the underlying decoder.
    ///
    /// It can be queried between pages and after FINISH, e.g. for
    /// [`Decoder::source_offset`] and the history row counts. After a
    /// decoding error only the history row counts remain available; the other
    /// values return [`Error::InvalidValue`].
    #[must_use]
    pub fn decoder(&self) -> &Decoder<'alloc, 'r> {
        // Shared only: the setters need `&mut`, and upstream rejects options
        // once READY has run anyway.
        &self.decoder
    }
    /// Return a shared reference to the terminal being decoded.
    #[must_use]
    pub fn terminal(&self) -> &Terminal<'alloc, 'cb> {
        &self.terminal
    }
    /// Return an exclusive reference to the terminal being decoded.
    ///
    /// Replacing the terminal behind this reference makes
    /// [`IncrementalDecoder::next`] fail until the original is put back.
    pub fn terminal_mut(&mut self) -> &mut Terminal<'alloc, 'cb> {
        &mut self.terminal
    }
    /// Stop decoding and obtain the final, fully decoded terminal.
    #[must_use]
    pub fn into_terminal(self) -> Terminal<'alloc, 'cb> {
        self.terminal
    }
}

/// The current progress of the decode process: what the most recent
/// successful [`IncrementalDecoder::next`] call decoded, a history page or the
/// pixels of a Kitty graphics image.
#[derive(Debug, Clone, Copy)]
pub struct Progress<'alloc, 'r, 'd> {
    decoder: &'d Decoder<'alloc, 'r>,
}

impl<'alloc, 'r, 'd> Progress<'alloc, 'r, 'd> {
    /// Screen associated with the most recently decoded history page or
    /// image pixels.
    pub fn screen(&self) -> Result<Screen> {
        self.decoder
            .get::<ffi::TerminalScreen::Type>(Data::PROGRESS_SCREEN)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }

    /// What the most recent successful [`IncrementalDecoder::next`] call
    /// decoded: a history page or the pixels of a Kitty graphics image.
    ///
    /// A kind these bindings don't know yet returns [`Error::InvalidValue`].
    pub fn kind(&self) -> Result<ProgressKind> {
        use ffi::SnapshotDecoderProgressKind as Kind;

        // Upstream only gives the page keys a value for a page and the image
        // keys a value for an image; the others return NO_VALUE. So read the
        // tag first, then only the keys that apply to it, the same way
        // `osc::Command::command_type` does. Every key read here has a value
        // for its kind, so plain `get` is enough: a NO_VALUE would be an
        // upstream bug, and surfaces as an error rather than a made-up zero.
        match self.decoder.get::<Kind::Type>(Data::PROGRESS_KIND)? {
            Kind::PAGE => Ok(ProgressKind::Page {
                rows: self.decoder.get(Data::PROGRESS_ROWS)?,
                remaining: self.decoder.get(Data::PROGRESS_REMAINING)?,
            }),
            Kind::IMAGE => Ok(ProgressKind::Image {
                id: self.decoder.get(Data::PROGRESS_IMAGE_ID)?,
                frame: self.decoder.get(Data::PROGRESS_IMAGE_FRAME)?,
                bytes: self.decoder.get(Data::PROGRESS_IMAGE_BYTES)?,
            }),
            // A newer libghostty may decode records we have no variant for.
            // Unlike `osc::CommandType`, which has an `Invalid` variant to
            // fall back to, this already returns a `Result`, so say so
            // there instead of guessing at a variant.
            _ => Err(Error::InvalidValue),
        }
    }

    /// Get a reference to the underlying decoder.
    #[must_use]
    pub fn as_decoder(self) -> &'d Decoder<'alloc, 'r> {
        self.decoder
    }
}

/// What the most recent successful [`IncrementalDecoder::next`] call
/// decoded.
///
/// Query it with [`Progress::kind`]; each variant carries the progress values
/// that apply to that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProgressKind {
    /// One history page, together with the Kitty graphics placements pinned
    /// to it. `rows` and `remaining` describe it.
    Page {
        /// Rows prepended by the most recently decoded history page.
        ///
        /// Zero means the page was consumed and validated but could not be
        /// applied to the live terminal.
        rows: usize,
        /// Page records remaining in the same screen's HISTORY sequence.
        ///
        /// This is not a count of all pages remaining in the snapshot.
        remaining: u32,
    },
    /// The pixels of one Kitty graphics image or animation frame. `id`,
    /// `frame`, and `bytes` describe it.
    Image {
        /// ID of the Kitty graphics image whose pixels the most recent
        /// [`IncrementalDecoder::next`] call decoded.
        id: u32,
        /// Which frame of the image those pixels are for, numbered as the
        /// Kitty graphics protocol numbers frames. 1 is the image itself, and
        /// 2 and up are its animation frames.
        frame: u32,
        /// Number of pixel bytes the most recent [`IncrementalDecoder::next`]
        /// call attached to its image.
        ///
        /// Zero means the pixels were consumed and validated but dropped.
        /// That happens when live input deleted the image or sent a new image
        /// with the same ID, when the image did not fit the storage limit, or
        /// when its screen was removed or replaced.
        bytes: usize,
    },
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;

    /// A snapshot of a terminal stopped in the middle of `ESC [31`.
    fn unfinished_snapshot() -> Vec<u8> {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.set_continuation_max_bytes(1024).unwrap();
        terminal.vt_write(b"\x1b[31");
        let mut bytes = Vec::new();
        terminal.encode_snapshot(&mut bytes).unwrap();
        bytes
    }

    fn continuation(terminal: &Terminal<'_, '_>) -> Option<Vec<u8>> {
        let mut buf = [0; 16];
        let len = terminal.continuation_buf(&mut buf).unwrap()?;
        Some(buf[..len].to_vec())
    }

    /// A snapshot of an 80x24 terminal with enough scrollback that some of it
    /// is encoded as HISTORY pages after READY, so that
    /// [`IncrementalDecoder::next`] has something to apply.
    fn snapshot_with_history() -> Vec<u8> {
        let mut terminal = Terminal::new(80, 24).unwrap();
        for i in 0..5000 {
            terminal.vt_write(format!("line {i}\r\n").as_bytes());
        }
        let mut bytes = Vec::new();
        terminal.encode_snapshot(&mut bytes).unwrap();
        bytes
    }

    /// Decode the rest of the history and return the rows it restored.
    ///
    /// A snapshot without Kitty images has only pages after READY, and with a
    /// single (primary) screen they form one HISTORY sequence, so `remaining`
    /// counts down by one per page and reaches zero on the last.
    fn decode_pages(incremental: &mut IncrementalDecoder<'_, '_, '_>) -> usize {
        let mut rows = 0;
        let mut last_remaining = None;
        while let Some(progress) = incremental.next().unwrap() {
            assert_eq!(progress.screen().unwrap(), Screen::Primary);
            let ProgressKind::Page {
                rows: page_rows,
                remaining,
            } = progress.kind().unwrap()
            else {
                panic!("expected only history pages");
            };
            if let Some(last) = last_remaining {
                assert_eq!(remaining + 1, last);
            }
            last_remaining = Some(remaining);
            rows += page_rows;
        }
        assert_eq!(last_remaining, Some(0));
        rows
    }

    #[test]
    fn next_restores_history() {
        let bytes = snapshot_with_history();
        let mut incremental = Decoder::new_buf(&bytes).unwrap().ready().unwrap();
        assert!(decode_pages(&mut incremental) > 0);
    }

    /// The pixels of a Kitty image arrive through `next` as their own
    /// progress step, among the history pages.
    #[test]
    #[cfg(feature = "kitty-graphics")]
    fn next_reports_kitty_image_pixels() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        for i in 0..5000 {
            terminal.vt_write(format!("line {i}\r\n").as_bytes());
        }
        // A 2x1 RGBA image (f=32), placed on the active screen after the
        // history so no scrollback pruning can take it with it. Its pixels
        // are 2 * 1 * 4 = 8 bytes: two opaque black pixels. The decoder's
        // default storage limit (that of a new terminal) easily fits them.
        terminal.vt_write(b"\x1b_Ga=T,f=32,s=2,v=1,i=7,q=2;AAAA/wAAAP8=\x1b\\");
        let mut bytes = Vec::new();
        terminal.encode_snapshot(&mut bytes).unwrap();

        let mut incremental = Decoder::new_buf(&bytes).unwrap().ready().unwrap();
        // READY restores image metadata before `next` attaches the pixels.
        {
            let graphics = incremental
                .terminal()
                .kitty_graphics()
                .unwrap()
                .expect("kitty graphics enabled");
            let image = graphics.image(7).unwrap();
            assert_eq!(image.data().unwrap(), None);
            assert_eq!(image.data_len().unwrap(), 8);
        }
        let mut images = Vec::new();
        let mut pages = 0;
        while let Some(progress) = incremental.next().unwrap() {
            // Upstream only promises that pixels arrive somewhere before
            // FINISH, so don't depend on where among the pages they are.
            match progress.kind().unwrap() {
                ProgressKind::Page { .. } => pages += 1,
                image => images.push((image, progress.screen().unwrap())),
            }
        }
        assert!(pages > 0);
        assert_eq!(
            images,
            [(
                ProgressKind::Image {
                    id: 7,
                    frame: 1,
                    bytes: 8,
                },
                Screen::Primary,
            )]
        );

        // The reported bytes are the pixels that were attached to the image.
        let graphics = incremental
            .terminal()
            .kitty_graphics()
            .unwrap()
            .expect("kitty graphics enabled");
        let image = graphics.image(7).unwrap();
        assert_eq!(
            image.data().unwrap(),
            Some(&[0, 0, 0, 255, 0, 0, 0, 255][..])
        );
    }

    /// Swapping the READY terminal out of the incremental decoder and dropping
    /// it must not let [`IncrementalDecoder::next`] write into the freed
    /// terminal.
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn next_after_swapping_out_the_terminal_is_not_a_use_after_free() {
        let bytes = snapshot_with_history();
        let alloc = crate::alloc::testing::Guard::allocator();

        let decoder = Decoder::new_buf_with_alloc(&alloc, &bytes).unwrap();
        let mut incremental = decoder.ready().unwrap();

        let original = std::mem::replace(
            incremental.terminal_mut(),
            Terminal::new_with_alloc(&alloc, 80, 24).unwrap(),
        );
        drop(original);

        // The READY terminal is gone, so libghostty must not be asked to
        // write history into it.
        assert!(matches!(incremental.next(), Err(Error::InvalidValue)));
    }

    #[test]
    fn terminal_may_be_replaced_after_finish() {
        let bytes = snapshot_with_history();
        let mut incremental = Decoder::new_buf(&bytes).unwrap().ready().unwrap();
        while incremental.next().unwrap().is_some() {}

        drop(std::mem::replace(
            incremental.terminal_mut(),
            Terminal::new(80, 24).unwrap(),
        ));
        assert!(incremental.next().unwrap().is_none());
    }

    #[test]
    fn next_resumes_once_the_ready_terminal_is_put_back() {
        let bytes = snapshot_with_history();
        let mut incremental = Decoder::new_buf(&bytes).unwrap().ready().unwrap();

        let original =
            std::mem::replace(incremental.terminal_mut(), Terminal::new(80, 24).unwrap());
        assert!(matches!(incremental.next(), Err(Error::InvalidValue)));

        // The rejected call consumed nothing, so decoding carries on as if the
        // swap never happened.
        *incremental.terminal_mut() = original;
        assert!(decode_pages(&mut incremental) > 0);
    }

    /// The allocator usually hands a terminal created right after the
    /// original was freed the same address. It must still be rejected,
    /// rather than receive the rest of another terminal's history.
    #[test]
    fn next_rejects_a_new_terminal_at_the_old_address() {
        let bytes = snapshot_with_history();
        let mut incremental = Decoder::new_buf(&bytes).unwrap().ready().unwrap();

        let original =
            std::mem::replace(incremental.terminal_mut(), Terminal::new(80, 24).unwrap());
        drop(original);
        *incremental.terminal_mut() = Terminal::new(80, 24).unwrap();
        assert!(matches!(incremental.next(), Err(Error::InvalidValue)));
    }

    #[test]
    fn decoded_continuation_is_not_retained_by_default() {
        let bytes = unfinished_snapshot();
        let decoder = Decoder::new_buf(&bytes).unwrap();
        assert!(!decoder.retain_continuation().unwrap());
        let restored = decoder.decode().unwrap();
        assert_eq!(restored.continuation_max_bytes().unwrap(), 0);
    }

    #[test]
    fn decoded_continuation_can_be_exported_and_resumed() {
        let bytes = unfinished_snapshot();
        let mut decoder = Decoder::new_buf(&bytes).unwrap();
        decoder
            .set_max_continuation_bytes(1024)
            .unwrap()
            .set_retain_continuation(true)
            .unwrap();
        assert!(decoder.retain_continuation().unwrap());

        let mut restored = decoder.decode().unwrap();
        assert_eq!(restored.continuation_max_bytes().unwrap(), 1024);
        assert_eq!(continuation(&restored).as_deref(), Some(&b"\x1b[31"[..]));

        // Tracking isn't needed after the export, so turn it off before
        // writing post-snapshot input. The parser state is still restored.
        restored.set_continuation_max_bytes(0).unwrap();
        restored.vt_write(b"mX");
        assert!(restored.is_vt_ground().unwrap());
        assert_eq!(restored.cursor_x().unwrap(), 1);
    }

    #[test]
    fn ready_retains_continuation_before_history_is_restored() {
        let bytes = unfinished_snapshot();
        let mut decoder = Decoder::new_buf(&bytes).unwrap();
        decoder.set_retain_continuation(true).unwrap();
        // The tracking limit is the decoder's, not the encoder's.
        let limit = decoder.max_continuation_bytes().unwrap();
        let mut incremental = decoder.ready().unwrap();
        assert_eq!(
            incremental.terminal().continuation_max_bytes().unwrap(),
            limit
        );
        assert_eq!(
            continuation(incremental.terminal()).as_deref(),
            Some(&b"\x1b[31"[..])
        );
        while incremental.next().unwrap().is_some() {}
    }

    #[test]
    fn ground_snapshot_keeps_tracking_enabled() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.vt_write(b"hi");
        let mut bytes = Vec::new();
        terminal.encode_snapshot(&mut bytes).unwrap();

        let mut decoder = Decoder::new_buf(&bytes).unwrap();
        decoder
            .set_max_continuation_bytes(1024)
            .unwrap()
            .set_retain_continuation(true)
            .unwrap();
        let mut restored = decoder.decode().unwrap();

        // There was nothing unfinished to restore, but tracking is still on
        // and exports an empty continuation rather than refusing to.
        assert_eq!(restored.continuation_max_bytes().unwrap(), 1024);
        assert_eq!(continuation(&restored).as_deref(), Some(&b""[..]));

        // Exporting doesn't turn tracking off, so later input is still tracked.
        restored.vt_write(b"\x1b[");
        assert_eq!(continuation(&restored).as_deref(), Some(&b"\x1b["[..]));
    }

    #[test]
    fn zero_limit_leaves_tracking_disabled() {
        // Only snapshots at ground are accepted with a zero limit.
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.vt_write(b"hi");
        let mut bytes = Vec::new();
        terminal.encode_snapshot(&mut bytes).unwrap();

        let mut decoder = Decoder::new_buf(&bytes).unwrap();
        decoder
            .set_max_continuation_bytes(0)
            .unwrap()
            .set_retain_continuation(true)
            .unwrap();
        let restored = decoder.decode().unwrap();
        assert_eq!(restored.continuation_max_bytes().unwrap(), 0);
    }

    /// Restoring leaves history uncompressed unless the option asks for it.
    /// This goes through the one-shot [`Decoder::decode`], which upstream runs
    /// as READY followed by every `next`, so the option applies there too.
    #[test]
    fn decode_compresses_history_when_enabled() {
        // Mirrors upstream's own test: at 215 columns a page holds 215 rows, so
        // 3 pages' worth of lines leaves several history pages above the
        // active area. The restored viewport shows the active area, so every
        // one of them is off screen and eligible. This is much cheaper than
        // `snapshot_with_history`, which formats and writes a line at a time.
        let mut terminal = Terminal::new(215, 2).unwrap();
        // Lift the default byte limit, which would otherwise prune all but
        // one of those history pages.
        terminal.set_scrollback_max_bytes(Some(usize::MAX)).unwrap();
        terminal.vt_write(&b"x\r\n".repeat(215 * 3));
        let mut bytes = Vec::new();
        terminal.encode_snapshot(&mut bytes).unwrap();

        // Left at its default, the option compresses nothing.
        let plain = Decoder::new_buf(&bytes).unwrap().decode().unwrap();
        assert_eq!(plain.memory_usage().unwrap().primary_compressed_pages, 0);

        let mut decoder = Decoder::new_buf(&bytes).unwrap();
        assert!(!decoder.is_compress_history_enabled().unwrap());
        decoder.set_compress_history_enabled(true).unwrap();
        assert!(decoder.is_compress_history_enabled().unwrap());
        // Turning it back off must stick too, not just turning it on.
        decoder.set_compress_history_enabled(false).unwrap();
        assert!(!decoder.is_compress_history_enabled().unwrap());
        decoder.set_compress_history_enabled(true).unwrap();
        let compressed = decoder.decode().unwrap().memory_usage().unwrap();
        // Upstream accepts the option where compression is unsupported, but it
        // has no effect there, so only expect compressed pages where it is.
        if compressed.compression_supported {
            assert!(compressed.primary_compressed_pages > 0);
        } else {
            assert_eq!(compressed.primary_compressed_pages, 0);
        }
    }

    /// The decoder starts with a new terminal's storage limit, not some
    /// value of its own, and whatever limit it holds is the one the restored
    /// terminal ends up with.
    #[test]
    #[cfg(feature = "kitty-graphics")]
    fn kitty_image_storage_limit_defaults_to_a_new_terminals_and_is_applied() {
        // Taken from a fresh terminal rather than the decoder, so a decoder
        // that made up its own default would fail here.
        let default = Terminal::new(8, 2)
            .unwrap()
            .kitty_image_storage_limit()
            .unwrap()
            .expect("kitty graphics enabled");
        let bytes = unfinished_snapshot();

        let mut decoder = Decoder::new_buf(&bytes).unwrap();
        assert_eq!(decoder.kitty_image_storage_limit().unwrap(), default);
        // Anything but the default, so a setter that did nothing, or a
        // restore that ignored it, can't pass by accident.
        let limit = default / 2 + 1;
        decoder.set_kitty_image_storage_limit(limit).unwrap();
        assert_eq!(decoder.kitty_image_storage_limit().unwrap(), limit);

        let restored = decoder.decode().unwrap();
        assert_eq!(restored.kitty_image_storage_limit().unwrap(), Some(limit));
    }

    /// Images are restored oldest first, so when they don't all fit within
    /// the decoder's storage limit, the newest evicts the older ones, exactly
    /// as a live transmission would. Mirrors upstream's "decode with a
    /// smaller limit keeps the newest images" test in
    /// `src/terminal/snapshot/kitty.zig`.
    ///
    /// This also checks that the limit is in force *while* images are
    /// restored, not just set on the terminal afterwards: the two orders
    /// evict different images. Upstream evicts unplaced images before placed
    /// ones, and oldest first among equals. The decoder restores every image
    /// before any placement, so while 8 is restored neither image is placed
    /// yet and the older 7 goes. Lowering the limit after the restore, with
    /// 7 placed and 8 not, would evict 8 instead.
    #[test]
    #[cfg(feature = "kitty-graphics")]
    fn kitty_image_storage_limit_keeps_the_newest_images() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        // Two copies of the 2x1 RGBA image from
        // `next_reports_kitty_image_pixels`, 8 bytes each, sent oldest (7)
        // first. 7 is transmitted and placed (`a=T`), 8 only transmitted
        // (`a=t`), so that a limit applied after the restore would keep 7.
        terminal.vt_write(b"\x1b_Ga=T,f=32,s=2,v=1,i=7,q=2;AAAA/wAAAP8=\x1b\\");
        terminal.vt_write(b"\x1b_Ga=t,f=32,s=2,v=1,i=8,q=2;AAAA/wAAAP8=\x1b\\");
        let mut bytes = Vec::new();
        terminal.encode_snapshot(&mut bytes).unwrap();

        // The control: at the default limit both images survive, placed or
        // not, so the absence below is down to the limit and not the
        // snapshot.
        let kept = Decoder::new_buf(&bytes).unwrap().decode().unwrap();
        let graphics = kept
            .kitty_graphics()
            .unwrap()
            .expect("kitty graphics enabled");
        assert!(graphics.image(7).is_some());
        assert!(graphics.image(8).is_some());

        // Room for exactly one image: nonzero, so Kitty graphics stay
        // enabled, and big enough that the newer image fits on its own, so
        // losing the older one can only be eviction.
        let mut decoder = Decoder::new_buf(&bytes).unwrap();
        decoder.set_kitty_image_storage_limit(8).unwrap();
        let restored = decoder.decode().unwrap();
        let graphics = restored
            .kitty_graphics()
            .unwrap()
            .expect("kitty graphics enabled");
        assert!(graphics.image(7).is_none());
        assert!(graphics.image(8).is_some());
    }

    /// The decoder stays reachable through the whole incremental decode, so
    /// READY metadata can be read before the first page and the end of the
    /// snapshot can be located after FINISH.
    #[test]
    fn decoder_is_queryable_before_history_and_after_finish() {
        // Same cheap history-bearing terminal as
        // `decode_compresses_history_when_enabled`: several history pages
        // remain to be decoded after READY.
        let mut terminal = Terminal::new(215, 2).unwrap();
        terminal.set_scrollback_max_bytes(Some(usize::MAX)).unwrap();
        terminal.vt_write(&b"x\r\n".repeat(215 * 3));
        let history = terminal.scrollback_rows().unwrap();
        let mut buf = Vec::new();
        terminal.encode_snapshot(&mut buf).unwrap();
        // Record the length before appending, so the expected offset doesn't
        // come from the decoder itself. The trailing bytes make the end of
        // the snapshot differ from the end of the buffer.
        let snapshot_len = buf.len();
        buf.extend_from_slice(b"trailing");

        let mut incremental = Decoder::new_buf(&buf).unwrap().ready().unwrap();
        // No page has been decoded yet, so there is no `Progress` to reach the
        // decoder through.
        assert_eq!(
            incremental.decoder().history_rows_primary().unwrap(),
            history as u64
        );
        let mut pages = 0;
        while incremental.next().unwrap().is_some() {
            pages += 1;
        }
        assert!(pages > 0);

        let offset = incremental.decoder().source_offset().unwrap();
        assert_eq!(offset, snapshot_len);
        assert_eq!(&buf[offset..], b"trailing");
    }

    /// Length of a record header: u16 tag, u32 payload length, u32 CRC32C.
    const RECORD_HEADER_LEN: usize = 10;

    fn encoded_snapshot() -> Vec<u8> {
        let mut terminal = Terminal::new(20, 5).expect("terminal should initialize");
        terminal.vt_write(b"hello\r\nworld");
        let bytes = terminal
            .encode_snapshot_alloc(None)
            .expect("snapshot should encode")
            .expect("snapshot should not be empty");
        bytes.to_vec()
    }

    #[test]
    fn finish_is_an_empty_marker_record() {
        let bytes = encoded_snapshot();

        // The envelope is "GHOSTSNP" followed by the u16 format version.
        assert_eq!(&bytes[..8], b"GHOSTSNP");
        assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 1);

        // FINISH is the final record and carries no payload, so the stream
        // ends with its bare header. Its declared payload length is zero.
        let finish = &bytes[bytes.len() - RECORD_HEADER_LEN..];
        assert_eq!(&finish[2..6], &0u32.to_le_bytes());

        assert!(Decoder::new_buf(&bytes).unwrap().decode().is_ok());
    }

    #[test]
    fn truncated_snapshot_is_invalid_value() {
        let bytes = encoded_snapshot();

        // Dropping FINISH means end-of-file before the required marker, which
        // the snapshot contract documents as `Error::InvalidValue`.
        let truncated = &bytes[..bytes.len() - RECORD_HEADER_LEN];
        let result = Decoder::new_buf(truncated).unwrap().decode();
        assert!(matches!(result, Err(Error::InvalidValue)));
    }

    #[test]
    fn corrupted_record_fails_to_decode() {
        let mut bytes = encoded_snapshot();

        // Every record, including the empty FINISH marker, is protected by
        // CRC32C. Flipping a bit in FINISH's checksum must be detected.
        let mut finish_crc = bytes.clone();
        let last = finish_crc.len() - 1;
        finish_crc[last] ^= 0x01;
        assert!(Decoder::new_buf(&finish_crc).unwrap().decode().is_err());

        // Likewise for a payload byte of the first (TERMINAL) record, which
        // starts right after the ten-byte envelope and its record header.
        let payload = 10 + RECORD_HEADER_LEN;
        bytes[payload] ^= 0x01;
        assert!(Decoder::new_buf(&bytes).unwrap().decode().is_err());
    }
}
