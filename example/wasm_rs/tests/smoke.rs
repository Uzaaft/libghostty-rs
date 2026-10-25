//! Integration checks, separate from the browser example.

use std::{
    cell::RefCell,
    ffi::CStr,
    mem::{align_of, offset_of, size_of},
};

use libghostty_vt::{
    RenderState, Terminal,
    alloc::Allocator,
    ffi,
    fmt::{Formatter, FormatterOptions},
};

/// Exercise the same public Rust API used on native targets.
/// A failed assertion traps, which makes the host runner fail as well.
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    // Compare target-dependent layouts against the linked Zig library, not
    // the host on which bindgen happened to run.
    // SAFETY: Upstream returns a process-lifetime NUL-terminated manifest.
    let manifest = unsafe { CStr::from_ptr(ffi::ghostty_type_json()) };
    let manifest: serde_json::Value = serde_json::from_slice(manifest.to_bytes()).unwrap();
    assert_eq!(manifest["abi"]["pointer_size"], size_of::<usize>());
    macro_rules! layout {
        ($ty:ident, $($field:ident),+ $(,)?) => {
            let ty = &manifest["types"][concat!("Ghostty", stringify!($ty))];
            assert_eq!(ty["size"], size_of::<ffi::$ty>());
            assert_eq!(ty["align"], align_of::<ffi::$ty>());
            $(assert_eq!(ty["fields"][stringify!($field)]["offset"], offset_of!(ffi::$ty, $field));)+
        };
    }
    layout!(String, ptr, len);
    layout!(Allocator, ctx, vtable);
    layout!(AllocatorVtable, alloc, resize, remap, free);
    layout!(FormatterTerminalOptions, size, emit, unwrap, trim, extra);

    // Keep Rust-owned memory alive while Zig grows and uses its own heap.
    let guard = vec![0x5au8; 256 * 1024];
    // Exercise both Zig's default heap and callbacks into Rust's allocator.
    for custom_allocator in [false, true] {
        let response = RefCell::new(Vec::new());
        let mut terminal = if custom_allocator {
            Terminal::new_with_alloc(&Allocator::GLOBAL, 13, 7)
        } else {
            Terminal::new(13, 7)
        }
        .unwrap();
        terminal
            .on_pty_write(|_, bytes: &[u8]| response.borrow_mut().extend_from_slice(bytes))
            .unwrap();
        terminal.vt_write(b"ab\x1b[31mC\x1b[0m\r\nxy\x1b[6n");
        assert_eq!(*response.borrow(), b"\x1b[2;3R");
        assert_eq!(terminal.cursor_x().unwrap(), 2);
        assert_eq!(terminal.cursor_y().unwrap(), 1);
        let mut state = RenderState::new().unwrap();
        let snapshot = state.update(&terminal).unwrap();
        assert_eq!(snapshot.cols().unwrap(), 13);
        assert_eq!(snapshot.rows().unwrap(), 7);
        let mut formatter =
            Formatter::new(&terminal, FormatterOptions::new().with_trim(true)).unwrap();
        let mut output = [0; 64];
        let len = formatter.format_buf(&mut output).unwrap();
        assert_eq!(&output[..len], b"abC\nxy");
    }

    #[cfg(target_family = "wasm")]
    // SAFETY: Each allocation is used within its bounds and released with
    // its matching destructor. Taking an empty slot does not own a handle.
    unsafe {
        assert!(ffi::ghostty_wasm_alloc(0).is_null());
        let ptr = ffi::ghostty_wasm_alloc(37);
        assert!(!ptr.is_null());
        ptr.cast::<u8>().write_bytes(0xa5, 37);
        assert_eq!(*ptr.cast::<u8>().add(36), 0xa5);
        ffi::ghostty_wasm_free(ptr, 37);
        let slot = ffi::ghostty_wasm_alloc_opaque();
        assert!(!slot.is_null());
        assert!(ffi::ghostty_wasm_take_opaque(slot).is_null());
        assert_eq!(
            ffi::ghostty_terminal_new(std::ptr::null(), slot.cast(), 5, 3),
            ffi::Result::SUCCESS
        );
        let terminal = ffi::ghostty_wasm_take_opaque(slot);
        assert!(!terminal.is_null());
        assert!((*slot).is_null());
        ffi::ghostty_terminal_free(terminal.cast());
        ffi::ghostty_wasm_free_opaque(slot);
    }
    assert!(guard.iter().all(|&byte| byte == 0x5a));
}

#[test]
fn native_api_and_abi() {
    run();
}
