//! Encode a terminal's full state and restore it three ways: in one shot,
//! with scrollback kept compressed, and page by page. The page by page path
//! gives you a terminal you can draw before the old history arrives.
#![allow(unused_variables)]

use libghostty_vt::{
    error::Error,
    snapshot::{Decoder, ProgressKind},
    Terminal,
};
use std::io::Read;

// ANCHOR: snapshot-buffer-reader
struct BufferReader<'a> {
    data: &'a [u8],
    offset: usize,
}
impl Read for BufferReader<'_> {
    // GhosttyReader callbacks are synchronous. A successful zero-byte read is
    // permanent EOF; returning false would report an I/O error.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.data.len() - self.offset;
        let count = remaining.min(buf.len());

        // Deliberately return short reads to demonstrate that the decoder retries.
        let count = count.min(64);
        buf[0..count].copy_from_slice(&self.data[self.offset..][0..count]);
        self.offset += count;
        Ok(count)
    }
}
// ANCHOR_END: snapshot-buffer-reader

fn main() -> Result<(), Error> {
    // ANCHOR: snapshot-encode
    // A wide, shallow screen fills backing pages quickly enough to leave older
    // PAGE records after READY for the incremental decoder to demonstrate.
    let mut source = Terminal::new(215, 2)?;

    // Snapshot encoding requires continuation tracking if the input can stop
    // partway through a sequence, and it has to be enabled before feeding
    // input. The limit bounds unfinished VT sequence retention.
    source.set_continuation_max_bytes(1024)?;

    // Keep enough scrollback to demonstrate incremental history restoration.
    source.set_scrollback_max_bytes(Some(usize::MAX))?;

    for _ in 0..1000 {
        source.vt_write(b"snapshot history line\r\n");
    }

    // Leave an SGR sequence unfinished so its continuation is snapshotted too.
    source.vt_write(b"\x1b[31");

    let snapshot = source
        .encode_snapshot_alloc(None)?
        .expect("snapshot should succeed");
    println!("encoded {} snapshot bytes", snapshot.len());
    // ANCHOR_END: snapshot-encode

    // ANCHOR: snapshot-decode
    let full_decoder = Decoder::new_buf(&snapshot)?;
    let full_terminal = full_decoder.decode()?;
    // ANCHOR_END: snapshot-decode

    // ANCHOR: snapshot-compress-history
    let mut compressed_decoder = Decoder::new_buf(&snapshot)?;

    // Options must be set before decoding starts. With this set, each history
    // page is compressed as soon as it is restored, so the full scrollback is
    // never held uncompressed in memory.
    compressed_decoder.set_compress_history_enabled(true)?;

    let compressed_terminal = compressed_decoder.decode()?;
    // ANCHOR_END: snapshot-compress-history

    // ANCHOR: snapshot-incremental
    let mut reader = BufferReader {
        data: &snapshot,
        offset: 0,
    };
    let incremental_decoder = Decoder::new(&mut reader)?;

    // READY returns a validated, renderable terminal before old history.
    let mut incremental_decoder = incremental_decoder.ready()?;
    let incremental_terminal = incremental_decoder.terminal();

    println!(
        "snapshot advertises {} primary history rows",
        incremental_decoder.decoder().history_rows_primary()?,
    );

    let mut page_count = 0usize;
    while let Some(progress) = incremental_decoder.next()? {
        // Each call decodes either a history page or the pixels of one Kitty
        // graphics image. This terminal has no images, but a real client should
        // check the kind before reading the page values.
        match progress.kind()? {
            ProgressKind::Image { id, .. } => {
                println!("restored the pixels of image {}", id);
            }
            ProgressKind::Page { rows, remaining } => {
                println!(
                    "restored {} rows to screen {:?} ({} pages remain)",
                    rows,
                    progress.screen()?,
                    remaining
                );
            }
            _ => {}
        }
        page_count += 1;
    }

    // None means FINISH validated successfully and is idempotent.
    assert!(page_count > 0);
    assert!(incremental_decoder.next()?.is_none());
    // ANCHOR_END: snapshot-incremental

    Ok(())
}
