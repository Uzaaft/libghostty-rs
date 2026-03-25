//! Ghostling (Rust) — minimal terminal emulator built on libghostty-rs.
//!
//! This is a Rust port of the C ghostling example from ghostty-org/ghostling.
//! It uses Raylib for windowing/rendering and libghostty-vt (via the safe
//! `ghostty` crate) for terminal emulation. The architecture is intentionally
//! simple: single-threaded, 2D software rendering, one file.

use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use ghostty::ffi;
use ghostty::{
    KeyEncoder, KeyEvent, MouseEncoder, MouseEvent, RenderState, RenderStateRowCells,
    RenderStateRowIterator, Terminal,
};
use raylib::prelude::*;

/// Embedded monospace font used by the renderer.
///
/// Using a specific TTF keeps glyph metrics and rasterization behavior aligned
/// with upstream ghostling and avoids platform-dependent defaults.
const JETBRAINS_MONO_TTF: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

/// Unicode blocks to bake into the font atlas.
///
/// Raylib defaults to 95 printable ASCII codepoints when none are provided,
/// which causes many prompt symbols and terminal drawing characters to render
/// as '?'. We load a broader terminal-focused set to avoid that fallback.
const TERMINAL_FONT_CODEPOINT_RANGES: &[(u32, u32)] = &[
    (0x0020, 0x007E), // Basic Latin
    (0x00A0, 0x00FF), // Latin-1 Supplement
    (0x0100, 0x024F), // Latin Extended
    (0x0370, 0x03FF), // Greek
    (0x0400, 0x052F), // Cyrillic
    (0x2000, 0x206F), // General punctuation
    (0x20A0, 0x20CF), // Currency symbols
    (0x2100, 0x214F), // Letterlike symbols
    (0x2190, 0x21FF), // Arrows
    (0x2200, 0x22FF), // Math operators
    (0x2300, 0x23FF), // Misc technical
    (0x2460, 0x24FF), // Enclosed alphanumerics
    (0x2500, 0x259F), // Box drawing + block elements
    (0x25A0, 0x25FF), // Geometric shapes
    (0x2600, 0x27BF), // Misc symbols + dingbats (includes ❯)
    (0x2800, 0x28FF), // Braille patterns
    (0x2B00, 0x2BFF), // Misc symbols and arrows
    (0xE0A0, 0xE0D7), // Powerline private-use symbols
];

fn build_terminal_font_codepoints() -> Result<Vec<i32>, String> {
    let mut codepoints: Vec<i32> = Vec::new();

    for &(start, end) in TERMINAL_FONT_CODEPOINT_RANGES {
        if start > end {
            return Err(format!(
                "invalid codepoint range for terminal font atlas: U+{start:04X}..=U+{end:04X}"
            ));
        }

        for cp in start..=end {
            let cp_i32 = i32::try_from(cp)
                .map_err(|_| format!("codepoint out of range for raylib: U+{cp:04X}"))?;
            codepoints.push(cp_i32);
        }
    }

    if codepoints.is_empty() {
        return Err("terminal font codepoint set must not be empty".to_owned());
    }

    Ok(codepoints)
}

fn load_embedded_mono_font(font_size_px: i32) -> Result<Font, String> {
    if font_size_px <= 0 {
        return Err(format!(
            "font_size_px must be greater than zero, got {font_size_px}"
        ));
    }

    let file_type = std::ffi::CString::new(".ttf")
        .map_err(|error| format!("invalid embedded font extension: {error}"))?;
    let font_data_len = i32::try_from(JETBRAINS_MONO_TTF.len())
        .map_err(|_| "embedded font is too large for raylib API".to_owned())?;
    let codepoints = build_terminal_font_codepoints()?;
    let codepoint_count = i32::try_from(codepoints.len())
        .map_err(|_| "font codepoint set is too large for raylib API".to_owned())?;

    let raw_font = unsafe {
        raylib::ffi::LoadFontFromMemory(
            file_type.as_ptr(),
            JETBRAINS_MONO_TTF.as_ptr(),
            font_data_len,
            font_size_px,
            codepoints.as_ptr().cast_mut(),
            codepoint_count,
        )
    };

    if raw_font.glyphs.is_null() || raw_font.texture.id == 0 {
        return Err("raylib failed to create a usable font atlas".to_owned());
    }

    let font = unsafe { Font::from_raw(raw_font) };
    if !font.is_font_valid() {
        return Err("raylib produced invalid font metadata".to_owned());
    }

    Ok(font)
}

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
fn pty_spawn(cols: u16, rows: u16) -> io::Result<(OwnedFd, libc::pid_t)> {
    let mut ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // forkpty() combines openpty + fork + login_tty into one call.
    // In the child it sets up the slave side as stdin/stdout/stderr.
    let mut master_fd: RawFd = -1;
    let child = unsafe {
        libc::forkpty(
            &mut master_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };

    if child < 0 {
        return Err(io::Error::last_os_error());
    }

    if child == 0 {
        // Determine the user's preferred shell. We try $SHELL first (the
        // standard convention), then fall back to the passwd entry, and
        // finally to /bin/sh if nothing else is available.
        let shell = std::env::var("SHELL")
            .ok()
            .and_then(|s| if s.is_empty() { None } else { Some(s) });

        let shell = shell.unwrap_or_else(|| {
            unsafe {
                let pw = libc::getpwuid(libc::getuid());
                if !pw.is_null() {
                    let shell_ptr = (*pw).pw_shell;
                    if !shell_ptr.is_null() {
                        let c_str = std::ffi::CStr::from_ptr(shell_ptr);
                        if let Ok(s) = c_str.to_str() {
                            if !s.is_empty() {
                                return s.to_owned();
                            }
                        }
                    }
                }
            }
            "/bin/sh".to_owned()
        });

        // Extract just the program name for argv[0] (e.g. "/bin/zsh" -> "zsh").
        let shell_name = shell.rsplit('/').next().unwrap_or(&shell);

        // Child process -- replace ourselves with the shell.
        // TERM tells programs what escape sequences we understand.
        unsafe {
            let term = std::ffi::CString::new("TERM").unwrap();
            let term_val = std::ffi::CString::new("xterm-256color").unwrap();
            libc::setenv(term.as_ptr(), term_val.as_ptr(), 1);

            let c_shell = std::ffi::CString::new(shell.clone()).unwrap();
            let c_name = std::ffi::CString::new(shell_name).unwrap();
            libc::execl(
                c_shell.as_ptr(),
                c_name.as_ptr(),
                std::ptr::null::<libc::c_char>(),
            );
            libc::_exit(127); // execl only returns on error
        }
    }

    // Parent -- make the master fd non-blocking so read() returns EAGAIN
    // instead of blocking when there's no data, letting us poll each frame.
    let flags = unsafe { libc::fcntl(master_fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let rc = unsafe { libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }

    let owned = unsafe { OwnedFd::from_raw_fd(master_fd) };
    Ok((owned, child))
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
/// Because the fd is non-blocking, read() returns -1 with EAGAIN once
/// the kernel buffer is empty, at which point we stop.
fn pty_read(fd: &OwnedFd, terminal: &mut Terminal) -> PtyReadResult {
    let raw_fd = fd.as_raw_fd();
    let mut buf = [0u8; 4096];

    loop {
        let n = unsafe { libc::read(raw_fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            terminal.vt_write(&buf[..n as usize]);
        } else if n == 0 {
            // EOF -- the child closed its side of the pty.
            return PtyReadResult::Eof;
        } else {
            // n == -1: distinguish "no data right now" from real errors.
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::EAGAIN) => return PtyReadResult::Ok,
                Some(libc::EINTR) => continue, // retry the read
                // On Linux, the slave closing often produces EIO rather
                // than a clean EOF (read returning 0). Treat it the same.
                Some(libc::EIO) => return PtyReadResult::Eof,
                _ => {
                    eprintln!("pty read: {err}");
                    return PtyReadResult::Error;
                }
            }
        }
    }
}

/// Best-effort write to the pty master fd. Because the fd is non-blocking,
/// write() may return short or fail with EAGAIN. We retry on EINTR, advance
/// past partial writes, and silently drop data if the kernel buffer is full
/// -- this matches what most terminal emulators do under back-pressure.
fn pty_write(fd: &OwnedFd, data: &[u8]) {
    let raw_fd = fd.as_raw_fd();
    let mut remaining = data;

    while !remaining.is_empty() {
        let n = unsafe { libc::write(raw_fd, remaining.as_ptr().cast(), remaining.len()) };
        if n > 0 {
            remaining = &remaining[n as usize..];
        } else if n < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            // EAGAIN or real error -- drop the remainder.
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Input handling
// ---------------------------------------------------------------------------

/// Map a raylib key constant to a GhosttyKey code.
/// Returns GHOSTTY_KEY_UNIDENTIFIED for keys we don't handle.
fn raylib_key_to_ghostty(rl_key: KeyboardKey) -> ffi::GhosttyKey {
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

    if k >= key_a && k <= key_z {
        return ffi::GhosttyKey_GHOSTTY_KEY_A + (k - key_a);
    }
    // Digits -- raylib KEY_ZERO..KEY_NINE are contiguous.
    if k >= key_zero && k <= key_nine {
        return ffi::GhosttyKey_GHOSTTY_KEY_DIGIT_0 + (k - key_zero);
    }
    // Function keys -- raylib KEY_F1..KEY_F12 are contiguous.
    if k >= key_f1 && k <= key_f12 {
        return ffi::GhosttyKey_GHOSTTY_KEY_F1 + (k - key_f1);
    }

    match rl_key {
        KEY_SPACE => ffi::GhosttyKey_GHOSTTY_KEY_SPACE,
        KEY_ENTER => ffi::GhosttyKey_GHOSTTY_KEY_ENTER,
        KEY_TAB => ffi::GhosttyKey_GHOSTTY_KEY_TAB,
        KEY_BACKSPACE => ffi::GhosttyKey_GHOSTTY_KEY_BACKSPACE,
        KEY_DELETE => ffi::GhosttyKey_GHOSTTY_KEY_DELETE,
        KEY_ESCAPE => ffi::GhosttyKey_GHOSTTY_KEY_ESCAPE,
        KEY_UP => ffi::GhosttyKey_GHOSTTY_KEY_ARROW_UP,
        KEY_DOWN => ffi::GhosttyKey_GHOSTTY_KEY_ARROW_DOWN,
        KEY_LEFT => ffi::GhosttyKey_GHOSTTY_KEY_ARROW_LEFT,
        KEY_RIGHT => ffi::GhosttyKey_GHOSTTY_KEY_ARROW_RIGHT,
        KEY_HOME => ffi::GhosttyKey_GHOSTTY_KEY_HOME,
        KEY_END => ffi::GhosttyKey_GHOSTTY_KEY_END,
        KEY_PAGE_UP => ffi::GhosttyKey_GHOSTTY_KEY_PAGE_UP,
        KEY_PAGE_DOWN => ffi::GhosttyKey_GHOSTTY_KEY_PAGE_DOWN,
        KEY_INSERT => ffi::GhosttyKey_GHOSTTY_KEY_INSERT,
        KEY_MINUS => ffi::GhosttyKey_GHOSTTY_KEY_MINUS,
        KEY_EQUAL => ffi::GhosttyKey_GHOSTTY_KEY_EQUAL,
        KEY_LEFT_BRACKET => ffi::GhosttyKey_GHOSTTY_KEY_BRACKET_LEFT,
        KEY_RIGHT_BRACKET => ffi::GhosttyKey_GHOSTTY_KEY_BRACKET_RIGHT,
        KEY_BACKSLASH => ffi::GhosttyKey_GHOSTTY_KEY_BACKSLASH,
        KEY_SEMICOLON => ffi::GhosttyKey_GHOSTTY_KEY_SEMICOLON,
        KEY_APOSTROPHE => ffi::GhosttyKey_GHOSTTY_KEY_QUOTE,
        KEY_COMMA => ffi::GhosttyKey_GHOSTTY_KEY_COMMA,
        KEY_PERIOD => ffi::GhosttyKey_GHOSTTY_KEY_PERIOD,
        KEY_SLASH => ffi::GhosttyKey_GHOSTTY_KEY_SLASH,
        KEY_GRAVE => ffi::GhosttyKey_GHOSTTY_KEY_BACKQUOTE,
        _ => ffi::GhosttyKey_GHOSTTY_KEY_UNIDENTIFIED,
    }
}

/// Return the unshifted Unicode codepoint for a raylib key, i.e. the
/// character the key produces with no modifiers on a US layout. The
/// Kitty keyboard protocol requires this to identify keys. Returns 0
/// for keys that don't have a natural codepoint (arrows, F-keys, etc.).
fn raylib_key_unshifted_codepoint(rl_key: KeyboardKey) -> u32 {
    use KeyboardKey::*;

    let key_a = KEY_A as u32;
    let key_z = KEY_Z as u32;
    let key_zero = KEY_ZERO as u32;
    let key_nine = KEY_NINE as u32;
    let k = rl_key as u32;

    if k >= key_a && k <= key_z {
        return b'a' as u32 + (k - key_a);
    }
    if k >= key_zero && k <= key_nine {
        return b'0' as u32 + (k - key_zero);
    }

    match rl_key {
        KEY_SPACE => b' ' as u32,
        KEY_MINUS => b'-' as u32,
        KEY_EQUAL => b'=' as u32,
        KEY_LEFT_BRACKET => b'[' as u32,
        KEY_RIGHT_BRACKET => b']' as u32,
        KEY_BACKSLASH => b'\\' as u32,
        KEY_SEMICOLON => b';' as u32,
        KEY_APOSTROPHE => b'\'' as u32,
        KEY_COMMA => b',' as u32,
        KEY_PERIOD => b'.' as u32,
        KEY_SLASH => b'/' as u32,
        KEY_GRAVE => b'`' as u32,
        _ => 0,
    }
}

/// Build a GhosttyMods bitmask from the current raylib modifier key state.
fn get_ghostty_mods(rl: &RaylibHandle) -> ffi::GhosttyMods {
    let mut mods: ffi::GhosttyMods = 0;
    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT) {
        mods |= ffi::GHOSTTY_MODS_SHIFT as u16;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
        || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL)
    {
        mods |= ffi::GHOSTTY_MODS_CTRL as u16;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_ALT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT) {
        mods |= ffi::GHOSTTY_MODS_ALT as u16;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER) || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER) {
        mods |= ffi::GHOSTTY_MODS_SUPER as u16;
    }
    mods
}

/// Map a raylib mouse button to a GhosttyMouseButton.
fn raylib_mouse_to_ghostty(rl_button: MouseButton) -> ffi::GhosttyMouseButton {
    match rl_button {
        MouseButton::MOUSE_BUTTON_LEFT => ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_LEFT,
        MouseButton::MOUSE_BUTTON_RIGHT => ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_RIGHT,
        MouseButton::MOUSE_BUTTON_MIDDLE => ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_MIDDLE,
        MouseButton::MOUSE_BUTTON_SIDE => ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FOUR,
        MouseButton::MOUSE_BUTTON_EXTRA => ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FIVE,
        MouseButton::MOUSE_BUTTON_FORWARD => ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SIX,
        MouseButton::MOUSE_BUTTON_BACK => ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SEVEN,
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
fn handle_input(
    rl: &RaylibHandle,
    pty_fd: &OwnedFd,
    encoder: &mut KeyEncoder,
    event: &mut KeyEvent,
    terminal: &Terminal,
) {
    // Sync encoder options from the terminal so mode changes (e.g.
    // application cursor keys, Kitty keyboard protocol) are honoured.
    encoder.setopt_from_terminal(terminal);

    // Drain printable characters from raylib's input queue. We collect
    // them into a single UTF-8 buffer so the encoder can attach text
    // to the key event.
    let mut char_utf8 = [0u8; 64];
    let mut char_utf8_len: usize = 0;
    loop {
        let ch = unsafe { raylib::ffi::GetCharPressed() };
        if ch == 0 {
            break;
        }
        let mut u8_buf = [0u8; 4];
        let n = ghostty::utf8_encode(ch as u32, &mut u8_buf);
        if char_utf8_len + n < char_utf8.len() {
            char_utf8[char_utf8_len..char_utf8_len + n].copy_from_slice(&u8_buf[..n]);
            char_utf8_len += n;
        }
    }

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
        if gkey == ffi::GhosttyKey_GHOSTTY_KEY_UNIDENTIFIED {
            continue;
        }

        let action = if released {
            ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_RELEASE
        } else if pressed {
            ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_PRESS
        } else {
            ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_REPEAT
        };

        event.set_key(gkey);
        event.set_action(action);
        event.set_mods(mods);

        let ucp = raylib_key_unshifted_codepoint(rl_key);
        event.set_unshifted_codepoint(ucp);

        let mut consumed: ffi::GhosttyMods = 0;
        if ucp != 0 && (mods & ffi::GHOSTTY_MODS_SHIFT as u16) != 0 {
            consumed |= ffi::GHOSTTY_MODS_SHIFT as u16;
        }
        event.set_consumed_mods(consumed);

        if char_utf8_len > 0 && !released {
            event.set_utf8(Some(&char_utf8[..char_utf8_len]));
            char_utf8_len = 0;
        } else {
            event.set_utf8(None);
        }

        let mut buf = [0u8; 128];
        match encoder.encode(event, &mut buf) {
            Ok(written) if written > 0 => pty_write(pty_fd, &buf[..written]),
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
fn mouse_encode_and_write(pty_fd: &OwnedFd, encoder: &mut MouseEncoder, event: &MouseEvent) {
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
fn handle_mouse(
    rl: &RaylibHandle,
    pty_fd: &OwnedFd,
    encoder: &mut MouseEncoder,
    event: &mut MouseEvent,
    terminal: &mut Terminal,
    cell_width: i32,
    cell_height: i32,
    pad: i32,
) {
    // Sync encoder tracking mode and format from terminal state so
    // mode changes (e.g. applications enabling SGR mouse reporting)
    // are honoured automatically.
    encoder.setopt_from_terminal(terminal);

    // Provide the encoder with the current terminal geometry so it
    // can convert pixel positions to cell coordinates.
    let scr_w = rl.get_screen_width();
    let scr_h = rl.get_screen_height();

    let mut enc_size = ffi::GhosttyMouseEncoderSize::default();
    enc_size.size = std::mem::size_of::<ffi::GhosttyMouseEncoderSize>();
    enc_size.screen_width = scr_w as u32;
    enc_size.screen_height = scr_h as u32;
    enc_size.cell_width = cell_width as u32;
    enc_size.cell_height = cell_height as u32;
    enc_size.padding_top = pad as u32;
    enc_size.padding_bottom = pad as u32;
    enc_size.padding_left = pad as u32;
    enc_size.padding_right = pad as u32;

    encoder.setopt(
        ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
        std::ptr::from_ref(&enc_size).cast(),
    );

    // Track whether any button is currently held -- the encoder uses
    // this to distinguish drags from plain motion.
    let any_pressed = rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT)
        || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT)
        || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE);
    encoder.setopt(
        ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
        std::ptr::from_ref(&any_pressed).cast(),
    );

    // Enable motion deduplication so the encoder suppresses redundant
    // motion events within the same cell.
    let track_cell = true;
    encoder.setopt(
        ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL,
        std::ptr::from_ref(&track_cell).cast(),
    );

    let mods = get_ghostty_mods(rl);
    let pos = rl.get_mouse_position();
    event.set_mods(mods);
    event.set_position(pos.x, pos.y);

    // Check each mouse button for press/release events.
    const BUTTONS: &[MouseButton] = &[
        MouseButton::MOUSE_BUTTON_LEFT,
        MouseButton::MOUSE_BUTTON_RIGHT,
        MouseButton::MOUSE_BUTTON_MIDDLE,
        MouseButton::MOUSE_BUTTON_SIDE,
        MouseButton::MOUSE_BUTTON_EXTRA,
        MouseButton::MOUSE_BUTTON_FORWARD,
        MouseButton::MOUSE_BUTTON_BACK,
    ];

    for &rl_btn in BUTTONS {
        let gbtn = raylib_mouse_to_ghostty(rl_btn);

        if rl.is_mouse_button_pressed(rl_btn) {
            event.set_action(ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_PRESS);
            event.set_button(gbtn);
            mouse_encode_and_write(pty_fd, encoder, event);
        } else if rl.is_mouse_button_released(rl_btn) {
            event.set_action(ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_RELEASE);
            event.set_button(gbtn);
            mouse_encode_and_write(pty_fd, encoder, event);
        }
    }

    // Mouse motion -- send a motion event with whatever button is held
    // (or no button for pure motion in any-event tracking mode).
    let delta = rl.get_mouse_delta();
    if delta.x != 0.0 || delta.y != 0.0 {
        event.set_action(ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_MOTION);
        if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
            event.set_button(ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_LEFT);
        } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT) {
            event.set_button(ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_RIGHT);
        } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE) {
            event.set_button(ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_MIDDLE);
        } else {
            event.clear_button();
        }
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
                ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FOUR
            } else {
                ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FIVE
            };
            event.set_button(scroll_btn);
            event.set_action(ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_PRESS);
            mouse_encode_and_write(pty_fd, encoder, event);
            event.set_action(ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_RELEASE);
            mouse_encode_and_write(pty_fd, encoder, event);
        } else {
            // Scroll the viewport through scrollback. Scroll 3 rows
            // per wheel tick for a comfortable pace.
            let scroll_delta: isize = if wheel > 0.0 { -3 } else { 3 };
            terminal.scroll_viewport_delta(scroll_delta);
        }
    }
}

/// Check whether any mouse tracking mode is enabled on the terminal.
///
/// The mode values correspond to DEC private modes 9 (X10), 1000 (normal),
/// 1002 (button), 1003 (any). In the packed GhosttyMode format, DEC private
/// modes have bit 15 set: mode_value = (1 << 15) | dec_mode_number.
fn is_mouse_tracking_enabled(terminal: &Terminal) -> bool {
    let tracking_mode_numbers: &[u16] = &[9, 1000, 1002, 1003];
    for &mode_num in tracking_mode_numbers {
        let mode: ffi::GhosttyMode = (1 << 15) | mode_num;
        if let Ok(true) = terminal.mode_get(mode) {
            return true;
        }
    }
    false
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
            terminal.scroll_viewport_delta(delta as isize);
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

/// Resolve a style color to an RGB value. Palette colors are looked up
/// in the render state's 256-color palette; RGB colors are used directly;
/// unset colors fall back to the provided default.
fn resolve_color(
    color: &ffi::GhosttyStyleColor,
    colors: &ffi::GhosttyRenderStateColors,
    fallback: ffi::GhosttyColorRgb,
) -> ffi::GhosttyColorRgb {
    match color.tag {
        ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_RGB => unsafe { color.value.rgb },
        ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE => {
            let idx = unsafe { color.value.palette };
            colors.palette[idx as usize]
        }
        _ => fallback,
    }
}

/// Render the terminal contents using the render state API.
///
/// Iterates over rows and cells from the render state, resolves styles
/// and colors, and draws each cell using raylib's 2D text rendering.
/// Also draws the cursor and an optional scrollbar thumb.
fn render_terminal(
    d: &mut RaylibDrawHandle,
    render_state: &mut RenderState,
    row_iter: &mut RenderStateRowIterator,
    cells: &mut RenderStateRowCells,
    font: &impl AsRef<raylib::ffi::Font>,
    cell_width: i32,
    cell_height: i32,
    font_size: i32,
    scrollbar: Option<&ffi::GhosttyTerminalScrollbar>,
) {
    let colors = match render_state.colors_get() {
        Ok(c) => c,
        Err(_) => return,
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
        fg_default = ffi::GhosttyColorRgb {
            r: 255,
            g: 255,
            b: 255,
        };
    }

    if render_state.populate_row_iterator(row_iter).is_err() {
        return;
    }

    let pad = 4;
    let mut y = pad;

    for row in row_iter.rows() {
        if row.populate_cells(cells).is_err() {
            y += cell_height;
            continue;
        }

        let mut x = pad;

        for cell in cells.cells() {
            let grapheme_len = cell.graphemes_len().unwrap_or(0);

            if grapheme_len == 0 {
                // Empty cell -- check for background-only content (palette
                // or direct RGB background without text).
                if let Ok(raw_cell) = cell.raw_cell() {
                    if let Ok(content_tag) = ghostty::cell_get_content_tag(raw_cell) {
                        if content_tag
                            == ffi::GhosttyCellContentTag_GHOSTTY_CELL_CONTENT_BG_COLOR_PALETTE
                        {
                            if let Ok(palette_idx) = ghostty::cell_get_color_palette(raw_cell) {
                                let bg = colors.palette[palette_idx as usize];
                                d.draw_rectangle(
                                    x,
                                    y,
                                    cell_width,
                                    cell_height,
                                    Color::new(bg.r, bg.g, bg.b, 255),
                                );
                            }
                        } else if content_tag
                            == ffi::GhosttyCellContentTag_GHOSTTY_CELL_CONTENT_BG_COLOR_RGB
                        {
                            if let Ok(bg) = ghostty::cell_get_color_rgb(raw_cell) {
                                d.draw_rectangle(
                                    x,
                                    y,
                                    cell_width,
                                    cell_height,
                                    Color::new(bg.r, bg.g, bg.b, 255),
                                );
                            }
                        }
                    }
                }
                x += cell_width;
                continue;
            }

            // Read grapheme codepoints and encode to a UTF-8 string.
            let mut codepoints = [0u32; 16];
            let len = grapheme_len.min(16) as usize;
            let _ = cell.graphemes_buf(&mut codepoints[..len]);

            let mut text_buf = [0u8; 64];
            let mut pos: usize = 0;
            for &cp in &codepoints[..len] {
                if pos >= 60 {
                    break;
                }
                let mut u8_buf = [0u8; 4];
                let n = ghostty::utf8_encode(cp, &mut u8_buf);
                text_buf[pos..pos + n].copy_from_slice(&u8_buf[..n]);
                pos += n;
            }
            text_buf[pos] = 0; // null-terminate for CStr

            // Resolve foreground, background, and style flags.
            let style = cell.style().unwrap_or_else(|_| {
                let mut s = ffi::GhosttyStyle::default();
                s.size = std::mem::size_of::<ffi::GhosttyStyle>();
                s
            });

            let mut fg = resolve_color(&style.fg_color, &colors, fg_default);
            let mut bg_rgb = resolve_color(&style.bg_color, &colors, colors.background);

            if style.inverse {
                std::mem::swap(&mut fg, &mut bg_rgb);
            }

            let ray_fg = Color::new(fg.r, fg.g, fg.b, 255);

            // Draw background if the cell has an explicit bg color or is inverted.
            if style.bg_color.tag != ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE
                || style.inverse
            {
                d.draw_rectangle(
                    x,
                    y,
                    cell_width,
                    cell_height,
                    Color::new(bg_rgb.r, bg_rgb.g, bg_rgb.b, 255),
                );
            }

            // Fake italic by shifting the text right slightly.
            let italic_offset = if style.italic { font_size / 6 } else { 0 };

            let text_cstr = unsafe { std::ffi::CStr::from_ptr(text_buf.as_ptr().cast()) };
            if let Ok(text_str) = text_cstr.to_str() {
                d.draw_text_ex(
                    font,
                    text_str,
                    Vector2::new((x + italic_offset) as f32, y as f32),
                    font_size as f32,
                    0.0,
                    ray_fg,
                );

                // Fake bold by drawing the text again offset by 1px.
                if style.bold {
                    d.draw_text_ex(
                        font,
                        text_str,
                        Vector2::new((x + italic_offset + 1) as f32, y as f32),
                        font_size as f32,
                        0.0,
                        ray_fg,
                    );
                }
            }

            x += cell_width;
        }

        // Mark the row as clean so we don't redraw it unnecessarily
        // on the next frame (the render state tracks per-row dirty flags).
        let _ = row.set_dirty(false);
        y += cell_height;
    }

    // Draw cursor.
    let cursor_visible = render_state.cursor_visible().unwrap_or(false);
    let cursor_in_viewport = render_state.cursor_viewport_has_value().unwrap_or(false);

    if cursor_visible && cursor_in_viewport {
        let cx = render_state.cursor_viewport_x().unwrap_or(0);
        let cy = render_state.cursor_viewport_y().unwrap_or(0);

        let cur_rgb = if colors.cursor_has_value {
            colors.cursor
        } else {
            fg_default
        };

        let cur_x = pad + cx as i32 * cell_width;
        let cur_y = pad + cy as i32 * cell_height;
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
    let _ = render_state.set_dirty(ffi::GhosttyRenderStateDirty_GHOSTTY_RENDER_STATE_DIRTY_FALSE);
}

// ---------------------------------------------------------------------------
// Build info
// ---------------------------------------------------------------------------

/// Log libghostty-vt build configuration (SIMD, optimization level).
fn log_build_info() {
    let simd = ghostty::build_info_simd().unwrap_or(false);
    let opt =
        ghostty::build_info_optimize().unwrap_or(ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_DEBUG);

    let opt_str = match opt {
        ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_DEBUG => "Debug",
        ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SAFE => "ReleaseSafe",
        ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SMALL => "ReleaseSmall",
        ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_FAST => "ReleaseFast",
        _ => "Unknown",
    };

    eprintln!(
        "ghostty-vt: simd: {}, optimize: {opt_str}",
        if simd { "enabled" } else { "disabled" }
    );
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

fn run() -> Result<(), Box<dyn std::error::Error>> {
    log_build_info();

    // Match upstream ghostling's HiDPI initialization flow.
    unsafe {
        raylib::ffi::SetConfigFlags(raylib::ffi::ConfigFlags::FLAG_WINDOW_HIGHDPI as u32);
    }

    let font_size: i32 = 16;
    let (mut rl, thread) = raylib::init()
        .size(800, 600)
        .title("ghostling")
        .resizable()
        .build();

    rl.set_target_fps(60);

    // Query window DPI so font rasterization happens at native pixel density.
    let dpi_scale = rl.get_window_scale_dpi();
    let dpi_x = if dpi_scale.x > 0.0 { dpi_scale.x } else { 1.0 };
    let dpi_y = if dpi_scale.y > 0.0 { dpi_scale.y } else { 1.0 };

    // Load JetBrains Mono at DPI-scaled pixel size, matching upstream.
    let font_size_px = ((font_size as f32 * dpi_y) as i32).max(1);
    let mono_font = load_embedded_mono_font(font_size_px)
        .map_err(|error| format!("failed to load embedded JetBrains Mono font: {error}"))?;

    // The font atlas is already created at native resolution, so bilinear
    // filtering smooths fractional positioning without magnification blur.
    mono_font.texture().set_texture_filter(
        &thread,
        raylib::consts::TextureFilter::TEXTURE_FILTER_BILINEAR,
    );

    // Measure a representative glyph in pixel space, then convert back to
    // logical screen space for terminal cell layout math.
    let glyph_size = mono_font.measure_text("M", font_size_px as f32, 0.0);
    let cell_width = ((glyph_size.x / dpi_x) as i32).max(1);
    let cell_height = ((glyph_size.y / dpi_y) as i32).max(1);

    let pad = 4;
    let scr_w = rl.get_screen_width();
    let scr_h = rl.get_screen_height();
    let term_cols = ((scr_w - 2 * pad) / cell_width).max(1) as u16;
    let term_rows = ((scr_h - 2 * pad) / cell_height).max(1) as u16;

    let mut terminal = Terminal::new(term_cols, term_rows, 1000)?;

    let (pty_fd, child) =
        pty_spawn(term_cols, term_rows).map_err(|e| format!("forkpty failed: {e}"))?;

    let mut key_encoder = KeyEncoder::new()?;
    let mut key_event = KeyEvent::new()?;
    let mut mouse_encoder = MouseEncoder::new()?;
    let mut mouse_event = MouseEvent::new()?;
    let mut render_state = RenderState::new()?;
    let mut row_iter = RenderStateRowIterator::new()?;
    let mut row_cells = RenderStateRowCells::new()?;

    let mut prev_width = scr_w;
    let mut prev_height = scr_h;
    let mut prev_focused = rl.is_window_focused();
    let mut scrollbar_dragging = false;
    let mut child_exited = false;
    let mut child_reaped = false;
    let mut child_exit_status: i32 = -1;

    while !rl.window_should_close() {
        // --- Resize ----------------------------------------------------------
        if rl.is_window_resized() {
            let w = rl.get_screen_width();
            let h = rl.get_screen_height();
            if w != prev_width || h != prev_height {
                let cols = ((w - 2 * pad) / cell_width).max(1) as u16;
                let rows = ((h - 2 * pad) / cell_height).max(1) as u16;
                let _ = terminal.resize(cols, rows, cell_width as u32, cell_height as u32);

                // Notify the pty of the new window size so the shell
                // and child programs can reflow their output.
                let new_ws = libc::winsize {
                    ws_row: rows,
                    ws_col: cols,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                };
                unsafe { libc::ioctl(pty_fd.as_raw_fd(), libc::TIOCSWINSZ, &new_ws) };

                prev_width = w;
                prev_height = h;
            }
        }

        // --- Focus tracking --------------------------------------------------
        let focused = rl.is_window_focused();
        if focused != prev_focused {
            if !child_exited {
                // Send focus gained/lost if the terminal has focus reporting
                // enabled (DEC private mode 1004).
                let focus_mode: ffi::GhosttyMode = (1 << 15) | 1004;
                if let Ok(true) = terminal.mode_get(focus_mode) {
                    let focus_event = if focused {
                        ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_GAINED
                    } else {
                        ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_LOST
                    };
                    let mut focus_buf = [0u8; 8];
                    if let Ok(written) = ghostty::focus_encode(focus_event, &mut focus_buf) {
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
            let mut wstatus: libc::c_int = 0;
            let wp = unsafe { libc::waitpid(child, &mut wstatus, libc::WNOHANG) };
            if wp > 0 {
                child_reaped = true;
                if libc::WIFEXITED(wstatus) {
                    child_exit_status = libc::WEXITSTATUS(wstatus);
                } else if libc::WIFSIGNALED(wstatus) {
                    child_exit_status = 128 + libc::WTERMSIG(wstatus);
                }
            }
        }

        // --- Scrollbar -------------------------------------------------------
        let scrollbar_consumed = handle_scrollbar(
            &rl,
            &mut terminal,
            &mut render_state,
            &mut scrollbar_dragging,
        );

        // --- Input -----------------------------------------------------------
        if !child_exited {
            handle_input(&rl, &pty_fd, &mut key_encoder, &mut key_event, &terminal);
            if !scrollbar_consumed {
                handle_mouse(
                    &rl,
                    &pty_fd,
                    &mut mouse_encoder,
                    &mut mouse_event,
                    &mut terminal,
                    cell_width,
                    cell_height,
                    pad,
                );
            }
        }

        // --- Update render state ---------------------------------------------
        let _ = render_state.update(&mut terminal);

        // --- Draw ------------------------------------------------------------
        let bg_colors = render_state
            .colors_get()
            .unwrap_or_else(|_| ffi::GhosttyRenderStateColors::default());
        let win_bg = Color::new(
            bg_colors.background.r,
            bg_colors.background.g,
            bg_colors.background.b,
            255,
        );

        let scrollbar = terminal.scrollbar().ok();

        let mut d = rl.begin_drawing(&thread);
        d.clear_background(win_bg);

        render_terminal(
            &mut d,
            &mut render_state,
            &mut row_iter,
            &mut row_cells,
            &mono_font,
            cell_width,
            cell_height,
            font_size,
            scrollbar.as_ref(),
        );

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

    // --- Cleanup -------------------------------------------------------------
    drop(pty_fd);
    if !child_reaped {
        if !child_exited {
            unsafe { libc::kill(child, libc::SIGHUP) };
        }
        unsafe { libc::waitpid(child, std::ptr::null_mut(), 0) };
    }

    Ok(())
}
