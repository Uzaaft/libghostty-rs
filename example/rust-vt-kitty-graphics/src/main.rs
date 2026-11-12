use libghostty_vt::{
    alloc::{Allocator, Bytes},
    error::Error,
    kitty::graphics::{self, Graphics},
    render::{ImageIterator, ImageProtocol, RenderState, Snapshot},
    Terminal,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

// ANCHOR: kitty-graphics-decode-png
/// Minimal PNG decoder callback for the sys interface.
///
/// A real implementation would use a PNG library (libpng, stb_image, etc.)
/// to decode the PNG data. This example uses a hardcoded 1x1 red pixel
/// since we know exactly what image we're sending.
///
/// WARNING: This is only an example for providing a callback, it DOES NOT
/// actually decode the PNG it is passed. It hardcodes a response.
struct StubPngDecoder {
    decode_count: Arc<AtomicUsize>,
}

impl graphics::DecodePng for StubPngDecoder {
    fn decode_png<'alloc, 'ctx>(
        &mut self,
        alloc: &'alloc Allocator<'ctx>,
        data: &[u8],
    ) -> Option<graphics::DecodedImage<'alloc>> {
        _ = self.decode_count.fetch_add(1, Ordering::SeqCst);
        let count = self.decode_count.load(Ordering::SeqCst);

        println!("  decode_png called (size={}, call #{count})", data.len());

        // Allocate RGBA pixel data through the provided allocator.
        let mut data = Bytes::new_zeroed_with_alloc(alloc, 4).ok()?;

        // Fill with red (R=255, G=0, B=0, A=255).
        data.copy_from_slice(&[255, 0, 0, 255]);
        Some(graphics::DecodedImage {
            width: 1,
            height: 1,
            data,
        })
    }
}
// ANCHOR_END: kitty-graphics-decode-png

// ANCHOR: kitty-graphics-animation-tick
/// Advance any playing animations while preparing a frame to draw.
///
/// Call this once per frame, before drawing images. It returns how many
/// milliseconds to wait before the next redraw, or -1 if no animation is
/// playing. In that case, don't schedule a redraw for animations at all.
/// A program that starts an animation does so by writing to the
/// terminal, which already causes a redraw.
///
/// `now_ms` comes from a clock you choose, such as milliseconds since
/// your app started. Always use the same clock with the same starting
/// point.
fn tick_animations(terminal: &mut Terminal<'_, '_>, now_ms: u64) -> Option<u64> {
    /// The shortest time between two redraws. About 8 ms suits a 120 Hz display.
    pub const MIN_FRAME_INTERVAL_MS: u64 = 8;

    let Ok(Some(next_ms)) = terminal.kitty_graphics_animation_tick(now_ms) else {
        return None;
    };

    // next_ms is an absolute time. Turn it into a delay from now.
    Some(next_ms.saturating_sub(now_ms).min(MIN_FRAME_INTERVAL_MS))
}
// ANCHOR_END: kitty-graphics-animation-tick

// ANCHOR: kitty-graphics-animation-play
/// Print the first pixel and the generation of an image.
fn print_image(graphics: &Graphics<'_>, image_id: u32) {
    let Some(image) = graphics.image(image_id) else {
        return;
    };

    // The image may be gone, and the first pixel below needs three bytes.
    let Ok(Some([p0, p1, p2, ..])) = image.data() else {
        print!("no pixels");
        return;
    };

    let generation = image.generation().unwrap_or(0);
    print!("pixel=({p0},{p1},{p2}) generation={generation}");
}

/// Play a two-frame animation using tick_animations().
///
/// A real app waits for the delay that tick_animations() returns. To keep
/// this example quick, a pretend clock jumps ahead by that delay instead.
fn run_animation(terminal: &mut Terminal<'_, '_>) -> Result<(), Error> {
    // These commands set up the animation:
    //
    //   a=T  Send a red 1x1 RGBA image with ID 2 and display it.
    //   a=f  Add a blue second frame that shows for 40 ms.
    //   a=a  Make the first frame show for 30 ms. It starts at 0 ms,
    //        which would mean the frame is skipped.
    //   a=a  Start playing on a loop (s=3).
    //
    // q=2 tells the terminal not to send replies.
    terminal.vt_write(b"\x1b_Ga=T,f=32,i=2,s=1,v=1,q=2;/wAA/w==\x1b\\");
    terminal.vt_write(b"\x1b_Ga=f,f=32,i=2,s=1,v=1,z=40,q=2;AAD//w==\x1b\\");
    terminal.vt_write(b"\x1b_Ga=a,i=2,r=1,z=30,q=2\x1b\\");
    terminal.vt_write(b"\x1b_Ga=a,i=2,s=3,q=2\x1b\\");

    // Draw three frames, following each delay.
    let mut now_ms = 0u64;
    for _ in 0..3 {
        let delay_ms = tick_animations(terminal, now_ms).expect("tick should succeed");

        // The tick invalidates image handles, so print_image() looks the
        // image up again. The generation changes whenever the frame does.
        print!("  t={now_ms} ms: ");
        let graphics = terminal
            .kitty_graphics()?
            .expect("graphics should be enabled");
        print_image(&graphics, 2);
        println!(", next redraw in {delay_ms} ms");

        now_ms += delay_ms;
    }

    // Stop playback (s=1). Once stopped, the tick asks for no redraws.
    terminal.vt_write(b"\x1b_Ga=a,i=2,s=1,q=2\x1b\\");

    // Writing to the terminal invalidates the storage handle. Get it again.
    if tick_animations(terminal, now_ms).is_none() {
        println!("  t={now_ms} ms: stopped, no redraw needed");
    }

    Ok(())
}
// ANCHOR_END: kitty-graphics-animation-play

// ANCHOR: kitty-graphics-draw
/// Read the render state's image list, the way a renderer builds its list
/// of images to draw. Returns the number of images, or -1 on error.
///
/// The list holds every Kitty image on screen, however the program placed
/// it, with its position, size and draw order worked out. The Kitty
/// graphics storage is only used to read pixels.
///
/// A real renderer would find or upload a texture for each image and
/// queue it for drawing, sorted by layer and then z. This example prints
/// what it would draw instead.
fn draw_images<'alloc>(
    snapshot: &Snapshot<'alloc, '_>,
    images: &mut ImageIterator<'alloc>,
    terminal: &Terminal<'alloc, '_>,
) -> Result<usize, Error> {
    // Fill the iterator from the last update.
    let mut image_it = images.update(snapshot)?;

    // The storage is only needed to read pixels.
    let graphics = terminal
        .kitty_graphics()?
        .expect("graphics should be enabled");

    let mut count = 0usize;
    while let Some(image) = image_it.next() {
        // Skip protocols this renderer doesn't support.
        let Ok(ImageProtocol::Kitty) = image.protocol() else {
            continue;
        };
        count += 1;

        println!(
            "  image {count}: kind={:?} image_id={} placement_id={:?}",
            image.kitty_kind()?,
            image.image_id()?,
            image.kitty_placement_id()?
        );
        println!(
            "    layer={:?} z={} cell=({},{}) cells={}x{} offset=({},{})",
            image.layer()?,
            image.z()?,
            image.viewport_col()?,
            image.viewport_row()?,
            image.grid_cols()?,
            image.grid_rows()?,
            image.cell_offset_x()?,
            image.cell_offset_y()?,
        );
        println!(
            "    size={}x{} source=({},{} {}x{})",
            image.pixel_width()?,
            image.pixel_height()?,
            image.source_x()?,
            image.source_y()?,
            image.source_width()?,
            image.source_height()?,
        );

        // Read the pixels to upload them as a texture. A real renderer keys
        // its textures by protocol, image ID and image generation, and only
        // gets here when it has no texture for that key yet. The generation
        // changes whenever the pixels do, so most frames skip this.
        let Some(image) = graphics.image(image.image_id()?) else {
            continue;
        };
        println!(
            "    pixels: {}x{} format={:?} data_len={} generation={}",
            image.width()?,
            image.height()?,
            image.format()?,
            image.data_len()?,
            image.generation()?,
        );
    }
    Ok(count)
}
// ANCHOR_END: kitty-graphics-draw

// ANCHOR: kitty-graphics-frame
/// Draw three frames, reading the image list only when it changed.
///
/// Each frame updates the render state and compares the placements
/// generation with the one from the last time the list was read. The
/// first frame always reads the list. The second frame changes nothing,
/// so it keeps the list it has. Before the third frame, new output
/// scrolls the screen by two rows, which moves both images, so the third
/// frame reads the list again.
fn run_frames(terminal: &mut Terminal<'_, '_>) -> Result<(), Error> {
    // Create the render state and the image iterator once, and reuse them.
    let mut state = RenderState::new()?;
    let mut images = ImageIterator::new()?;

    // The flag makes sure the first frame always reads the list.
    let mut last_generation = 0u64;
    let mut have_draw_list = false;

    for frame in 1..=3 {
        if frame == 3 {
            // Move to the bottom row and print two newlines to scroll.
            terminal.vt_write(b"\x1b[24;1H\r\n\r\n");
        }
        let snapshot = state.update(&terminal)?;
        let generation = snapshot.image_placements_generation()?;

        if have_draw_list && generation == last_generation {
            println!("\nFrame {frame}: no image changed, keep drawing the last list");
            continue;
        }
        last_generation = generation;
        have_draw_list = true;

        println!("\nFrame {frame}: images changed, reading the list:");

        let count = draw_images(&snapshot, &mut images, &terminal)?;
        println!("Total images: {count}");
    }

    Ok(())
}
// ANCHOR_END: kitty-graphics-frame

// ANCHOR: kitty-graphics-main
fn main() -> Result<(), Error> {
    // Install the PNG decoder.
    let decode_count = Arc::new(AtomicUsize::new(0));
    unsafe {
        graphics::set_png_decoder(Some(Box::new(StubPngDecoder {
            decode_count: Arc::clone(&decode_count),
        })))?;
    }

    // Create a terminal with Kitty graphics enabled.
    let mut terminal = Terminal::new(80, 24)?;

    // Set the cell size in pixels. Image sizes depend on it.
    terminal.resize(80, 24, 8, 16)?;

    // Set a storage limit to enable Kitty graphics.
    terminal.set_kitty_image_storage_limit(64 * 1024 * 1024)?; // 64 MiB

    // Install write_pty to see the protocol response.
    // The Kitty graphics protocol sends an APC response back to the pty
    // when an image is loaded (unless suppressed with q=2).
    terminal.on_pty_write(|_term, data| {
        println!("  response ({} bytes): {}", data.len(), data.escape_ascii());
    })?;

    // Move the cursor to row 10 so the images have room to scroll up.
    terminal.vt_write(b"\x1b[10;1H");

    // Send a 1x1 PNG image and place it at the cursor. The image is
    // attached to that cell, so it moves when the text moves.
    //
    // The escape sequence is:
    //   ESC _G a=T,f=100,q=1; <base64 PNG data> ESC \
    //
    // Where:
    //   a=T    transmit the image and place it
    //   f=100  the data is a PNG
    //   q=1    only reply on errors (q=0 would also reply on success)
    println!("Sending Kitty graphics PNG image:");
    terminal.vt_write(b"\x1b_Ga=T,f=100,q=1;");
    terminal.vt_write(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAA");
    terminal.vt_write(b"DUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==");
    terminal.vt_write(b"\x1b\\");

    println!("PNG decode calls: {}", decode_count.load(Ordering::SeqCst));

    // Show a second image with placeholder cells. First send a green 1x1
    // image with ID 3 and give it a virtual placement 2 columns wide and
    // 1 row tall (U=1). Then, on the next row, print two placeholder
    // characters (U+10EEEE) where the image should appear:
    //
    //   ESC[38;5;3m    the foreground color says which image: ID 3
    //   U+0305 U+0305  the first cell shows row 0, column 0 of the image
    //   U+0305 U+030D  the second cell shows row 0, column 1
    //
    // The image moves with those cells, like text.
    terminal.vt_write(b"\x1b_Ga=T,U=1,f=32,i=3,s=1,v=1,c=2,r=1,q=2;AP8A/w==\x1b\\");
    terminal.vt_write(b"\r\n\x1b[38;5;3m");
    terminal.vt_write(b"\xf4\x8e\xbb\xae\xcc\x85\xcc\x85");
    terminal.vt_write(b"\xf4\x8e\xbb\xae\xcc\x85\xcc\x8d");
    terminal.vt_write(b"\x1b[39m");

    // The storage has a generation of its own. It changes when a program
    // sends, places or deletes an image, but not when images only move on
    // screen. That is why the frames below use the render state's
    // placements generation to know when to read the list again.
    let graphics = terminal
        .kitty_graphics()?
        .expect("graphics should be enabled");
    let storage_generation = graphics.generation()?;
    println!("\nStorage generation: {storage_generation}");

    // Find what to draw from the render state, over three frames.
    run_frames(&mut terminal)?;

    // Animate a second image by driving the animation tick.
    println!("\nPlaying an animation:");
    run_animation(&mut terminal)?;

    Ok(())
}
// ANCHOR_END: kitty-graphics-main
