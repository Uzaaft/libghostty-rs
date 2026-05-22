//! A Rust port of Ghostling, using the `ghostty` crate for safe, idiomatic
//! `libghostty-vt` Rust bindings, and macroquad instead of raylib for
//! window management and rendering.
//!
//! This is quite a feature-rich implementation of a terminal emulator in
//! spite of its ~1kLoC size, which goes to show how much heavy-lifting
//! `libghostty-vt` is able to achieve behind a simple interface.

#![deny(unsafe_code)] // Well, almost.
use std::cell::Cell;

use macroquad::{
    miniquad::{conf, window::order_quit},
    prelude::*,
};
use nix::sys::wait;

use libghostty_vt::{
    Terminal, TerminalOptions,
    alloc::Bytes,
    build_info,
    key::{self, Key},
    kitty::graphics::{self, DecodePng, DecodedImage, Graphics, Layer, PlacementIterator},
    mouse,
    render::{CellIterator, Dirty, RenderState, RowIterator},
    style::RgbColor,
    terminal::{
        ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode,
        PrimaryDeviceAttributes, ScrollViewport, SecondaryDeviceAttributes, SizeReportSize,
    },
};

use crate::pty::{Child, Pty, PtyError};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// ---------------------------------------------------------------------------
// Adjustable constants for the renderer. All of these are chosen empirically,
// so feel free to tweak and tune these settings to your liking :)
// ---------------------------------------------------------------------------

/// Desired font size in logical (screen) points — the actual texture
/// will be rasterized at font_size * dpi_scale so glyphs stay crisp on
/// HiDPI / Retina displays.
const FONT_SIZE: u16 = 10;

/// Small padding from window edges
const PADDING: f32 = 6.0;

/// Horizontal gap between cells. This is usually not necessary for
/// TrueType fonts, but for bitmap fonts like macroquad's builtin font
/// this should be set to a positive value.
const CELL_GAP: f32 = 0.0;

/// Vertical gap between rows.
const ROW_GAP: f32 = 12.0;

#[macroquad::main(macroquad_conf)]
async fn main() -> Result<()> {
    let font = load_ttf_font_from_bytes(include_bytes!("../fonts/JetBrainsMono-Medium.ttf"))?;

    // Compute the initial grid from the window size
    // and measured cell metrics.
    let mut dims = Dimensions::new(Some(&font));
    let (cols, rows) = dims.grid_size();

    // Track window size so we only recalculate the grid on actual changes.
    //
    // This is kept in a Cell<T> since the terminal effect handler
    // has to keep a reference to it, and we also want to modify it in the
    // main loop below. Don't worry, this is completely safe and is just
    // here to make the Rust compiler happy.
    let grid_size = Cell::new((cols, rows));

    // Spawn a child shell connected to a pseudo-terminal.
    let (pty, mut child) = Pty::new(dims)?;

    // Install the PNG decoder so the terminal can handle PNG images in the
    // Kitty Graphics Protocol. This is a process-global setting and must be
    // done before any terminal is created.
    graphics::set_png_decoder(Some(Box::new(PngDecoder)))?;

    // Create a ghostty virtual terminal with the computed grid and 1000
    // lines of scrollback.  This holds all the parsed screen state (cells,
    // cursor, styles, modes) but knows nothing about the pty or the window.
    let mut terminal = Terminal::new(TerminalOptions {
        cols,
        rows,
        max_scrollback: 1000,
    })?;

    // The terminal options don't include cell pixel dimensions, so
    // issue an initial resize to set them.  Without this, Kitty
    // graphics placement_rect would divide by zero cell sizes.
    terminal.resize(cols, rows, dims.cell_width as u32, dims.cell_height as u32)?;

    terminal
        // Enable Kitty graphics by setting a storage limit.  Without this the
        // terminal rejects all image data.  64 MiB is a generous default.
        .set_kitty_image_storage_limit(64 * 1024 * 1024)?
        // Allow images to be transmitted via file, temp file, and shared
        // memory mediums in addition to the default inline (direct) medium.
        .set_kitty_image_from_file_allowed(true)?
        .set_kitty_image_from_temp_file_allowed(true)?
        .set_kitty_image_from_shared_mem_allowed(true)?;

    // Register effects so the terminal can respond to VT queries (device
    // attributes, mode reports, size queries, etc.) that programs like
    // vim, tmux, and htop send during startup.  Without these, query
    // sequences are silently dropped and those programs may hang or
    // fall back to degraded behavior.
    terminal
        // write_pty effect — the terminal calls this whenever a VT sequence
        // requires a response back to the application (device status reports,
        // mode queries, device attributes, etc.).  Without this, programs like
        // vim and tmux that probe terminal capabilities would hang.
        .on_pty_write(|_t, data| pty.write(data))?
        // size effect — responds to XTWINOPS size queries (CSI 14/16/18 t)
        // so programs can discover the terminal geometry in cells and pixels.
        .on_size({
            let gs = &grid_size;
            move |_term| {
                let (columns, rows) = gs.get();
                Some(SizeReportSize {
                    rows,
                    columns,
                    cell_width: dims.cell_width as u32,
                    cell_height: dims.cell_height as u32,
                })
            }
        })?
        // device_attributes effect — responds to DA1/DA2/DA3 queries so
        // terminal applications can identify the terminal's capabilities.
        // We report VT220-level conformance with a modest feature set.
        .on_device_attributes(|_term| {
            Some(DeviceAttributes {
                // DA1: VT220-level with a few common features.
                primary: PrimaryDeviceAttributes::new(
                    ConformanceLevel::VT220,
                    [
                        DeviceAttributeFeature::COLUMNS_132,
                        DeviceAttributeFeature::SELECTIVE_ERASE,
                        DeviceAttributeFeature::ANSI_COLOR,
                    ],
                ),
                // DA2: VT220-type, version 1, no ROM cartridge.
                secondary: SecondaryDeviceAttributes {
                    device_type: DeviceType::VT220,
                    firmware_version: 1,
                    rom_cartridge: 0,
                },
                // DA3: default unit id (0).
                tertiary: Default::default(),
            })
        })?
        // xtversion effect — responds to CSI > q with our application name.
        .on_xtversion(|_term| Some("ghostling-rs"))?
        // color_scheme effect — responds to CSI ? 996 n.
        // We don't have any API to query the OS color scheme, so we return
        // false to silently ignore the query rather than guessing.
        .on_color_scheme(|_term| None)?;

    // Create the objects used in rendering the terminal, including the
    // render state and various reusable iterators.
    let mut renderer = Renderer::new()?;

    // Create various objects used for input handling and encoding.
    // These include a key encoder, a reusable key event object, a mouse
    // encoder, a reusable mouse event object, and a byte buffer storing
    // encoded events to be written back into the PTY.
    //
    // The encoders translate input events into the correct VT escape
    // sequences, respecting terminal modes like application cursor keys,
    // the Kitty keyboard protocol, SGR mouse reporting and tracking modes,
    // etc.
    let mut input = Input::new()?;

    println!(
        "ghostling-rs | simd: {}, optimize: {:?}, link: {:?}",
        if build_info::supports_simd()? {
            "enabled"
        } else {
            "disabled"
        },
        build_info::optimize_mode()?,
        build_info::link_mode(),
    );
    println!("Initialized terminal with size {cols}x{rows}");

    // Each frame: handle resize → read pty → process input → render.
    loop {
        // Recalculate grid dimensions when the window is resized.
        // We update both the ghostty terminal (so it reflows text) and the
        // pty's winsize (so the child shell knows about the new size and
        // can send SIGWINCH to its foreground process group).
        if dims.update() {
            let (cols, rows) = dims.grid_size();
            grid_size.set((cols, rows));
            terminal.resize(cols, rows, dims.cell_width as u32, dims.cell_height as u32)?;
            pty.resize(dims);
        }

        // Do different things based on whether the child process
        // (the user shell) is still active or not. We only want to keep
        // processing inputs and communicate with the child when it's
        // alive, and when it's exited we should handle cleanup properly.
        match child {
            Child::Active(pid) => {
                // Forward keyboard/mouse input only while the child is alive.
                input.handle_keyboard_input(&terminal)?;
                input.handle_mouse_input(&mut terminal, dims)?;

                // Write the composed response back to the PTY,
                // and clear it in preparation of the next frame.
                pty.write(&input.response);
                input.response.clear();

                match pty.read(&mut terminal) {
                    Ok(_) => {}
                    Err(PtyError::EndOfStream) => {
                        // EOF — the child's side of the pty is closed.
                        child = Child::Exited(pid);
                    }
                    Err(PtyError::OtherError(e)) => {
                        // Other error — the child's side of the pty is closed.
                        eprintln!("failed to read from pty: {e}");
                        child = Child::Exited(pid);
                    }
                }
            }
            // Try to reap the child if it had already exited.
            //
            // EOF can arrive before the child is waitable, so a single
            // WNOHANG attempt right at EOF may miss.  We also check for
            // signal death so the banner can report it properly.
            Child::Exited(pid) => {
                if let Ok(wp) = wait::waitpid(pid, Some(wait::WaitPidFlag::WNOHANG)) {
                    child = Child::Reaped(wp);
                }
                // FIXME: For some reason the child isn't being reaped properly.
                // Let's just unconditionally quit for now.
                order_quit();
            }
            Child::Reaped(status) => {
                println!("Child process exited: {status:?}");
                order_quit();
            }
        }

        // Draw the current terminal screen.
        renderer.render_terminal(&terminal, dims, Some(&font))?;
        next_frame().await
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

struct Renderer<'alloc> {
    render_state: RenderState<'alloc>,
    row_it: RowIterator<'alloc>,
    cell_it: CellIterator<'alloc>,
    placement_it: PlacementIterator<'alloc>,
}

impl<'alloc> Renderer<'alloc> {
    fn new() -> Result<Self> {
        Ok(Self {
            render_state: RenderState::new()?,
            row_it: RowIterator::new()?,
            cell_it: CellIterator::new()?,
            placement_it: PlacementIterator::new()?,
        })
    }

    /// Render the current terminal screen using the RenderState API.
    ///
    /// For each row/cell we read the grapheme codepoints and the cell's style,
    /// resolve foreground/background colors via the palette, and draw each
    /// character individually with `draw_text_ex`.  This supports per-cell colors
    /// from SGR sequences (bold, 256-color, 24-bit RGB, etc.).
    ///
    /// cell_width and cell_height are the measured dimensions of a single
    /// monospace glyph at the current font size, in screen (logical) pixels.
    /// font_size is the logical font size (before DPI scaling).
    fn render_terminal(
        &mut self,
        terminal: &Terminal<'alloc, '_>,
        dims: Dimensions,
        font: Option<&Font>,
    ) -> Result<()> {
        // Snapshot the terminal state into our render state. This is the
        // only point where we need access to the terminal; after this the
        // snapshot owns everything we need to draw the frame.
        //
        // When a snapshot is active, the render state cannot be updated —
        // this is upheld in the Rust API by making the snapshot take an
        // exclusive (mutable) reference to the render state.
        let snapshot = self.render_state.update(terminal)?;

        // Get the terminal's background color from the render state snapshot.
        let colors = snapshot.colors()?;
        clear_background(color(colors.background));

        // Obtain the Kitty graphics storage from the terminal. This is a
        // reference valid until the next mutating terminal call.
        let graphics = terminal.kitty_graphics();

        // Populate the row iterator from the current render state snapshot,
        // resulting in a row *iteration* object, which can be thought of as
        // a cursor into a row of the snapshot. Attributes of the current row
        // can be read from it via a shared reference, or it can be moved
        // to point at the next row via a mutable reference.
        let mut row_it = self.row_it.update(&snapshot)?;

        // --- Layer 1: images below cell backgrounds (z < INT32_MIN/2) ---
        if let Ok(graphics) = graphics.as_ref() {
            Self::render_kitty_images(
                terminal,
                &mut self.placement_it,
                graphics,
                Layer::BelowBg,
                dims,
            )?
        }

        // Small padding from the window edges.
        let mut y = PADDING;

        // For convenience, `next` gives you the same iteration back only
        // as a shared pointer, so you can simultaneously iterate through
        // all rows while having a handle to query data for each row.
        while let Some(row) = row_it.next() {
            let mut x = PADDING;

            // Cell iterators work similarly as they create a cell *iteration*
            // from a row iteration, which can then be used in a similar pattern.
            let mut cell_it = self.cell_it.update(row)?;

            while let Some(cell) = cell_it.next() {
                let graphemes = cell.graphemes_len()?;
                let bg = cell.bg_color()?;

                if graphemes == 0 {
                    // The cell has no text, but it might have a background
                    // color (e.g. from an erase with a color set).
                    if let Some(bg) = bg {
                        draw_rectangle(x, y, dims.cell_width, dims.cell_height, color(bg));
                    }
                } else {
                    // Convert read grapheme codepoints into UTF-8 text.
                    let text: String = cell.graphemes()?.into_iter().collect();

                    // Resolve foreground and background colors using the new
                    // per-cell color queries. These flatten style colors,
                    // content-tag colors, and palette lookups into a single RGB
                    // value, returning `None` when the cell has no explicit color
                    // (in which case we use the terminal default).
                    let mut fg = cell.fg_color()?.unwrap_or(colors.foreground);
                    let mut has_bg = bg.is_some();
                    let mut bg = bg.unwrap_or(colors.background);

                    // Read the style for flags (inverse, bold, italic) — color
                    // resolution is handled above via the new API.
                    let style = cell.style()?;

                    // Inverse (reverse video): swap foreground and background colors.
                    if style.inverse {
                        std::mem::swap(&mut fg, &mut bg);
                        has_bg = true;
                    }

                    // Draw a background rectangle if the cell has a non-default bg
                    // or if inverse mode forced a swap.
                    if has_bg {
                        draw_rectangle(x, y, dims.cell_width, dims.cell_height, color(bg));
                    }

                    // Draw the text for the cell.
                    draw_text_ex(
                        &text,
                        x,
                        y + FONT_SIZE as f32 + ROW_GAP,
                        TextParams {
                            font,
                            font_size: (FONT_SIZE as f32 * screen_dpi_scale()) as u16,
                            color: color(fg),
                            ..Default::default()
                        },
                    );

                    // Bold: draw the text a second time shifted 1 pixel to the
                    // right to thicken the strokes ("fake bold").
                    //
                    // In a more sophisticated terminal one would obviously use the
                    // correct bold version of the font using font discovery, but
                    // let's do it the hackier way here.
                    if style.bold {
                        draw_text_ex(
                            &text,
                            x + 1.0,
                            y + FONT_SIZE as f32 + ROW_GAP,
                            TextParams {
                                font,
                                font_size: (FONT_SIZE as f32 * screen_dpi_scale()) as u16,
                                color: color(fg),
                                ..Default::default()
                            },
                        );
                    }
                }

                x += dims.cell_width;
                continue;
            }

            // Clear per-row dirty flag after rendering it.
            row.set_dirty(false)?;
            y += dims.cell_height;
        }

        // --- Layer 2: images below text (i32::MIN / 2 <= z < 0) ---
        // Drawn after cell backgrounds but before the cursor and any
        // above-text images.  In our single-pass renderer the cell text
        // has already been drawn, but this still achieves the correct
        // visual for the common case where images sit behind text.
        if let Ok(graphics) = graphics.as_ref() {
            Self::render_kitty_images(
                terminal,
                &mut self.placement_it,
                graphics,
                Layer::BelowText,
                dims,
            )?
        }

        // Draw the cursor if visible.
        if snapshot.cursor_visible()?
            && let Some(vp) = snapshot.cursor_viewport()?
        {
            // Draw the cursor using the foreground color (or explicit cursor
            // color if the terminal set one).
            let cursor_color = colors.cursor.unwrap_or(colors.foreground);

            draw_rectangle(
                PADDING + vp.x as f32 * dims.cell_width,
                PADDING + vp.y as f32 * dims.cell_height,
                dims.cell_width,
                dims.cell_height,
                color(cursor_color),
            );
        }

        // --- Layer 3: images above text (z >= 0) ---
        if let Ok(graphics) = graphics.as_ref() {
            Self::render_kitty_images(
                terminal,
                &mut self.placement_it,
                graphics,
                Layer::AboveText,
                dims,
            )?
        }

        // Reset global dirty state so the next update reports changes accurately.
        snapshot.set_dirty(Dirty::Clean)?;
        Ok(())
    }

    fn render_kitty_images<'t>(
        terminal: &'t Terminal<'_, '_>,
        placement_it: &mut PlacementIterator<'_>,
        graphics: &Graphics<'t>,
        layer: Layer,
        dims: Dimensions,
    ) -> Result<()> {
        let mut placements = placement_it.update(graphics)?;
        placements.set_layer(layer)?;

        while let Some(placement) = placements.next() {
            let image_id = placement.image_id()?;
            let Some(image) = graphics.image(image_id) else {
                continue;
            };

            let info = placement.placement_render_info(&image, terminal)?;

            // Skip images that aren't visible in our viewport,
            // or have zero width/height in either pixel or grid coordinates
            if !info.viewport_visible
                || info.pixel_width == 0
                || info.pixel_height == 0
                || info.grid_cols == 0
                || info.grid_rows == 0
            {
                continue;
            }

            // We only handle RGBA (the PNG decoder we registered converts
            // everything to RGBA)
            if image.format()? != graphics::ImageFormat::Rgba {
                continue;
            }

            let image_width = image.width()?;
            let image_height = image.height()?;
            let data = image.data()?;
            if data.len() < (image_width * image_height * 4) as usize {
                continue;
            }

            // Compute grid cell count for rendered size.
            let dest_size = vec2(
                info.grid_cols as f32 * dims.cell_width,
                info.grid_rows as f32 * dims.cell_height,
            );

            // Read the sub-cell pixel offsets
            let x_offset = placement.x_offset()?;
            let y_offset = placement.y_offset()?;

            let tex = Texture2D::from_rgba8(image_width as u16, image_height as u16, data);

            draw_texture_ex(
                &tex,
                PADDING + info.viewport_col as f32 * dims.cell_width + x_offset as f32,
                PADDING + info.viewport_row as f32 * dims.cell_height + y_offset as f32,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(dest_size),
                    source: Some(Rect {
                        x: info.source_x as f32,
                        y: info.source_y as f32,
                        w: info.source_width as f32,
                        h: info.source_height as f32,
                    }),
                    ..Default::default()
                },
            );
        }

        Ok(())
    }
}

/// Convert Ghostty colors to macroquad colors
fn color(rgb: RgbColor) -> Color {
    let RgbColor { r, g, b } = rgb;
    color_u8!(r, g, b, 255)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Dimensions {
    window_width: f32,
    window_height: f32,
    cell_width: f32,
    cell_height: f32,
}

impl Dimensions {
    fn new(font: Option<&Font>) -> Self {
        // Measure a representative glyph to derive the monospace cell size.
        let glyph_size = measure_text("M", font, FONT_SIZE, screen_dpi_scale());

        // Guard against zero cell dimensions — these would cause division
        // by zero when computing the terminal grid.
        // Add in the cell and row gaps here too to make calculations easier.
        Self {
            cell_width: glyph_size.width.max(1.0) + CELL_GAP,
            cell_height: glyph_size.height.max(1.0) + ROW_GAP,
            window_width: screen_width(),
            window_height: screen_height(),
        }
    }

    fn grid_size(self) -> (u16, u16) {
        let cols = ((self.window_width - 2.0 * PADDING) / self.cell_width).max(1.0) as u16;
        let rows = ((self.window_height - 2.0 * PADDING) / self.cell_height).max(1.0) as u16;
        (cols, rows)
    }
    fn update(&mut self) -> bool {
        if screen_width() == self.window_width && screen_height() == self.window_height {
            return false;
        }
        self.window_width = screen_width();
        self.window_height = screen_height();
        true
    }
    fn to_winsize(self) -> nix::pty::Winsize {
        let (cols, rows) = self.grid_size();
        nix::pty::Winsize {
            ws_col: cols,
            ws_row: rows,
            ws_xpixel: self.window_width as u16,
            ws_ypixel: self.window_height as u16,
        }
    }
}

// ---------------------------------------------------------------------------
// Input handling
// ---------------------------------------------------------------------------

struct Input<'alloc> {
    key_encoder: key::Encoder<'alloc>,
    key_event: key::Event<'alloc>,
    mouse_encoder: mouse::Encoder<'alloc>,
    mouse_event: mouse::Event<'alloc>,
    response: Vec<u8>,
}

impl<'alloc> Input<'alloc> {
    fn new() -> Result<Self> {
        Ok(Self {
            key_encoder: key::Encoder::new()?,
            key_event: key::Event::new()?,
            mouse_encoder: mouse::Encoder::new()?,
            mouse_event: mouse::Event::new()?,
            response: Vec::with_capacity(64),
        })
    }

    /// Poll macroquad for keyboard events and use the libghostty key encoder
    /// to produce the correct VT escape sequences, which are then written
    /// to the pty.  The encoder respects terminal modes (cursor key
    /// application mode, Kitty keyboard protocol, etc.) so we don't need
    /// to maintain our own escape-sequence tables.
    fn handle_keyboard_input(&mut self, terminal: &Terminal<'alloc, '_>) -> Result<()> {
        // Drain printable characters from macroquad's input queue.  We collect
        // them into a single UTF-8 buffer so the encoder can attach text
        // to the key event.
        let mut chars_pressed = Vec::<char>::with_capacity(4);
        while let Some(ch) = get_char_pressed() {
            chars_pressed.push(ch);
        }
        let mut text: String = chars_pressed.into_iter().collect();

        for (kc, key, ucp) in Self::ALL_KEYS {
            let action = if is_key_released(kc) {
                key::Action::Release
            } else if is_key_pressed(kc) {
                key::Action::Press
            } else {
                continue;
            };

            // Conditionally attach any UTF-8 text that macroquad produced for this frame.
            // For unmodified printable keys this is the character itself;
            // for special keys or ctrl combos there's typically no text.
            // Release events never carry text.
            let maybe_text = if !text.is_empty() && !is_key_released(kc) {
                Some(text.as_str())
            } else {
                None
            };

            let mods = Self::keyboard_mods();

            // Consumed mods are modifiers the platform's text input
            // already accounted for when producing the UTF-8 text.
            // For printable keys, shift is consumed (it turns 'a' → 'A').
            // For non-printable keys nothing is consumed.
            let mut consumed = key::Mods::empty();
            if ucp != '\0' && mods.contains(key::Mods::SHIFT) {
                consumed |= key::Mods::SHIFT;
            }

            self.key_event
                .set_action(action)
                .set_key(key)
                .set_mods(mods)
                .set_consumed_mods(consumed)
                // The unshifted codepoint is the character the key produces
                // with no modifiers.  The Kitty protocol needs it to identify
                // keys independent of the current shift state.
                .set_unshifted_codepoint(ucp)
                .set_utf8(maybe_text);

            if maybe_text.is_some() {
                text.clear();
            }

            self.key_encoder
                // Sync encoder options from the terminal so mode changes (e.g.
                // application cursor keys, Kitty keyboard protocol) are honoured.
                .set_options_from_terminal(terminal)
                .encode_to_vec(&self.key_event, &mut self.response)?;

            if !self.response.is_empty() {
                // Text was consumed by the encoder — clear it so the
                // fallback below doesn't double-send.
                text.clear();
            }
        }

        // Fallback: on some platforms (e.g. VMs) the character event arrives
        // a frame after the key-press event.  If we collected UTF-8 text but
        // no key event consumed it, write it directly to the PTY so input
        // isn't silently dropped.
        if !text.is_empty() {
            self.response.extend_from_slice(text.as_bytes());
        }
        Ok(())
    }

    /// Poll macroquad for mouse events and use the libghostty mouse encoder
    /// to produce the correct VT escape sequences, which are then written
    /// to the pty.  The encoder handles tracking mode (X10, normal, button,
    /// any-event) and output format (X10, UTF8, SGR, URxvt, SGR-Pixels)
    /// based on what the terminal application has requested.
    fn handle_mouse_input(
        &mut self,
        terminal: &mut Terminal<'alloc, '_>,
        dims: Dimensions,
    ) -> Result<()> {
        // Track whether any button is currently held — the encoder uses
        // this to distinguish drags from plain motion.
        let any_pressed = Self::ALL_MOUSE_BUTTONS
            .into_iter()
            .any(|(m, _)| is_mouse_button_down(m));

        let (x, y) = mouse_position();
        self.mouse_event
            .set_mods(Self::keyboard_mods())
            .set_position(mouse::Position { x, y });

        self.mouse_encoder
            // Sync encoder tracking mode and format from terminal state so
            // mode changes (e.g. applications enabling SGR mouse reporting)
            // are honoured automatically.
            .set_options_from_terminal(terminal)
            // Provide the encoder with the current terminal geometry so it
            // can convert pixel positions to cell coordinates.
            .set_size(mouse::EncoderSize {
                screen_width: dims.window_width as u32,
                screen_height: dims.window_height as u32,
                cell_width: dims.cell_width as u32,
                cell_height: dims.cell_height as u32,
                padding_top: PADDING as u32,
                padding_bottom: PADDING as u32,
                padding_left: PADDING as u32,
                padding_right: PADDING as u32,
            })
            .set_any_button_pressed(any_pressed)
            // Enable motion deduplication so the encoder suppresses redundant
            // motion events within the same cell.
            .set_track_last_cell(true);

        // Check each mouse button for press/release events.
        for (mb, btn) in Self::ALL_MOUSE_BUTTONS {
            let action = if is_mouse_button_released(mb) {
                mouse::Action::Release
            } else if is_mouse_button_pressed(mb) {
                mouse::Action::Press
            } else {
                continue;
            };

            self.mouse_event.set_action(action).set_button(Some(btn));
            self.mouse_encoder
                .encode_to_vec(&self.mouse_event, &mut self.response)?;
        }

        // Mouse motion — send a motion event with whatever button is held
        // (or no button for pure motion in any-event tracking mode).
        let delta = mouse_delta_position();
        if delta.x.abs() > 1e-6 || delta.y.abs() > 1e-6 {
            self.mouse_event
                .set_action(mouse::Action::Motion)
                .set_button(
                    Self::ALL_MOUSE_BUTTONS
                        .into_iter()
                        .find(|(mb, _)| is_mouse_button_down(*mb))
                        .map(|(_, btn)| btn),
                );
            self.mouse_encoder
                .encode_to_vec(&self.mouse_event, &mut self.response)?;
        }

        // Scroll wheel handling.  When a mouse tracking mode is active the
        // wheel events are forwarded to the application as button 4/5
        // press+release pairs.  Otherwise we scroll the viewport through
        // the scrollback buffer so the user can review history.
        let (wheel_x, wheel_y) = mouse_wheel();
        if wheel_x.abs() > 1e-6 || wheel_y.abs() > 1e-6 {
            // Check whether any mouse tracking mode is enabled.  If so,
            // the application wants to handle scroll events itself.
            let is_mouse_tracking = [
                Mode::X10_MOUSE,
                Mode::NORMAL_MOUSE,
                Mode::BUTTON_MOUSE,
                Mode::ANY_MOUSE,
            ]
            .into_iter()
            .any(|mode| matches!(terminal.mode(mode), Ok(true)));

            if is_mouse_tracking {
                let scroll_btn = if wheel_y > 0.0 {
                    mouse::Button::Four
                } else {
                    mouse::Button::Five
                };

                self.mouse_event
                    .set_button(Some(scroll_btn))
                    .set_action(mouse::Action::Press);
                self.mouse_encoder
                    .encode_to_vec(&self.mouse_event, &mut self.response)?;

                self.mouse_event.set_action(mouse::Action::Release);
                self.mouse_encoder
                    .encode_to_vec(&self.mouse_event, &mut self.response)?;
            } else {
                // Scroll the viewport through scrollback. Adapt
                // the scroll delta to the wheel/touchpad velocity
                // for a comfortable pace.  Delta is negative to scroll
                // up (into history), positive to scroll down.
                let scroll_delta: isize = (wheel_y * -2.5) as isize;
                terminal.scroll_viewport(ScrollViewport::Delta(scroll_delta));
            }
        }
        Ok(())
    }

    // Build a Mods bitmask from the current macroquad modifier key state.
    fn keyboard_mods() -> key::Mods {
        let mut mods = key::Mods::empty();
        if is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift) {
            mods |= key::Mods::SHIFT;
        }
        if is_key_down(KeyCode::LeftAlt) || is_key_down(KeyCode::RightAlt) {
            mods |= key::Mods::ALT;
        }
        if is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl) {
            mods |= key::Mods::CTRL;
        }
        if is_key_down(KeyCode::LeftSuper) || is_key_down(KeyCode::RightSuper) {
            mods |= key::Mods::SUPER;
        }
        mods
    }

    /// All macroquad mouse buttons we want to check with their libghostty equivalent.
    const ALL_MOUSE_BUTTONS: [(MouseButton, mouse::Button); 3] = [
        (MouseButton::Left, mouse::Button::Left),
        (MouseButton::Right, mouse::Button::Right),
        (MouseButton::Middle, mouse::Button::Middle),
    ];

    /// All macroquad keys we want to check for press/repeat/release events,
    /// with their libghostty equivalent and their unshifted Unicode codepoint,
    /// i.e. character the key produces with no modifiers on a US layout. The
    /// Kitty keyboard protocol requires this to identify keys. Returns NUL
    /// for keys that don't have a natural codepoint (arrows, F-keys, etc.).
    const ALL_KEYS: [(KeyCode, Key, char); 74] = [
        (KeyCode::A, Key::A, 'a'),
        (KeyCode::B, Key::B, 'b'),
        (KeyCode::C, Key::C, 'c'),
        (KeyCode::D, Key::D, 'd'),
        (KeyCode::E, Key::E, 'e'),
        (KeyCode::F, Key::F, 'f'),
        (KeyCode::G, Key::G, 'g'),
        (KeyCode::H, Key::H, 'h'),
        (KeyCode::I, Key::I, 'i'),
        (KeyCode::J, Key::J, 'j'),
        (KeyCode::K, Key::K, 'k'),
        (KeyCode::L, Key::L, 'l'),
        (KeyCode::M, Key::M, 'm'),
        (KeyCode::N, Key::N, 'n'),
        (KeyCode::O, Key::O, 'o'),
        (KeyCode::P, Key::P, 'p'),
        (KeyCode::Q, Key::Q, 'q'),
        (KeyCode::R, Key::R, 'r'),
        (KeyCode::S, Key::S, 's'),
        (KeyCode::T, Key::T, 't'),
        (KeyCode::U, Key::U, 'u'),
        (KeyCode::V, Key::V, 'v'),
        (KeyCode::W, Key::W, 'w'),
        (KeyCode::X, Key::X, 'x'),
        (KeyCode::Y, Key::Y, 'y'),
        (KeyCode::Z, Key::Z, 'z'),
        (KeyCode::Key0, Key::Digit0, '0'),
        (KeyCode::Key1, Key::Digit1, '1'),
        (KeyCode::Key2, Key::Digit2, '2'),
        (KeyCode::Key3, Key::Digit3, '3'),
        (KeyCode::Key4, Key::Digit4, '4'),
        (KeyCode::Key5, Key::Digit5, '5'),
        (KeyCode::Key6, Key::Digit6, '6'),
        (KeyCode::Key7, Key::Digit7, '7'),
        (KeyCode::Key8, Key::Digit8, '8'),
        (KeyCode::Key9, Key::Digit9, '9'),
        (KeyCode::Space, Key::Space, ' '),
        (KeyCode::Enter, Key::Enter, '\0'),
        (KeyCode::Tab, Key::Tab, '\0'),
        (KeyCode::Backspace, Key::Backspace, '\0'),
        (KeyCode::Delete, Key::Delete, '\0'),
        (KeyCode::Escape, Key::Escape, '\0'),
        (KeyCode::Up, Key::ArrowUp, '\0'),
        (KeyCode::Down, Key::ArrowDown, '\0'),
        (KeyCode::Left, Key::ArrowLeft, '\0'),
        (KeyCode::Right, Key::ArrowRight, '\0'),
        (KeyCode::Home, Key::Home, '\0'),
        (KeyCode::End, Key::End, '\0'),
        (KeyCode::PageUp, Key::PageUp, '\0'),
        (KeyCode::PageDown, Key::PageDown, '\0'),
        (KeyCode::Insert, Key::Insert, '\0'),
        (KeyCode::Minus, Key::Minus, '-'),
        (KeyCode::Equal, Key::Equal, '='),
        (KeyCode::LeftBracket, Key::BracketLeft, '['),
        (KeyCode::RightBracket, Key::BracketRight, ']'),
        (KeyCode::Backslash, Key::Backslash, '\\'),
        (KeyCode::Semicolon, Key::Semicolon, ';'),
        (KeyCode::Apostrophe, Key::Quote, '\''),
        (KeyCode::Comma, Key::Comma, ','),
        (KeyCode::Period, Key::Period, '.'),
        (KeyCode::Slash, Key::Slash, '/'),
        (KeyCode::GraveAccent, Key::Backquote, '`'),
        (KeyCode::F1, Key::F1, '\0'),
        (KeyCode::F2, Key::F2, '\0'),
        (KeyCode::F3, Key::F3, '\0'),
        (KeyCode::F4, Key::F4, '\0'),
        (KeyCode::F5, Key::F5, '\0'),
        (KeyCode::F6, Key::F6, '\0'),
        (KeyCode::F7, Key::F7, '\0'),
        (KeyCode::F8, Key::F8, '\0'),
        (KeyCode::F9, Key::F9, '\0'),
        (KeyCode::F10, Key::F10, '\0'),
        (KeyCode::F11, Key::F11, '\0'),
        (KeyCode::F12, Key::F12, '\0'),
    ];
}

/// Configuration for macroquad.
///
/// By default the window is resizable and DPI-aware,
/// while prioritizing Wayland over X11 on Linux.
fn macroquad_conf() -> Conf {
    Conf {
        window_title: "ghostling-rs".to_owned(),
        window_resizable: true,
        high_dpi: true,
        platform: conf::Platform {
            // Default to Wayland and fallback to X11 on Linux.
            linux_backend: conf::LinuxBackend::WaylandWithX11Fallback,
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// System callbacks (process-global, set once at startup)
// ---------------------------------------------------------------------------

/// A decoder that decodes raw PNG data into 8-bit RGBA pixels using
/// `macroquad`'s PNG decoder. The output buffer is allocated through the
/// provided `Allocator` so the library can free it later.
struct PngDecoder;

impl DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc libghostty_vt::alloc::Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        // `macroquad` decodes the PNG image for us and converts it
        // to the correct RGBA8 pixel format automatically.
        let img = Image::from_file_with_format(data, Some(ImageFormat::Png)).ok()?;
        let mut data = Bytes::new_with_alloc(alloc, img.bytes.len()).ok()?;
        data.copy_from_slice(&img.bytes);

        Some(DecodedImage {
            width: img.width as u32,
            height: img.height as u32,
            data,
        })
    }
}

/// This is a full implementation of a pseudo-terminal (PTY) interface.
///
/// Normally we would try to avoid using unsafe code in examples,
/// but pseudo-terminals are very hard to use safely in the general case
/// and as such the standard library does not handle them at all.
///
/// Of course, you could just use any off-the-shelf PTY crate on crates.io,
/// but we'd like to keep this demo self-contained using minimal dependencies,
/// so here you go.
mod pty {
    // Unfortunately, there will be some unsafe shenanigans here.
    #![allow(unsafe_code)]
    use std::{
        os::{
            fd::{AsRawFd, OwnedFd},
            unix::process::CommandExt,
        },
        path::PathBuf,
        process::Command,
    };

    use libghostty_vt::Terminal;
    use nix::{
        errno::Errno,
        fcntl::{self, OFlag},
        pty::ForkptyResult,
        sys::{signal, wait},
        unistd::{self, Pid},
    };

    use crate::Dimensions;

    /// Handle to a pseudo-terminal (PTY).
    pub struct Pty(OwnedFd);

    impl Pty {
        /// Spawn the user's default shell in a new pseudo-terminal.
        ///
        /// Creates a pty pair via forkpty(), sets the initial window size, execs the
        /// shell in the child, and puts the master fd into non-blocking mode so we
        /// can poll it each frame without stalling the render loop.
        ///
        /// The shell is chosen by checking, in order:
        ///   1. $SHELL environment variable
        ///   2. The pw_shell field from the passwd database
        ///   3. /bin/sh as a last resort
        pub fn new(dims: Dimensions) -> std::io::Result<(Self, Child)> {
            // forkpty() combines openpty + fork + login_tty into one call.
            // In the child it sets up the slave side as stdin/stdout/stderr.
            match unsafe { nix::pty::forkpty(&dims.to_winsize(), None)? } {
                // Child process -- replace ourselves with the shell.
                // TERM tells programs what escape sequences we understand.
                ForkptyResult::Child => {
                    // Determine the user's preferred shell. We try $SHELL first (the
                    // standard convention), then fall back to the passwd entry, and
                    // finally to /bin/sh if nothing else is available.
                    let shell = match std::env::var_os("SHELL") {
                        Some(shell) if !shell.is_empty() => PathBuf::from(shell),
                        _ => match unistd::User::from_uid(unistd::getuid()) {
                            Ok(Some(user)) => user.shell,
                            _ => PathBuf::from("/bin/sh"),
                        },
                    };

                    // Extract just the program name for argv[0] (e.g. "/bin/zsh" -> "zsh").
                    let arg0 = shell.file_name().unwrap_or(shell.as_os_str());

                    // Replace the child process with the user's shell via `exec`.
                    _ = Command::new(&shell)
                        .env("TERM", "xterm-256color")
                        .arg0(arg0)
                        .exec();

                    // `exec` only returns on error.
                    std::process::exit(127);
                }

                // Parent -- make the master fd non-blocking so read() returns EAGAIN
                // instead of blocking when there's no data, letting us poll each frame.
                ForkptyResult::Parent { child, master: fd } => {
                    let raw_flags = fcntl::fcntl(&fd, fcntl::F_GETFL)?;
                    let flags = OFlag::from_bits_retain(raw_flags) | OFlag::O_NONBLOCK;
                    _ = fcntl::fcntl(&fd, fcntl::F_SETFL(flags))?;

                    Ok((Self(fd), Child::Active(child)))
                }
            }
        }

        /// Drain all available output from the pty master and feed it into the
        /// ghostty terminal. The terminal's VT parser will process any escape
        /// sequences and update its internal screen/cursor/style state.
        ///
        /// Because the fd is non-blocking, read() returns an error with
        /// EAGAIN once the kernel buffer is empty, at which point we stop.
        pub fn read(&self, term: &mut Terminal) -> Result<(), PtyError> {
            let mut buf = [0u8; 4096];
            loop {
                match nix::unistd::read(&self.0, &mut buf) {
                    // EOF -- the child closed its side of the pty.
                    Ok(0) => return Err(PtyError::EndOfStream),
                    Ok(len) => term.vt_write(&buf[..len]),

                    // Distinguish "no data right now" from real errors.
                    Err(Errno::EAGAIN) => return Ok(()),
                    Err(Errno::EINTR) => continue, // retry the read
                    // On Linux, the slave closing often produces EIO rather
                    // than a clean EOF (read returning 0). Treat it the same.
                    Err(Errno::EIO) => return Err(PtyError::EndOfStream),
                    Err(err) => return Err(PtyError::OtherError(err)),
                };
            }
        }

        /// Best-effort write to the pty master fd. Because the fd is
        /// non-blocking, write() may return short or fail with EAGAIN.
        /// We retry on EINTR, advance past partial writes, and silently
        /// drop data if the kernel buffer is full -- this matches what most
        /// terminal emulators do under back-pressure.
        pub fn write(&self, data: &[u8]) {
            let mut remaining = data;
            while !remaining.is_empty() {
                match nix::unistd::write(&self.0, remaining) {
                    Ok(len) => remaining = &remaining[len..],
                    Err(Errno::EINTR) => continue,
                    // EAGAIN or real error -- drop the remainder.
                    Err(_) => break,
                }
            }
        }

        /// Send a resize event to the pty via the TIOCSWINSZ ioctl.
        ///
        /// ioctl is a way to control a resource without inventing a dedicated
        /// system call for it, and in this case we have to generate bindings
        /// for this ioctl and feed the new window size to it.
        pub fn resize(&self, dims: Dimensions) {
            nix::ioctl_write_ptr_bad!(tiocswinsz, nix::libc::TIOCSWINSZ, nix::pty::Winsize);
            _ = unsafe { tiocswinsz(self.0.as_raw_fd(), &dims.to_winsize()) };
        }
    }

    /// The child process within a pseudo-terminal.
    ///
    /// The child can be in one of three states: active (default), exited
    /// and reaped. The parent process has to wait for the child to fully
    /// exit, which is why it tries to "reap" the child process by waiting
    /// for it to exit via `waitpid`. Once it has exited fully, it will be
    /// considered "reaped".
    pub enum Child {
        Active(Pid),
        Exited(Pid),
        Reaped(wait::WaitStatus),
    }

    impl Drop for Child {
        fn drop(&mut self) {
            // If the child is still running, ask it to exit. Then do a
            // blocking waitpid to reap it and avoid leaving a zombie.
            match *self {
                Child::Active(pid) => {
                    let _ = signal::kill(pid, signal::SIGHUP);
                    let _ = wait::waitpid(pid, None);
                }
                Child::Exited(pid) => {
                    let _ = wait::waitpid(pid, None);
                }
                Child::Reaped(_) => {}
            }
        }
    }

    #[derive(Clone, Debug)]
    pub enum PtyError {
        EndOfStream,
        OtherError(nix::errno::Errno),
    }
}
