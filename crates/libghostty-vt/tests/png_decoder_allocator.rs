//! A safe `DecodePng` can return a buffer from an allocator other than the
//! one libghostty passed in, and libghostty would free it with the wrong
//! one. The decoder hook is process-global, so this lives in its own test
//! binary.
// These call into libghostty, which Miri cannot execute.
#![cfg(all(feature = "kitty-graphics", not(miri)))]

use std::{cell::RefCell, io::Write, rc::Rc};

use libghostty_vt::{
    Terminal,
    alloc::{Allocator, Bytes},
    kitty::graphics::{self, DecodePng, DecodedImage},
};

/// Returns a 1x1 red pixel, allocated either with the allocator libghostty
/// passed in or with libghostty's default allocator.
struct OnePixel {
    use_given_allocator: bool,
    decode_on_drop: bool,
}

impl DecodePng for OnePixel {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        _data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        let mut data = if self.use_given_allocator {
            Bytes::new_with_alloc(alloc, 4).ok()?
        } else {
            Bytes::new(4).ok()?
        };
        data.copy_from_slice(&[255, 0, 0, 255]);
        Some(DecodedImage {
            width: 1,
            height: 1,
            data,
        })
    }
}

impl Drop for OnePixel {
    fn drop(&mut self) {
        if self.decode_on_drop {
            // Destruction may use another terminal. The replacement decoder
            // must already be callable without waiting for this drop to finish.
            assert!(transmit_png().contains(";OK"));
        }
    }
}

/// Transmit a 1x1 PNG and return the terminal's reply.
fn transmit_png() -> String {
    let replies = Rc::new(RefCell::new(Vec::new()));
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.resize(80, 24, 8, 16).unwrap();
    terminal
        .set_kitty_image_storage_limit(64 * 1024 * 1024)
        .unwrap();
    let sink = Rc::clone(&replies);
    terminal
        .on_pty_write(move |_, data| sink.borrow_mut().extend_from_slice(data))
        .unwrap();
    terminal.vt_write(
        b"\x1b_Ga=t,f=100,i=1;\
          iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAA\
          DUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==\
          \x1b\\",
    );
    String::from_utf8_lossy(&replies.borrow()).into_owned()
}

#[test]
fn decoded_images_must_use_the_given_allocator() {
    // If replaced decoders are ever dropped under the decoder mutex again,
    // the decode in `OnePixel::drop` deadlocks and this test would hang
    // forever. Fail it instead. The watchdog never calls into libghostty,
    // so the `set_png_decoder` safety contract below still holds.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(30));
        // Not `eprintln!`: libtest captures that, and exiting skips printing
        // captured output, so the message would be lost.
        let _ = writeln!(
            std::io::stderr(),
            "png_decoder_allocator: timed out after 30s, likely deadlocked"
        );
        std::process::exit(1);
    });

    // SAFETY: No other thread in this test binary uses libghostty.
    unsafe {
        graphics::set_png_decoder(Some(Box::new(OnePixel {
            use_given_allocator: true,
            decode_on_drop: true,
        })))
    }
    .unwrap();
    assert!(transmit_png().contains(";OK"));

    // Replacing the decoder must allow its destructor to decode PNG input.
    // SAFETY: As above.
    unsafe {
        graphics::set_png_decoder(Some(Box::new(OnePixel {
            use_given_allocator: true,
            decode_on_drop: false,
        })))
    }
    .unwrap();

    // Freeing this buffer with the terminal's allocator would be a
    // cross-allocator free, so the image must be rejected instead.
    // SAFETY: As above.
    unsafe {
        graphics::set_png_decoder(Some(Box::new(OnePixel {
            use_given_allocator: false,
            decode_on_drop: false,
        })))
    }
    .unwrap();
    let reply = transmit_png();
    assert!(reply.contains("i=1;"), "no reply: {reply:?}");
    assert!(!reply.contains(";OK"), "image was accepted: {reply:?}");

    // SAFETY: As above.
    unsafe { graphics::set_png_decoder(None) }.unwrap();
}
