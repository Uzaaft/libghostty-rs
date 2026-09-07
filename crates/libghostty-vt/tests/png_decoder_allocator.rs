//! A safe `DecodePng` can return a buffer from an allocator other than the
//! one libghostty passed in, and libghostty would free it with the wrong
//! one. The decoder hook is process-global, so this lives in its own test
//! binary.
#![cfg(feature = "kitty-graphics")]

use std::{cell::RefCell, rc::Rc};

use libghostty_vt::{
    Terminal,
    alloc::{Allocator, Bytes},
    kitty::graphics::{self, DecodePng, DecodedImage},
};

/// Returns a 1x1 red pixel, allocated either with the allocator libghostty
/// passed in or with libghostty's default allocator.
struct OnePixel {
    use_given_allocator: bool,
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
    graphics::set_png_decoder(Some(Box::new(OnePixel {
        use_given_allocator: true,
    })))
    .unwrap();
    assert!(transmit_png().contains(";OK"));

    // Freeing this buffer with the terminal's allocator would be a
    // cross-allocator free, so the image must be rejected instead.
    graphics::set_png_decoder(Some(Box::new(OnePixel {
        use_given_allocator: false,
    })))
    .unwrap();
    let reply = transmit_png();
    assert!(reply.contains("i=1;"), "no reply: {reply:?}");
    assert!(!reply.contains(";OK"), "image was accepted: {reply:?}");

    graphics::set_png_decoder(None).unwrap();
}
