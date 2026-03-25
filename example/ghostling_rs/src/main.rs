//! Ghostling (Rust) — minimal terminal emulator built on libghostty-rs.
//!
//! This is a Rust port of the C ghostling example from ghostty-org/ghostling.
//! It uses Raylib for windowing/rendering and libghostty-vt (via the safe
//! `ghostty` crate) for terminal emulation. The architecture is intentionally
//! simple: single-threaded, 2D software rendering, one file.

use std::cell::{Cell, RefCell};
use std::io;
use std::os::fd::AsFd;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use ghostty::focus;
use ghostty::key::Key;
use ghostty::render::{Dirty, Snapshot};
use ghostty::style::RgbColor;
use ghostty::terminal::{self, Mode};
use ghostty::{
    Terminal, TerminalOptions, ffi, key, mouse,
    render::{CellIterator, RenderState, RowIterator},
};
use nix::errno::Errno;
use nix::fcntl::{self, OFlag};
use nix::pty::ForkptyResult;
use nix::sys::{signal, wait};
use nix::unistd::{self, Pid};
use raylib::prelude::*;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// ---------------------------------------------------------------------------
// PTY helpers
// ---------------------------------------------------------------------------

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
unsafe fn pty_spawn(cols: u16, rows: u16) -> io::Result<(OwnedFd, Pid)> {
    let ws = nix::pty::Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // forkpty() combines openpty + fork + login_tty into one call.
    // In the child it sets up the slave side as stdin/stdout/stderr.
    match unsafe { nix::pty::forkpty(&ws, None)? } {
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

            _ = Command::new(&shell)
                .env("TERM", "xterm-256color")
                .arg0(arg0)
                .exec();

            // `exec` only returns on error.
            std::process::exit(127);
        }

        // Parent -- make the master fd non-blocking so read() returns EAGAIN
        // instead of blocking when there's no data, letting us poll each frame.
        ForkptyResult::Parent { child, master } => {
            let raw_flags = fcntl::fcntl(&master, fcntl::F_GETFL)?;
            let flags = OFlag::from_bits_retain(raw_flags).union(OFlag::O_NONBLOCK);
            _ = fcntl::fcntl(&master, fcntl::F_SETFL(flags))?;
            Ok((master, child))
        }
    }
}

/// Result of draining the pty master fd.
#[derive(Debug, PartialEq)]
enum PtyReadResult {
    /// Data was drained (or EAGAIN, i.e. nothing available right now).
    Ok,
    /// The child closed its end of the pty.
    Eof,
    /// A real read error occurred.
    Error,
}

/// Drain all available output from the pty master and feed it into the
/// ghostty terminal. The terminal's VT parser will process any escape
/// sequences and update its internal screen/cursor/style state.
///
/// Because the fd is non-blocking, read() returns an error with
/// EAGAIN once the kernel buffer is empty, at which point we stop.
fn pty_read<Fd: AsFd>(fd: Fd, terminal: &mut Terminal) -> PtyReadResult {
    let mut buf = [0u8; 4096];

    loop {
        match nix::unistd::read(&fd, &mut buf) {
            // EOF -- the child closed its side of the pty.
            Ok(0) => return PtyReadResult::Eof,
            Ok(len) => terminal.vt_write(&buf[..len]),

            // Distinguish "no data right now" from real errors.
            Err(Errno::EAGAIN) => return PtyReadResult::Ok,
            Err(Errno::EINTR) => continue, // retry the read
            // On Linux, the slave closing often produces EIO rather
            // than a clean EOF (read returning 0). Treat it the same.
            Err(Errno::EIO) => return PtyReadResult::Eof,
            Err(err) => {
                eprintln!("pty read: {err}");
                return PtyReadResult::Error;
            }
        }
    }
}

/// Best-effort write to the pty master fd. Because the fd is non-blocking,
/// write() may return short or fail with EAGAIN. We retry on EINTR, advance
/// past partial writes, and silently drop data if the kernel buffer is full
/// -- this matches what most terminal emulators do under back-pressure.
fn pty_write<Fd: AsFd>(fd: Fd, data: &[u8]) {
    let mut remaining = data;

    while !remaining.is_empty() {
        match nix::unistd::write(&fd, remaining) {
            Ok(len) => remaining = &remaining[len..],
            Err(Errno::EINTR) => continue,
            // EAGAIN or real error -- drop the remainder.
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Input handling
// ---------------------------------------------------------------------------

/// Map a raylib key constant to a GhosttyKey code.
/// Returns GHOSTTY_KEY_UNIDENTIFIED for keys we don't handle.
fn raylib_key_to_ghostty(rl_key: KeyboardKey) -> Key {
    use KeyboardKey::*;

    // Letters -- raylib KEY_A..KEY_Z are contiguous, and so are
    // GHOSTTY_KEY_A..GHOSTTY_KEY_Z.
    let key_a = KEY_A as u32;
    let key_z = KEY_Z as u32;
    let key_zero = KEY_ZERO as u32;
    let key_nine = KEY_NINE as u32;
    let key_f1 = KEY_F1 as u32;
    let key_f12 = KEY_F12 as u32;
    let k = rl_key as u32;

    if (key_a..=key_z).contains(&k) {
        return Key::try_from(Key::A as u32 + (k - key_a)).unwrap();
    }
    // Digits -- raylib KEY_ZERO..KEY_NINE are contiguous.
    if (key_zero..=key_nine).contains(&k) {
        return Key::try_from(Key::Digit0 as u32 + (k - key_zero)).unwrap();
    }
    // Function keys -- raylib KEY_F1..KEY_F12 are contiguous.
    if (key_f1..=key_f12).contains(&k) {
        return Key::try_from(Key::F1 as u32 + (k - key_f1)).unwrap();
    }

    match rl_key {
        KEY_SPACE => Key::Space,
        KEY_ENTER => Key::Enter,
        KEY_TAB => Key::Tab,
        KEY_BACKSPACE => Key::Backspace,
        KEY_DELETE => Key::Delete,
        KEY_ESCAPE => Key::Escape,
        KEY_UP => Key::ArrowUp,
        KEY_DOWN => Key::ArrowDown,
        KEY_LEFT => Key::ArrowLeft,
        KEY_RIGHT => Key::ArrowRight,
        KEY_HOME => Key::Home,
        KEY_END => Key::End,
        KEY_PAGE_UP => Key::PageUp,
        KEY_PAGE_DOWN => Key::PageDown,
        KEY_INSERT => Key::Insert,
        KEY_MINUS => Key::Minus,
        KEY_EQUAL => Key::Equal,
        KEY_LEFT_BRACKET => Key::BracketLeft,
        KEY_RIGHT_BRACKET => Key::BracketRight,
        KEY_BACKSLASH => Key::Backslash,
        KEY_SEMICOLON => Key::Semicolon,
        KEY_APOSTROPHE => Key::Quote,
        KEY_COMMA => Key::Comma,
        KEY_PERIOD => Key::Period,
        KEY_SLASH => Key::Slash,
        KEY_GRAVE => Key::Backquote,
        _ => Key::Unidentified,
    }
}

/// Return the unshifted Unicode codepoint for a raylib key, i.e. the
/// character the key produces with no modifiers on a US layout. The
/// Kitty keyboard protocol requires this to identify keys. Returns 0
/// for keys that don't have a natural codepoint (arrows, F-keys, etc.).
fn raylib_key_unshifted_codepoint(rl_key: KeyboardKey) -> char {
    use KeyboardKey::*;

    let key_a = KEY_A as u32;
    let key_z = KEY_Z as u32;
    let key_zero = KEY_ZERO as u32;
    let key_nine = KEY_NINE as u32;
    let k = rl_key as u32;

    if (key_a..=key_z).contains(&k) {
        return char::from_u32('a' as u32 + (k - key_a)).unwrap();
    }
    if (key_zero..=key_nine).contains(&k) {
        return char::from_u32('0' as u32 + (k - key_zero)).unwrap();
    }

    match rl_key {
        KEY_SPACE => ' ',
        KEY_MINUS => '-',
        KEY_EQUAL => '=',
        KEY_LEFT_BRACKET => '[',
        KEY_RIGHT_BRACKET => ']',
        KEY_BACKSLASH => '\\',
        KEY_SEMICOLON => ';',
        KEY_APOSTROPHE => '\'',
        KEY_COMMA => ',',
        KEY_PERIOD => '.',
        KEY_SLASH => '/',
        KEY_GRAVE => '`',
        _ => '\0',
    }
}

/// Build a GhosttyMods bitmask from the current raylib modifier key state.
fn get_ghostty_mods(rl: &RaylibHandle) -> key::Mods {
    let mut mods = key::Mods::empty();
    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT) {
        mods |= key::Mods::SHIFT;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL)
    {
        mods |= key::Mods::CTRL;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_ALT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT) {
        mods |= key::Mods::ALT;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER) || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER) {
        mods |= key::Mods::SUPER;
    }
    mods
}

/// Map a raylib mouse button to a GhosttyMouseButton.
fn raylib_mouse_to_ghostty(rl_button: MouseButton) -> mouse::Button {
    match rl_button {
        MouseButton::MOUSE_BUTTON_LEFT => mouse::Button::Left,
        MouseButton::MOUSE_BUTTON_RIGHT => mouse::Button::Right,
        MouseButton::MOUSE_BUTTON_MIDDLE => mouse::Button::Middle,
        MouseButton::MOUSE_BUTTON_SIDE => mouse::Button::Four,
        MouseButton::MOUSE_BUTTON_EXTRA => mouse::Button::Five,
        MouseButton::MOUSE_BUTTON_FORWARD => mouse::Button::Six,
        MouseButton::MOUSE_BUTTON_BACK => mouse::Button::Seven,
    }
}

/// All raylib keys we want to check for press/repeat/release events.
/// Letters and digits are handled via ranges; everything else is
/// enumerated explicitly.
fn all_keys_to_check() -> Vec<KeyboardKey> {
    let mut keys = Vec::new();
    for k in (KeyboardKey::KEY_A as u32)..=(KeyboardKey::KEY_Z as u32) {
        keys.push(num_to_keyboard_key(k));
    }
    for k in (KeyboardKey::KEY_ZERO as u32)..=(KeyboardKey::KEY_NINE as u32) {
        keys.push(num_to_keyboard_key(k));
    }
    keys.extend_from_slice(&[
        KeyboardKey::KEY_SPACE,
        KeyboardKey::KEY_ENTER,
        KeyboardKey::KEY_TAB,
        KeyboardKey::KEY_BACKSPACE,
        KeyboardKey::KEY_DELETE,
        KeyboardKey::KEY_ESCAPE,
        KeyboardKey::KEY_UP,
        KeyboardKey::KEY_DOWN,
        KeyboardKey::KEY_LEFT,
        KeyboardKey::KEY_RIGHT,
        KeyboardKey::KEY_HOME,
        KeyboardKey::KEY_END,
        KeyboardKey::KEY_PAGE_UP,
        KeyboardKey::KEY_PAGE_DOWN,
        KeyboardKey::KEY_INSERT,
        KeyboardKey::KEY_MINUS,
        KeyboardKey::KEY_EQUAL,
        KeyboardKey::KEY_LEFT_BRACKET,
        KeyboardKey::KEY_RIGHT_BRACKET,
        KeyboardKey::KEY_BACKSLASH,
        KeyboardKey::KEY_SEMICOLON,
        KeyboardKey::KEY_APOSTROPHE,
        KeyboardKey::KEY_COMMA,
        KeyboardKey::KEY_PERIOD,
        KeyboardKey::KEY_SLASH,
        KeyboardKey::KEY_GRAVE,
        KeyboardKey::KEY_F1,
        KeyboardKey::KEY_F2,
        KeyboardKey::KEY_F3,
        KeyboardKey::KEY_F4,
        KeyboardKey::KEY_F5,
        KeyboardKey::KEY_F6,
        KeyboardKey::KEY_F7,
        KeyboardKey::KEY_F8,
        KeyboardKey::KEY_F9,
        KeyboardKey::KEY_F10,
        KeyboardKey::KEY_F11,
        KeyboardKey::KEY_F12,
    ]);
    keys
}

/// Convert a u32 back to a KeyboardKey. Valid for the contiguous ranges
/// we use (A-Z, 0-9, specials).
fn num_to_keyboard_key(n: u32) -> KeyboardKey {
    unsafe { std::mem::transmute::<u32, KeyboardKey>(n) }
}

/// Poll raylib for keyboard events and use the libghostty key encoder
/// to produce the correct VT escape sequences, which are then written
/// to the pty. The encoder respects terminal modes (cursor key
/// application mode, Kitty keyboard protocol, etc.) so we don't need
/// to maintain our own escape-sequence tables.
fn handle_input<Fd: AsFd>(
    rl: &mut RaylibHandle,
    pty_fd: Fd,
    encoder: &mut key::Encoder,
    event: &mut key::Event,
    terminal: &Terminal,
) {
    // Sync encoder options from the terminal so mode changes (e.g.
    // application cursor keys, Kitty keyboard protocol) are honoured.
    encoder.with_options_from_terminal(terminal);

    // Drain printable characters from raylib's input queue. We collect
    // them into a single UTF-8 buffer so the encoder can attach text
    // to the key event.
    let mut char_utf8 = [0u8; 64];
    let mut char_utf8_len: usize = 0;
    while let Some(ch) = rl.get_char_pressed() {
        if char_utf8_len + ch.len_utf8() < char_utf8.len() {
            _ = ch.encode_utf8(&mut char_utf8[char_utf8_len..]);
        }
    }
    let text = std::str::from_utf8(&char_utf8[..char_utf8_len]).expect("a valid UTF-8 string");

    let keys = all_keys_to_check();
    let mods = get_ghostty_mods(rl);

    for rl_key in keys {
        let pressed = rl.is_key_pressed(rl_key);
        let repeated = rl.is_key_pressed_repeat(rl_key);
        let released = rl.is_key_released(rl_key);
        if !pressed && !repeated && !released {
            continue;
        }

        let gkey = raylib_key_to_ghostty(rl_key);
        if gkey == Key::Unidentified {
            continue;
        }

        event.set_key(gkey);
        event.set_action(if released {
            key::Action::Release
        } else if pressed {
            key::Action::Press
        } else {
            key::Action::Repeat
        });
        event.set_mods(mods);

        let ucp = raylib_key_unshifted_codepoint(rl_key);
        event.set_unshifted_codepoint(ucp);

        let mut consumed = key::Mods::empty();
        if ucp != '\0' && mods.contains(key::Mods::SHIFT) {
            consumed |= key::Mods::SHIFT;
        }
        event.set_consumed_mods(consumed);

        if char_utf8_len > 0 && !released {
            event.set_utf8(Some(text));
            char_utf8_len = 0;
        } else {
            event.set_utf8(None);
        }

        let mut buf = [0u8; 128];
        match encoder.encode(event, &mut buf) {
            Ok(written) if written > 0 => pty_write(&pty_fd, &buf[..written]),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Mouse handling
// ---------------------------------------------------------------------------

/// Encode a mouse event and write the resulting escape sequence to the pty.
/// If the encoder produces no output (e.g. tracking is disabled), this is
/// a no-op.
fn mouse_encode_and_write<Fd: AsFd>(
    pty_fd: Fd,
    encoder: &mut mouse::Encoder,
    event: &mouse::Event,
) {
    let mut buf = [0u8; 128];
    match encoder.encode(event, &mut buf) {
        Ok(written) if written > 0 => pty_write(pty_fd, &buf[..written]),
        _ => {}
    }
}

/// Poll raylib for mouse events and use the libghostty mouse encoder
/// to produce the correct VT escape sequences, which are then written
/// to the pty. The encoder handles tracking mode (X10, normal, button,
/// any-event) and output format (X10, UTF8, SGR, URxvt, SGR-Pixels)
/// based on what the terminal application has requested.
fn handle_mouse<Fd: AsFd>(
    rl: &RaylibHandle,
    pty_fd: &Fd,
    encoder: &mut mouse::Encoder,
    event: &mut mouse::Event,
    terminal: &mut Terminal,
    cell_width: u32,
    cell_height: u32,
    pad: u32,
) {
    // Provide the encoder with the current terminal geometry so it
    // can convert pixel positions to cell coordinates.
    let scr_w = rl.get_screen_width() as u32;
    let scr_h = rl.get_screen_height() as u32;

    // Track whether any button is currently held -- the encoder uses
    // this to distinguish drags from plain motion.
    let any_pressed = rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT)
        || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT)
        || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE);

    encoder
        // Sync encoder tracking mode and format from terminal state so
        // mode changes (e.g. applications enabling SGR mouse reporting)
        // are honoured automatically.
        .with_options_from_terminal(terminal)
        .with_size(mouse::EncoderSize {
            screen_width: scr_w,
            screen_height: scr_h,
            cell_width: cell_width,
            cell_height: cell_height,
            padding_top: pad,
            padding_bottom: pad,
            padding_left: pad,
            padding_right: pad,
        })
        .with_any_button_pressed(any_pressed)
        // Enable motion deduplication so the encoder suppresses redundant
        // motion events within the same cell.
        .with_track_last_cell(true);

    let mods = get_ghostty_mods(rl);
    let pos = rl.get_mouse_position();
    event.set_mods(mods);
    event.set_position(mouse::Position { x: pos.x, y: pos.y });

    // Check each mouse button for press/release events.
    const BUTTONS: [MouseButton; 7] = [
        MouseButton::MOUSE_BUTTON_LEFT,
        MouseButton::MOUSE_BUTTON_RIGHT,
        MouseButton::MOUSE_BUTTON_MIDDLE,
        MouseButton::MOUSE_BUTTON_SIDE,
        MouseButton::MOUSE_BUTTON_EXTRA,
        MouseButton::MOUSE_BUTTON_FORWARD,
        MouseButton::MOUSE_BUTTON_BACK,
    ];

    for rl_btn in BUTTONS {
        let gbtn = raylib_mouse_to_ghostty(rl_btn);

        if rl.is_mouse_button_pressed(rl_btn) {
            event.set_action(mouse::Action::Press);
            event.set_button(Some(gbtn));
            mouse_encode_and_write(&pty_fd, encoder, event);
        } else if rl.is_mouse_button_released(rl_btn) {
            event.set_action(mouse::Action::Release);
            event.set_button(Some(gbtn));
            mouse_encode_and_write(&pty_fd, encoder, event);
        }
    }

    // Mouse motion -- send a motion event with whatever button is held
    // (or no button for pure motion in any-event tracking mode).
    let delta = rl.get_mouse_delta();
    if delta.x != 0.0 || delta.y != 0.0 {
        event.set_action(mouse::Action::Motion);
        event.set_button(if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
            Some(mouse::Button::Left)
        } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT) {
            Some(mouse::Button::Right)
        } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE) {
            Some(mouse::Button::Middle)
        } else {
            None
        });
        mouse_encode_and_write(pty_fd, encoder, event);
    }

    // Scroll wheel handling. When a mouse tracking mode is active the
    // wheel events are forwarded to the application as button 4/5
    // press+release pairs. Otherwise we scroll the viewport through
    // the scrollback buffer so the user can review history.
    let wheel = rl.get_mouse_wheel_move();
    if wheel != 0.0 {
        let mouse_tracking = is_mouse_tracking_enabled(terminal);

        if mouse_tracking {
            // Forward to the application via the mouse encoder.
            let scroll_btn = if wheel > 0.0 {
                mouse::Button::Four
            } else {
                mouse::Button::Five
            };
            event.set_button(Some(scroll_btn));
            event.set_action(mouse::Action::Press);
            mouse_encode_and_write(pty_fd, encoder, event);
            event.set_action(mouse::Action::Release);
            mouse_encode_and_write(pty_fd, encoder, event);
        } else {
            // Scroll the viewport through scrollback. Scroll 3 rows
            // per wheel tick for a comfortable pace.
            let scroll_delta: isize = if wheel > 0.0 { -3 } else { 3 };
            terminal.scroll_viewport(terminal::ScrollViewport::Delta(scroll_delta));
        }
    }
}

/// Check whether any mouse tracking mode is enabled on the terminal.
fn is_mouse_tracking_enabled(terminal: &Terminal) -> bool {
    [
        Mode::X10Mouse,
        Mode::NormalMouse,
        Mode::ButtonMouse,
        Mode::AnyMouse,
    ]
    .into_iter()
    .any(|mode| matches!(terminal.mode(mode), Ok(true)))
}

// ---------------------------------------------------------------------------
// Scrollbar
// ---------------------------------------------------------------------------

/// Handle scrollbar drag-to-scroll interaction.
///
/// When the user clicks in the scrollbar region and drags, we compute
/// the target scroll offset from the mouse Y position and scroll the
/// terminal viewport accordingly. Returns true if the scrollbar consumed
/// the mouse event (so handle_mouse should skip it).
fn handle_scrollbar(
    rl: &RaylibHandle,
    terminal: &mut Terminal,
    render_state: &mut RenderState,
    dragging: &mut bool,
) -> bool {
    let scrollbar = match terminal.scrollbar() {
        Ok(sb) => sb,
        Err(_) => {
            *dragging = false;
            return false;
        }
    };

    if scrollbar.total <= scrollbar.len {
        *dragging = false;
        return false;
    }

    let scr_w = rl.get_screen_width();
    let scr_h = rl.get_screen_height();
    let hit_left = scr_w - 16;
    let mpos = rl.get_mouse_position();

    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
        && mpos.x >= hit_left as f32
        && mpos.x <= scr_w as f32
    {
        *dragging = true;
    }

    if *dragging && rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
        let scrollable = scrollbar.total - scrollbar.len;
        let frac = (mpos.y as f64 / scr_h as f64).clamp(0.0, 1.0);
        let target = (frac * scrollable as f64) as i64;
        let delta = target - scrollbar.offset as i64;

        if delta != 0 {
            terminal.scroll_viewport(terminal::ScrollViewport::Delta(delta as isize));
            let _ = render_state.update(terminal);
        }
    }

    if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
        *dragging = false;
    }

    *dragging
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the terminal contents using the render state API.
///
/// Iterates over rows and cells from the render state, resolves styles
/// and colors, and draws each cell using raylib's 2D text rendering.
/// Also draws the cursor and an optional scrollbar thumb.
fn render_terminal<'alloc>(
    d: &mut RaylibDrawHandle,
    snapshot: &Snapshot<'alloc, '_>,
    rows: &mut RowIterator<'alloc>,
    cells: &mut CellIterator<'alloc>,
    font: &impl AsRef<raylib::ffi::Font>,
    cell_width: i32,
    cell_height: i32,
    font_size: i32,
    scrollbar: Option<&ffi::GhosttyTerminalScrollbar>,
) -> Result<()> {
    let Ok(colors) = snapshot.colors() else {
        return Ok(());
    };

    // If both fg and bg are black (no palette loaded yet), use white
    // foreground so text is visible on the default black background.
    let mut fg_default = colors.foreground;
    if fg_default.r == 0
        && fg_default.g == 0
        && fg_default.b == 0
        && colors.background.r == 0
        && colors.background.g == 0
        && colors.background.b == 0
    {
        fg_default = RgbColor {
            r: 255,
            g: 255,
            b: 255,
        };
    }

    let Ok(mut row_iter) = rows.update(snapshot) else {
        return Ok(());
    };

    let pad = 4;
    let mut y = pad;

    while let Some(row) = row_iter.next() {
        let Ok(mut cell_iter) = cells.update(row) else {
            continue;
        };

        let mut x = pad;
        while let Some(cell) = cell_iter.next() {
            let grapheme_len = cell.graphemes_len().unwrap_or(0);

            if grapheme_len == 0 {
                // Empty cell -- check for background-only content (palette
                // or direct RGB background without text).
                if let Ok(raw_cell) = cell.raw_cell() {
                    use ghostty::screen::CellContentTag;

                    match raw_cell.content_tag() {
                        Ok(CellContentTag::BgColorPalette) => {
                            if let Ok(palette_idx) = raw_cell.bg_color_palette() {
                                let bg = colors.palette[palette_idx.0 as usize];
                                d.draw_rectangle(
                                    x,
                                    y,
                                    cell_width,
                                    cell_height,
                                    Color::new(bg.r, bg.g, bg.b, 255),
                                );
                            }
                        }
                        Ok(CellContentTag::BgColorRgb) => {
                            if let Ok(bg) = raw_cell.bg_color_rgb() {
                                d.draw_rectangle(
                                    x,
                                    y,
                                    cell_width,
                                    cell_height,
                                    Color::new(bg.r, bg.g, bg.b, 255),
                                );
                            }
                        }
                        _ => {}
                    }
                }
                x += cell_width;
                continue;
            }

            // Read grapheme codepoints and encode to a UTF-8 string.
            let codepoints = cell.graphemes()?;

            let mut text_buf = [0u8; 64];
            let mut pos: usize = 0;
            for cp in &codepoints {
                if pos >= 60 {
                    break;
                }
                _ = cp.encode_utf8(&mut text_buf[pos..]);
                pos += cp.len_utf8();
            }
            let text = std::str::from_utf8(&text_buf[..pos]).expect("a valid UTF-8 string");

            // Resolve foreground and background colors using the new
            // per-cell color queries.  These flatten style colors,
            // content-tag colors, and palette lookups into a single RGB
            // value, returning INVALID_VALUE when the cell has no
            // explicit color (in which case we use the terminal default).
            let mut fg = cell.fg_color()?.unwrap_or(colors.foreground);
            let bg = cell.bg_color()?;
            let mut has_bg = bg.is_some();
            let mut bg = bg.unwrap_or(colors.background);

            // Read the style for flags (inverse, bold, italic) — color
            // resolution is handled above via the new API.
            let style = cell.style()?;
            if style.inverse {
                std::mem::swap(&mut fg, &mut bg);
                has_bg = true;
            }

            let ray_fg = Color::new(fg.r, fg.g, fg.b, 255);

            // Draw a background rectangle if the cell has a non-default bg
            // or if inverse mode forced a swap.
            if has_bg {
                d.draw_rectangle(
                    x,
                    y,
                    cell_width,
                    cell_height,
                    Color::new(bg.r, bg.g, bg.b, 255),
                );
            }

            // Italic: apply a simple shear by shifting the top of the glyph
            // to the right.  The offset is proportional to font size so it
            // looks reasonable at any scale.
            let italic_offset = if style.italic { font_size / 6 } else { 0 };

            d.draw_text_ex(
                font,
                text,
                Vector2::new((x + italic_offset) as f32, y as f32),
                font_size as f32,
                0.0,
                ray_fg,
            );

            // Fake bold by drawing the text again offset by 1px.
            if style.bold {
                d.draw_text_ex(
                    font,
                    text,
                    Vector2::new((x + italic_offset + 1) as f32, y as f32),
                    font_size as f32,
                    0.0,
                    ray_fg,
                );
            }

            x += cell_width;
        }

        // Mark the row as clean so we don't redraw it unnecessarily
        // on the next frame (the render state tracks per-row dirty flags).
        row_iter.set_dirty(false)?;
        y += cell_height;
    }

    // Draw cursor.
    let cursor_visible = snapshot.cursor_visible().unwrap_or(false);

    if cursor_visible && let Ok(Some(viewport)) = snapshot.cursor_viewport() {
        let cur_rgb = colors.cursor.unwrap_or(fg_default);
        let cur_x = pad + viewport.x as i32 * cell_width;
        let cur_y = pad + viewport.y as i32 * cell_height;
        d.draw_rectangle(
            cur_x,
            cur_y,
            cell_width,
            cell_height,
            Color::new(cur_rgb.r, cur_rgb.g, cur_rgb.b, 128),
        );
    }

    // Draw scrollbar thumb.
    if let Some(sb) = scrollbar {
        if sb.total > sb.len {
            let scr_w = d.get_screen_width();
            let scr_h = d.get_screen_height();
            let bar_width = 6;
            let bar_margin = 2;
            let bar_x = scr_w - bar_width - bar_margin;

            let visible_frac = sb.len as f64 / sb.total as f64;
            let thumb_height = ((scr_h as f64 * visible_frac) as i32).max(10);

            let scroll_frac = if sb.total > sb.len {
                sb.offset as f64 / (sb.total - sb.len) as f64
            } else {
                1.0
            };
            let thumb_y = (scroll_frac * (scr_h - thumb_height) as f64) as i32;

            d.draw_rectangle(
                bar_x,
                thumb_y,
                bar_width,
                thumb_height,
                Color::new(200, 200, 200, 128),
            );
        }
    }

    // Clear the global dirty flag so we know when the next update
    // actually changes something.
    snapshot.set_dirty(Dirty::Clean)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Build info
// ---------------------------------------------------------------------------

/// Log libghostty-vt build configuration (SIMD, optimization level).
fn log_build_info() {
    use ghostty::build_info::*;
    let simd = supports_simd().unwrap_or(false);
    let opt = optimization_mode().unwrap_or(OptimizeMode::Debug);

    eprintln!(
        "ghostty-vt: simd: {}, optimize: {opt:?}",
        if simd { "enabled" } else { "disabled" }
    );
}

// ioctl wrapper
nix::ioctl_write_ptr_bad!(tiocswinsz, libc::TIOCSWINSZ, nix::pty::Winsize);

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

// TODO: Port to native types
fn get_device_attributes() -> Option<ffi::GhosttyDeviceAttributes> {
    let mut da1_features = [0u16; 64];
    da1_features[0] = ffi::GHOSTTY_DA_FEATURE_COLUMNS_132 as u16;
    da1_features[1] = ffi::GHOSTTY_DA_FEATURE_SELECTIVE_ERASE as u16;
    da1_features[2] = ffi::GHOSTTY_DA_FEATURE_ANSI_COLOR as u16;

    Some(ffi::GhosttyDeviceAttributes {
        // DA1: VT220-level with a few common features.
        primary: ffi::GhosttyDeviceAttributesPrimary {
            conformance_level: ffi::GHOSTTY_DA_CONFORMANCE_VT220 as u16,
            features: da1_features,
            num_features: 3,
        },

        // DA2: VT220-type, version 1, no ROM cartridge.
        secondary: ffi::GhosttyDeviceAttributesSecondary {
            device_type: ffi::GHOSTTY_DA_DEVICE_TYPE_VT220 as u16,
            firmware_version: 1,
            rom_cartridge: 0,
        },

        // DA3: arbitrary unit id.
        tertiary: ffi::GhosttyDeviceAttributesTertiary { unit_id: 0 },
    })
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    if let Err(e) = run() {
        eprintln!("ghostling_rs failed: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    log_build_info();

    let font_size: i32 = 16;
    let (rl, thread) = raylib::init()
        .size(800, 600)
        .title("ghostling")
        .resizable()
        .build();
    let rl = Rc::new(RefCell::new(rl));

    rl.borrow_mut().set_target_fps(60);

    // Use raylib's default font. Replace with LoadFontFromMemory() and an
    // embedded TTF (e.g. JetBrains Mono) for proper monospace rendering.
    let mono_font = rl.borrow().get_font_default();

    // Measure a glyph to determine cell dimensions.
    let glyph_size = mono_font.measure_text("M", font_size as f32, 0.0);
    let cell_width = (glyph_size.x as i32).max(1);
    let cell_height = (glyph_size.y as i32).max(1);

    let pad = 4;
    let scr_w = rl.borrow().get_screen_width();
    let scr_h = rl.borrow().get_screen_height();
    let term_cols = Rc::new(Cell::new(((scr_w - 2 * pad) / cell_width).max(1) as u16));
    let term_rows = Rc::new(Cell::new(((scr_h - 2 * pad) / cell_height).max(1) as u16));

    let (pty_fd, child) = unsafe { pty_spawn(term_cols.get(), term_rows.get()) }
        .map_err(|e| format!("forkpty failed: {e}"))?;

    let mut terminal = Terminal::new(TerminalOptions {
        cols: term_cols.get(),
        rows: term_rows.get(),
        max_scrollback: 1000,
    })?;

    terminal
        // write_pty effect — the terminal calls this whenever a VT sequence
        // requires a response back to the application (device status reports,
        // mode queries, device attributes, etc.).  Without this, programs like
        // vim and tmux that probe terminal capabilities would hang.
        .on_pty_write(|_term, data| {
            pty_write(&pty_fd, data);
        })?
        // size effect — responds to XTWINOPS size queries (CSI 14/16/18 t)
        // so programs can discover the terminal geometry in cells and pixels.
        .on_size({
            let term_cols = term_cols.clone();
            let term_rows = term_rows.clone();
            move |_term| {
                // TODO
                Some(ffi::GhosttySizeReportSize {
                    rows: term_rows.get(),
                    columns: term_cols.get(),
                    cell_width: cell_width as u32,
                    cell_height: cell_height as u32,
                })
            }
        })?
        // device_attributes effect — responds to DA1/DA2/DA3 queries so
        // terminal applications can identify the terminal's capabilities.
        // We report VT220-level conformance with a modest feature set.
        .on_device_attributes(|_term| get_device_attributes())?
        // xtversion effect — responds to CSI > q with our application name.
        .on_xtversion(|_term| Some("ghostling-rs"))?
        // title_changed effect — updates the raylib window title whenever the
        // terminal receives an OSC 0 or OSC 2 title-setting sequence.
        .on_title_changed(|term| {
            let Ok(title) = term.title() else {
                return;
            };
            rl.borrow().set_window_title(&thread, title);
        })?
        // color_scheme effect — responds to CSI ? 996 n.  Raylib has no API to
        // query the OS color scheme, so we return false to silently ignore the
        // query rather than guessing.
        .on_color_scheme(|_term| None)?;

    let mut key_encoder = key::Encoder::new()?;
    let mut key_event = key::Event::new()?;
    let mut mouse_encoder = mouse::Encoder::new()?;
    let mut mouse_event = mouse::Event::new()?;
    let mut render_state = RenderState::new()?;
    let mut rows = RowIterator::new()?;
    let mut cells = CellIterator::new()?;

    let mut prev_width = scr_w;
    let mut prev_height = scr_h;
    let mut prev_focused = rl.borrow().is_window_focused();
    let mut scrollbar_dragging = false;
    let mut child_exited = false;
    let mut child_reaped = false;
    let mut child_exit_status: i32 = -1;

    while !rl.borrow().window_should_close() {
        // --- Resize ----------------------------------------------------------
        if rl.borrow().is_window_resized() {
            let w = rl.borrow().get_screen_width();
            let h = rl.borrow().get_screen_height();
            if w != prev_width || h != prev_height {
                let cols = ((w - 2 * pad) / cell_width).max(1) as u16;
                let rows = ((h - 2 * pad) / cell_height).max(1) as u16;
                term_rows.set(rows);
                term_cols.set(cols);
                terminal.resize(cols, rows, cell_width as u32, cell_height as u32)?;

                // Notify the pty of the new window size so the shell
                // and child programs can reflow their output.
                let new_ws = nix::pty::Winsize {
                    ws_row: rows,
                    ws_col: cols,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                };

                _ = unsafe { tiocswinsz(pty_fd.as_raw_fd(), &new_ws) };
                prev_width = w;
                prev_height = h;
            }
        }

        // --- Focus tracking --------------------------------------------------
        let focused = rl.borrow().is_window_focused();
        if focused != prev_focused {
            if !child_exited {
                // Send focus gained/lost if the terminal has focus reporting enabled.
                if let Ok(true) = terminal.mode(Mode::FocusEvent) {
                    let focus_event = if focused {
                        focus::Event::Gained
                    } else {
                        focus::Event::Lost
                    };
                    let mut focus_buf = [0u8; 8];
                    if let Ok(written) = focus_event.encode(&mut focus_buf) {
                        if written > 0 {
                            pty_write(&pty_fd, &focus_buf[..written]);
                        }
                    }
                }
            }
            prev_focused = focused;
        }

        // --- PTY read --------------------------------------------------------
        if !child_exited {
            let rc = pty_read(&pty_fd, &mut terminal);
            if rc != PtyReadResult::Ok {
                child_exited = true;
            }
        }

        // --- Reap child ------------------------------------------------------
        if child_exited && !child_reaped {
            if let Ok(wp) = wait::waitpid(child, Some(wait::WaitPidFlag::WNOHANG)) {
                child_reaped = true;
                match wp {
                    wait::WaitStatus::Exited(_, status) => child_exit_status = status,
                    wait::WaitStatus::Signaled(_, sig, _) => child_exit_status = 128 + sig as i32,
                    _ => {}
                }
            }
        }

        // --- Scrollbar -------------------------------------------------------
        let scrollbar_consumed = handle_scrollbar(
            &rl.borrow(),
            &mut terminal,
            &mut render_state,
            &mut scrollbar_dragging,
        );

        // --- Input -----------------------------------------------------------
        if !child_exited {
            handle_input(
                &mut rl.borrow_mut(),
                &pty_fd,
                &mut key_encoder,
                &mut key_event,
                &terminal,
            );
            if !scrollbar_consumed {
                handle_mouse(
                    &rl.borrow(),
                    &pty_fd,
                    &mut mouse_encoder,
                    &mut mouse_event,
                    &mut terminal,
                    cell_width as u32,
                    cell_height as u32,
                    pad as u32,
                );
            }
        }

        // --- Update render state ---------------------------------------------
        let snapshot = render_state.update(&mut terminal)?;

        // --- Draw ------------------------------------------------------------
        let bg_colors = snapshot.colors()?;
        let win_bg = Color::new(
            bg_colors.background.r,
            bg_colors.background.g,
            bg_colors.background.b,
            255,
        );

        let scrollbar = terminal.scrollbar().ok();

        {
            let mut rl = rl.borrow_mut();
            let mut d = rl.begin_drawing(&thread);
            d.clear_background(win_bg);

            render_terminal(
                &mut d,
                &snapshot,
                &mut rows,
                &mut cells,
                &mono_font,
                cell_width,
                cell_height,
                font_size,
                scrollbar.as_ref(),
            )?;

            // Show an exit banner when the child process has terminated.
            if child_exited {
                let exit_msg = if child_exit_status >= 0 {
                    format!("[process exited with status {child_exit_status}]")
                } else {
                    "[process exited]".to_owned()
                };

                let msg_size = mono_font.measure_text(&exit_msg, font_size as f32, 0.0);
                let screen_w = d.get_screen_width();
                let screen_h = d.get_screen_height();
                let banner_h = msg_size.y as i32 + 8;

                d.draw_rectangle(
                    0,
                    screen_h - banner_h,
                    screen_w,
                    banner_h,
                    Color::new(0, 0, 0, 180),
                );
                d.draw_text_ex(
                    &mono_font,
                    &exit_msg,
                    Vector2::new(
                        (screen_w as f32 - msg_size.x) / 2.0,
                        (screen_h - banner_h + 4) as f32,
                    ),
                    font_size as f32,
                    0.0,
                    Color::WHITE,
                );
            }
        }
    }

    // --- Cleanup -------------------------------------------------------------
    if !child_reaped {
        if !child_exited {
            _ = signal::kill(child, signal::SIGHUP);
        }
        _ = wait::waitpid(child, None)
    }

    Ok(())
}
