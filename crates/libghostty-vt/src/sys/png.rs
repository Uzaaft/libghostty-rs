use std::mem::ManuallyDrop;

use crate::{
    alloc::{Allocator, Bytes},
    ffi,
};

/// A PNG decoder that can be used by the Kitty graphics protocol
/// to decode PNG images into 8-bit RGBA pixels.
///
/// See [`sys::PNG_DECODER`](PNG_DECODER) for more details.
pub trait DecodePng: Sync + 'static {
    /// Decode a PNG into 8-bit RGBA pixels.
    ///
    /// The library calls this when it receives a PNG image and needs the raw
    /// pixels. The decoder decodes the PNG bytes in `data` and returns the
    /// result. See the [Kitty graphics example](crate::kitty::graphics#example)
    /// for a complete callback.
    ///
    /// # On success
    ///
    /// Allocate the pixel buffer with [`Bytes::new_zeroed_in`] and `alloc`,
    /// write the decoded pixels into it, and return it in a [`DecodedImage`].
    /// The library then owns the buffer and frees it with the same allocator.
    /// See [`DecodedImage`] for the expected pixel layout.
    ///
    /// The allocator limits how much memory a single image may use, so
    /// [`Bytes::new_zeroed_in`] can fail for very large images. Treat that as
    /// a failure.
    ///
    /// # On failure
    ///
    /// Return `None`, and the image is rejected. An image whose buffer comes
    /// from any allocator other than `alloc` is also rejected, as if decoding
    /// failed.
    fn decode_png<'alloc>(
        &self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>>;
}

/// The PNG decoder for the [Kitty graphics protocol](crate::kitty::graphics).
///
/// When installed, the terminal can accept PNG images via the Kitty graphics
/// protocol. Otherwise, PNG decoding is unsupported and PNG image data
/// will be rejected.
///
/// This is a process-global setting. It is simplest to set it once at
/// startup, before using any terminal functionality that depends on it.
pub static PNG_DECODER: super::SysFunc<dyn DecodePng> = super::SysFunc::new(
    ffi::SysOption::DECODE_PNG,
    trampoline as Trampoline as *const std::ffi::c_void,
);

type Trampoline = unsafe extern "C" fn(
    _userdata: *mut std::ffi::c_void,
    allocator: *const ffi::Allocator,
    data: *const u8,
    data_len: usize,
    out: *mut ffi::SysImage,
) -> bool;

unsafe extern "C" fn trampoline(
    _userdata: *mut std::ffi::c_void,
    allocator: *const ffi::Allocator,
    data: *const u8,
    data_len: usize,
    out: *mut ffi::SysImage,
) -> bool {
    let Some(decoder) = PNG_DECODER.get() else {
        return false;
    };

    // SAFETY: We trust libghostty to return valid values.
    let alloc = unsafe { Allocator::from_raw(allocator) };
    let data = unsafe { std::slice::from_raw_parts(data, data_len) };

    match decoder.decode_png(&alloc, data) {
        // libghostty frees the buffer with the allocator it passed
        // in, so a buffer from any other allocator would be freed
        // with the wrong one. `DecodePng` is a safe trait and
        // `Bytes::new_zeroed` type-checks here, so reject such an image
        // instead of trusting the decoder; dropping it frees the
        // buffer with its own allocator.
        Some(result) if result.data.allocator() != alloc.to_raw() => false,
        Some(result) => {
            // IMPORTANT: Do NOT run the Rust destructor here
            // to avoid double-freeing the byte buffer.
            let mut result = ManuallyDrop::new(result);
            unsafe {
                *out = ffi::SysImage {
                    width: result.width,
                    height: result.height,
                    data: result.data.as_mut_ptr(),
                    data_len: result.data.len(),
                }
            };
            true
        }
        None => false,
    }
}

/// A PNG decoder for [`sys::PNG_DECODER`](PNG_DECODER) using the [`png`] crate.
///
/// ```rust
/// use libghostty_vt::kitty::graphics;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // SAFETY: No other thread uses libghostty yet.
/// unsafe { graphics::set_png_decoder(Some(Box::new(graphics::RustPngDecoder::new()))) }?;
/// # Ok(())
/// # }
/// ```
#[cfg(all(feature = "kitty-graphics", feature = "png"))]
#[derive(Clone, Debug, Default)]
// Keep the decoder constructible only through `new` and `Default`, so that
// it can grow internal state later without breaking callers.
#[non_exhaustive]
pub struct RustPngDecoder;
#[cfg(all(feature = "kitty-graphics", feature = "png"))]
impl RustPngDecoder {
    /// Create a PNG decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
#[cfg(all(feature = "kitty-graphics", feature = "png"))]
impl DecodePng for RustPngDecoder {
    fn decode_png<'alloc>(
        &self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        use png::{BitDepth, ColorType, Decoder, Transformations};
        use std::io::Cursor;

        use crate::alloc::Bytes;

        let mut decoder = Decoder::new(Cursor::new(data));

        // libghostty only accepts RGBA8 data, so we have to apply some
        // transformations to accept images in other formats, namely
        // expanding palette colors, adding alpha, and stripping
        // 16-bit color depth information back down into 8-bit.
        decoder.set_transformations(Transformations::ALPHA | Transformations::STRIP_16);

        let mut reader = decoder.read_info().ok()?;
        let (width, height) = reader.info().size();
        let pixels = (width as usize).checked_mul(height as usize)?;
        let rgba_len = pixels.checked_mul(4)?;

        // Allocate the output with the provided allocator *before* decoding
        // anything. libghostty caps what the decoder may allocate, and the
        // dimensions come straight from untrusted PNG headers: decoding into
        // an image-sized buffer on the global heap would sidestep that cap
        // and let a tiny PNG claim gigabytes. (The png crate's own row and
        // zlib buffers stay on the global heap, but they are bounded by the
        // row length and the compressed size, not the image size.)
        let mut bytes = Bytes::new_zeroed_in(alloc, rgba_len).ok()?;

        // `ALPHA` expands palettes and adds alpha, but keeps grayscale as two
        // channels, so the decoded output is either RGBA8 or GA8. Both fit in
        // the RGBA buffer, so decode into its prefix and expand GA in place.
        let (decoded_len, gray_alpha) = match reader.output_color_type() {
            (ColorType::Rgba, BitDepth::Eight) => (rgba_len, false),
            (ColorType::GrayscaleAlpha, BitDepth::Eight) => (pixels.checked_mul(2)?, true),
            _ => return None,
        };
        // Bail out instead of letting slicing panic below, since a panic
        // here would abort through the `extern "C"` callback.
        if reader.output_buffer_size()? != decoded_len {
            return None;
        }

        let info = reader.next_frame(&mut bytes[..decoded_len]).ok()?;
        if info.buffer_size() != decoded_len {
            return None;
        }
        reader.finish().ok()?;

        if gray_alpha {
            // Walk backwards: pixel `i` is read from `2i..2i + 2` and written
            // to `4i..4i + 4`, which never overlaps a lower pixel that has not
            // been read yet.
            for i in (0..pixels).rev() {
                let (luma, alpha) = (bytes[2 * i], bytes[2 * i + 1]);
                bytes[4 * i..4 * i + 4].copy_from_slice(&[luma, luma, luma, alpha]);
            }
        }

        Some(DecodedImage {
            width,
            height,
            data: bytes,
        })
    }
}

/// A decoded image, returned by a decoder such as [`DecodePng`].
///
/// Pixels are 8-bit RGBA: four bytes per pixel, stored row by row starting
/// at the top-left corner, with no padding between rows. A complete image
/// is therefore `width * height * 4` bytes long.
///
/// The pixel buffer must be allocated with the allocator passed to
/// [`DecodePng::decode_png`]. When the decoder returns the image and its
/// buffer came from that allocator, the library takes ownership of the
/// buffer and frees it with that same allocator.
#[derive(Debug)]
pub struct DecodedImage<'alloc> {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// The decoded RGBA pixels, allocated with the allocator passed to
    /// [`DecodePng::decode_png`].
    pub data: Bytes<'alloc>,
}
