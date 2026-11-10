//! Respond to what programs ask of the terminal, such as the bell,
//! title changes, and queries.
//!
//! An effect is a callback the terminal calls when a program asks for
//! something the terminal can't do by itself, such as ringing the bell
//! or changing the window title.
//!
//! You need these as soon as a real program runs in the terminal. Until
//! you register one, the terminal ignores that request, and a program that
//! asks a question gets no answer. If you only feed bytes in to read the
//! result back, you can skip this module.
//!
//! # Example
//! ```rust
//! # use libghostty_vt::Terminal;
//! # let mut terminal = Terminal::new(80, 24).unwrap();
//! terminal.on_bell(|_term| println!("Bell rang!"));
//! ```
use std::{marker::PhantomData, num::NonZeroU16};

use crate::{
    error::{Error, Result},
    ffi, osc,
    report::{ColorScheme, SizeReportSize},
};

// Only referenced in docs.
#[allow(unused)]
use crate::{Terminal, mode::Mode};

/// You might be wondering just what the heck this is.
///
/// Truth to be told, you don't need to understand how it works
/// in order to use it. It does a bunch of voodoo behind the scenes
/// that make sure all the invariants of the C API are upheld, while
/// providing a convenient API for Rust users.
///
/// Each handler is defined in this following format:
/// ```ignore
/// pub fn on_foobar(
///     &mut self,
///     // The corresponding GhosttyTerminalOption
///     tag = FOOBAR,
///
///     // The name of the original function type in C,
///     // along with the extra C parameters and the expected C return type
///     from = TerminalFoobarFn(foo: *const u8, bar: usize) -> bool,
///
///     // The name of mapped Rust function type,
///     // along with the Rust parameters and return type.
///     //
///     // `<'t>` is used to tie the return value to the lifetime of the
///     // terminal. The name is arbitrary - any lifetime marker will do.
///     to = <'t>FoobarFn(&'t [u8]) -> bool,
/// ) |term, func| {
///     // `term` is the terminal and `func` is the Rust callback.
///     // Both names are arbitrary.
///
///     // Convert the raw parameters into Rust types.
///     // This is just to illustrate how.
///     let slice = unsafe { std::slice::from_raw_parts(foo, bar) };
///
///     // Call into user logic and return.
///     func(&terminal, slice)
/// }
/// ```
macro_rules! handlers {
    {
        $(
            $(#[$fmeta:meta])*
            $vis:vis fn $name:ident(
                &mut self,
                tag = $tag:ident,
                from = $rawfnty:ident( $($rfname:ident: $rfty:ty),*$(,)? ) $(-> $rawrty:ty)?,
                $(#[$tmeta:meta])*
                to = $(<$lf:lifetime>)? $fnty:ident( $($fty:ty),*$(,)? ) $(-> $rty:ty)?,
            ) |$t:ident, $func:ident| $block:block
        )*
    } => {
        /// Methods for registering [effect handlers](#effects).
        impl<'alloc, 'cb> $crate::terminal::Terminal<'alloc, 'cb> {$(
            $(#[$fmeta])*
            ///
            /// See [#Effects](Terminal#effects) for more details.
            $vis fn $name(&mut self, f: impl $fnty<'alloc, 'cb>) -> $crate::error::Result<&mut Self> {
                unsafe extern "C" fn callback(
                    t: $crate::ffi::Terminal,
                    ud: *mut std::ffi::c_void,
                    $($rfname: $rfty),*
                ) $(-> $rawrty)? {
                    // SAFETY: USERDATA is the owning raw VTable pointer
                    // returned by Box::into_raw, unchanged by Terminal moves,
                    // before the callback is registered. ghostty invokes
                    // callbacks synchronously from vt_write, vt_write_until,
                    // reset, resize and paste (which writes to the pty). All
                    // five take `&mut self`, so the VTable outlives this call
                    // and nothing outside the callbacks touches it meanwhile.
                    // Callbacks only get a `&Terminal`, so they can't reach
                    // any of those entry points.
                    //
                    // Dispatch nests in one place: `ClipboardRead::reply`
                    // writes the answer to the pty right away, so the pty
                    // write callback runs inside the clipboard read callback.
                    // That is still sound, since neither trampoline keeps a
                    // reference into the VTable across the user's closure:
                    // `vtable` is only used to pick out the closure, which
                    // lives in its own box, and different callbacks use
                    // different boxes.
                    let vtable = unsafe { &mut *ud.cast::<VTable<'_, '_>>() };

                    let obj = $crate::alloc::Object::new(t).expect("received null terminal ptr in callback - this is a bug!");
                    // Build a temporary borrowed Terminal view for the callback
                    // without taking ownership of the underlying ghostty terminal.
                    //
                    // The view must carry the real terminal's id, not a
                    // placeholder: checks such as `PlacementIteration::rect`
                    // compare ids to tell terminals apart, and a shared
                    // placeholder would make views of different terminals
                    // look like the same one.
                    let term = ::core::mem::ManuallyDrop::new($crate::terminal::Terminal::<'_, '_> {
                        inner: obj,
                        vtable: ud.cast(),
                        id: vtable.id,
                    });
                    let $t: &$crate::terminal::Terminal = &term;
                    let $func = vtable.$name.as_deref_mut()
                        .expect("no handler set but callback is still called - this is a bug!");
                    $block
                }

                // SAFETY: Registration has exclusive access to the terminal;
                // no callback is active and the VTable allocation is live.
                unsafe { (*self.vtable).$name = Some(::std::boxed::Box::new(f)); }

                // USERDATA is a raw pointer option: pass the heap allocation
                // itself, not the address of the Box smart pointer field stored
                // inline in Terminal.
                //
                // Reuse the owning pointer, not a pointer derived from a
                // temporary mutable reference into the VTable.
                let userdata = self.vtable.cast::<::std::ffi::c_void>().cast_const();
                self.set_ptr($crate::ffi::TerminalOption::USERDATA, userdata)?;

                // The callback must be coerced into a function *pointer*
                // and not a function *item* (which is a ZST whose address is meaningless).
                // :)
                // Type-check against the generated C callback alias so ABI changes
                // cannot silently pass through the type-erased option setter.
                let _: $crate::ffi::$rawfnty = Some(callback);

                let callback_ptr: unsafe extern "C" fn(
                    $crate::ffi::Terminal,
                    *mut ::std::ffi::c_void,
                    $($rfty),*
                ) $(-> $rawrty)? = callback;

                let result = unsafe {
                    $crate::ffi::ghostty_terminal_set(
                        self.inner.as_raw(),
                        $crate::ffi::TerminalOption::$tag,
                        callback_ptr as *const ::std::ffi::c_void
                    )
                };
                $crate::error::from_result(result)?;
                Ok(self)
            }
        )*}
        $(
            #[doc = concat!(
                "[Effect](Terminal#effects) callback type for [`Terminal::",
                stringify!($name),
                "`](Terminal::",
                stringify!($name),
                ").\n"
            )]
            $(#[$tmeta])*
            pub trait $fnty<'alloc, 'cb>:
                $(for<$lf>)? FnMut(
                    &$($lf)? $crate::terminal::Terminal<'alloc, 'cb>,
                    $($fty),*
                ) $(-> $rty)? + 'cb {}

            impl<'alloc, 'cb, F> $fnty<'alloc, 'cb> for F
            where
                F: $(for<$lf>)? FnMut(
                    &$($lf)? $crate::terminal::Terminal<'alloc, 'cb>,
                    $($fty),*
                ) $(-> $rty)? + 'cb
            {}
        )*

        pub(crate) struct VTable<'alloc, 'cb> {
            // The `Terminal::id` of the terminal owning this VTable, for the
            // callback views. See the trampoline above.
            id: u64,
            $($name: Option<::std::boxed::Box<dyn $fnty<'alloc, 'cb>>>),*
        }

        impl ::core::fmt::Debug for VTable<'_, '_> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                f.write_str("VTable {..}")
            }
        }

        impl VTable<'_, '_> {
            pub(crate) fn new(id: u64) -> Self {
                Self {
                    id,
                    $($name: None),*
                }
            }
        }
    };
}

handlers! {
    /// Call the given function when the terminal needs to write data back
    /// to the pty (e.g. in response to a DECRQM query, device status report,
    /// or VT-driven mode 2048 enable).
    pub fn on_pty_write(
        &mut self,
        tag = WRITE_PTY,
        from = TerminalWritePtyFn(ptr: *const u8, len: usize),
        to = <'t>PtyWriteFn(&'t [u8]),
    ) |term, func| {
        // SAFETY: We trust libghostty to return valid memory given we
        // uphold all lifetime invariants (e.g. no `vt_write` calls
        // during this callback, which is guaranteed via the mutable reference).
        let data = unsafe { std::slice::from_raw_parts(ptr, len) };
        func(term, data);
    }

    /// Call the given function when the terminal receives
    /// a BEL character (0x07).
    pub fn on_bell(
        &mut self,
        tag = BELL,
        from = TerminalBellFn(),
        to = BellFn(),
    ) |term, func| {
        func(term);
    }

    /// Call the given function when the running program asks the terminal to
    /// stop updating the screen, and again when it lets the screen update
    /// again. We call the time in between a "render hold".
    ///
    /// Programs use a hold to avoid flicker. A full-screen program usually
    /// redraws in several steps: clear, draw the text, move the cursor. If
    /// the screen is drawn halfway through, the user sees a broken frame. To
    /// prevent that, the program starts a hold, draws everything, and then
    /// releases the hold. The screen should keep showing the last finished
    /// frame the whole time and then switch to the new one all at once.
    ///
    /// Today the only way a program can start a hold is synchronized output
    /// ([`Mode::SYNC_OUTPUT`], DEC private mode 2026). The callback is named
    /// for what the embedder should do rather than for that mode so that
    /// other sources of holds can be added later.
    ///
    /// # When it is called
    ///
    /// With `held` set to `true` when the program sets mode 2026.
    ///
    /// With `held` set to `false` when the hold ends, which happens when:
    ///
    /// - the program resets mode 2026
    /// - the terminal is fully reset, by the program (RIS) or by
    ///   [`Terminal::reset`]
    /// - the terminal is resized with [`Terminal::resize`]
    ///
    /// The two calls always come in pairs. Setting the mode while a hold is
    /// already active does nothing, and neither does resetting it when there
    /// is no hold. Changing the mode yourself with [`Terminal::set_mode`]
    /// never invokes the callback.
    ///
    /// # What to do
    ///
    /// When a hold begins, the terminal contains exactly the frame the
    /// program wants left on screen. Nothing after the start of the hold has
    /// been processed yet, even if more bytes follow in the same
    /// [`Terminal::vt_write`] call. Capture that frame by calling
    /// [`RenderState::update`](crate::RenderState::update) from within the
    /// callback, then stop updating the render state until the hold ends. You
    /// can keep drawing the render state in the meantime. It won't change.
    ///
    /// <div class="warning">
    ///
    /// The callback runs inside an `extern "C"` function, so a panic in it
    /// aborts the process. If the callback updates the render state, don't
    /// keep the render state borrowed (e.g. through a
    /// [`Snapshot`](crate::render::Snapshot) or a
    /// [`RefMut`](std::cell::RefMut)) across [`Terminal::vt_write`],
    /// [`Terminal::reset`] or [`Terminal::resize`], since any of them can
    /// invoke the callback. Use a non-panicking borrow such as
    /// [`RefCell::try_borrow_mut`](std::cell::RefCell::try_borrow_mut)
    /// inside the callback.
    ///
    /// </div>
    ///
    /// ```rust
    /// use std::cell::{Cell, RefCell};
    /// use std::time::{Duration, Instant};
    /// use libghostty_vt::{RenderState, Terminal, mode::Mode};
    ///
    /// struct Renderer {
    ///     render_state: RefCell<RenderState<'static>>,
    ///     held: Cell<bool>,
    ///     hold_started: Cell<Instant>,
    /// }
    ///
    /// fn draw(r: &Renderer, terminal: &mut Terminal<'static, '_>) -> libghostty_vt::error::Result<()> {
    ///     // Give up on a program that holds the screen for too long.
    ///     if r.held.get() && r.hold_started.get().elapsed() >= Duration::from_secs(1) {
    ///         terminal.set_mode(Mode::SYNC_OUTPUT, false)?;
    ///         r.held.set(false);
    ///     }
    ///
    ///     // During a hold, skip the update and draw the captured frame.
    ///     if !r.held.get() {
    ///         r.render_state.borrow_mut().update(terminal)?;
    ///     }
    ///     // draw_render_state(&r.render_state);
    ///     Ok(())
    /// }
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let renderer = Renderer {
    ///     render_state: RefCell::new(RenderState::new()?),
    ///     held: Cell::new(false),
    ///     hold_started: Cell::new(Instant::now()),
    /// };
    ///
    /// let mut terminal = Terminal::new(80, 24)?;
    /// terminal.on_render_hold(|term, held| {
    ///     if held {
    ///         // Capture the frame the program wants left on screen. Don't
    ///         // panic if that fails, e.g. because the render state is
    ///         // borrowed elsewhere; keep drawing the previous frame instead.
    ///         let _ = renderer
    ///             .render_state
    ///             .try_borrow_mut()
    ///             .map(|mut state| state.update(term).map(|_| ()));
    ///         renderer.hold_started.set(Instant::now());
    ///     }
    ///     renderer.held.set(held);
    /// })?;
    ///
    /// terminal.vt_write(b"\x1b[?2026h");
    /// assert!(renderer.held.get());
    /// draw(&renderer, &mut terminal)?;
    ///
    /// terminal.vt_write(b"\x1b[?2026l");
    /// assert!(!renderer.held.get());
    /// draw(&renderer, &mut terminal)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Timeouts
    ///
    /// The terminal has no clock, so it never ends a hold on its own. A
    /// program that crashes or forgets to release its hold would freeze the
    /// screen forever, so you need a timeout like the one above. One second
    /// is a common choice. When it expires, reset the mode yourself and go
    /// back to updating normally. Because setting the mode again during a
    /// hold does nothing, a program can't keep pushing your deadline back.
    ///
    /// # Why a callback
    ///
    /// You could instead check [`Mode::SYNC_OUTPUT`] before each draw and
    /// skip the update when it is set. That is simpler, but it has two
    /// problems. First, the frame left on screen is whatever you happened to
    /// draw last, which can be older than what the program intended or even
    /// a half-drawn frame. Second, if the program releases a hold and starts
    /// the next one between two of your draws, you never see the mode turn
    /// off and the finished frame in between is lost. A program that draws
    /// continuously can then appear frozen. Capturing the frame when each
    /// hold begins avoids both.
    ///
    /// # Other notes
    ///
    /// You are free to ignore a hold whenever showing live content matters
    /// more, such as when the user scrolls or starts a selection.
    ///
    /// Like every callback, this runs on the thread that called
    /// [`Terminal::vt_write`], [`Terminal::reset`] or [`Terminal::resize`].
    /// Since neither a terminal nor a render state can be sent to another
    /// thread, the update in the callback needs no locking.
    pub fn on_render_hold(
        &mut self,
        tag = RENDER_HOLD,
        from = TerminalRenderHoldFn(held: bool),
        to = RenderHoldFn(bool),
    ) |term, func| {
        func(term, held);
    }

    /// Call the given function when the terminal receives
    /// an ENQ character (0x05).
    pub fn on_enquiry(
        &mut self,
        tag = ENQUIRY,
        from = TerminalEnquiryFn() -> ffi::String,
        to = <'t>EnquiryFn() -> Option<&'t str>,
    ) |term, func| {
        func(term).unwrap_or("").into()
    }

    /// Call the given function when the terminal receives an XTVERSION
    /// query (CSI > q), and respond with the resulting version string
    /// (e.g. "myterm 1.0").
    pub fn on_xtversion(
        &mut self,
        tag = XTVERSION,
        from = TerminalXtversionFn() -> ffi::String,
        to = <'t>XtversionFn() -> Option<&'t str>,
    ) |term, func| {
        func(term).unwrap_or("").into()
    }

    /// Call the given function when the terminal title changes
    /// via escape sequences (e.g. OSC 0 or OSC 2).
    ///
    /// The new title can be queried from the terminal after
    /// the callback returns.
    pub fn on_title_changed(
        &mut self,
        tag = TITLE_CHANGED,
        from = TerminalTitleChangedFn(),
        to = TitleChangedFn(),
    ) |term, func| {
        func(term);
    }

    /// Call the given function when the terminal current working directory
    /// changes via escape sequences (e.g. OSC 7, OSC 9, or OSC 1337).
    ///
    /// The new working directory can be queried from the terminal after
    /// the callback returns.
    pub fn on_pwd_changed(
        &mut self,
        tag = PWD_CHANGED,
        from = TerminalPwdChangedFn(),
        to = PwdChangedFn(),
    ) |term, func| {
        func(term);
    }

    /// Call the given function in response to XTWINOPS size queries
    /// (CSI 14/16/18 t) and when VT input enables in-band size reports (mode
    /// 2048). Return the current terminal geometry, or `None` to suppress the
    /// report.
    pub fn on_size(
        &mut self,
        tag = SIZE,
        from = TerminalSizeFn(out: *mut ffi::SizeReportSize) -> bool,
        to = SizeFn() -> Option<SizeReportSize>,
    ) |term, func| {
        if let Some(size) = func(term) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size };
            true
        } else {
            false
        }
    }

    /// Call the given function in response to a color scheme
    /// device status report query (CSI ? 996 n).
    ///
    /// Return `Some` to report the current color scheme,
    /// or return `None` to silently ignore.
    pub fn on_color_scheme(
        &mut self,
        tag = COLOR_SCHEME,
        from = TerminalColorSchemeFn(out: *mut ffi::ColorScheme::Type) -> bool,
        to = ColorSchemeFn() -> Option<ColorScheme>,
    ) |term, func| {
        if let Some(size) = func(term) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size.into() };
            true
        } else {
            false
        }
    }

    /// Call the given function in response to a device attributes query
    /// (CSI c, CSI > c, or CSI = c).
    ///
    /// Return `Some` with the response data,
    /// or return `None` to silently ignore.
    pub fn on_device_attributes(
        &mut self,
        tag = DEVICE_ATTRIBUTES,
        from = TerminalDeviceAttributesFn(out: *mut ffi::DeviceAttributes) -> bool,
        to = DeviceAttributesFn() -> Option<DeviceAttributes>,
    ) |term, func| {
        if let Some(size) = func(term) {
            // SAFETY: Out pointer is assumed to be valid.
            unsafe { *out = size.into() };
            true
        } else {
            false
        }
    }

    /// Call the given function when the running program performs a clipboard write.
    ///
    /// Protocol details such as OSC 52 selectors, base64 encoding, multipart
    /// chunks, aliases, and terminators are normalized before this callback is
    /// invoked. OSC 52, iTerm2 OSC 1337 Copy, and Kitty clipboard (OSC 5522)
    /// writes therefore use the same callback shape. Without this callback,
    /// clipboard writes are ignored and Kitty clipboard writes are refused
    /// with ENOSYS.
    ///
    /// The embedder may ask for permission to write or perform the write
    /// async, but the callback itself is synchronous and
    /// [`ClipboardWrite::reply`] must be called before it returns. While this
    /// callback is active the VT stream is paused. Returning without a reply
    /// denies the write.
    ///
    /// The request may carry an optional program name requesting the write
    /// and the state of prior permission granted. If
    /// [`ClipboardWrite::can_remember`] is set the reply may set `remember`,
    /// and future requests from this same program will be
    /// [granted](ClipboardWrite::granted) so the embedder can skip permission
    /// requests.
    ///
    /// Clipboard read requests (OSC 52 `?` and OSC 5522 reads) are delivered
    /// to [`Self::on_clipboard_read`] instead.
    pub fn on_clipboard_write(
        &mut self,
        tag = CLIPBOARD_WRITE,
        from = TerminalClipboardWriteFn(
            write: *const ffi::ClipboardWrite
        ),
        to = <'t>ClipboardWriteFn(ClipboardWrite<'t>),
    ) |term, func| {
        // SAFETY: The request is only borrowed for the callback duration,
        // which `ClipboardWrite`'s lifetime enforces.
        func(term, unsafe { ClipboardWrite::from_raw(write) });
    }

    /// Call the given function when the running program requests clipboard
    /// contents via OSC 52 with a `?` payload or a Kitty clipboard (OSC 5522)
    /// read.
    ///
    /// Answering lets the program read the user's clipboard, so the embedder
    /// is expected to mediate consent. Because the read is synchronous, an
    /// embedder that needs to ask the user must block (for example by running
    /// a modal prompt) until it has an answer; the VT stream waits until the
    /// callback returns.
    ///
    /// Answer by calling [`ClipboardRead::reply`] before returning. See
    /// [`ClipboardRead`] for the full contract.
    ///
    /// OSC 5522 requests carry the program's MIME list, name, and password
    /// grant state; a reply that sets `remember` records a session grant so
    /// later requests with the same password arrive with
    /// [`granted`](ClipboardRead::granted) set. Kitty itself serves a request
    /// for only the targets listing ([`list`](ClipboardRead::list) with no
    /// [`mimes`](ClipboardRead::mimes)) without prompting, and embedders are
    /// expected to do the same; the terminal never consults grants for such
    /// requests (`granted` is false and one-time passwords are not consumed).
    ///
    /// Without this callback, OSC 52 read requests are ignored and OSC 5522
    /// reads are refused with EPERM.
    ///
    /// Installing this callback also enables Kitty paste events (mode 5522):
    /// [`Terminal::paste`] sends the program an event instead of the text, and
    /// the program's follow-up read arrives here with `granted` set since the
    /// user already pasted.
    pub fn on_clipboard_read(
        &mut self,
        tag = CLIPBOARD_READ,
        from = TerminalClipboardReadFn(read: *const ffi::ClipboardRead),
        to = <'t>ClipboardReadFn(ClipboardRead<'t>),
    ) |term, func| {
        // SAFETY: The request is only borrowed for the callback duration,
        // which `ClipboardRead`'s lifetime enforces.
        func(term, unsafe { ClipboardRead::from_raw(read) });
    }

    /// Callback invoked when the running program requests a desktop
    /// notification via OSC 9 or OSC 777.
    pub fn on_desktop_notification(
        &mut self,
        tag = DESKTOP_NOTIFICATION,
        from = TerminalDesktopNotificationFn(
            notif: *const ffi::TerminalDesktopNotification
        ),
        to = <'t>DesktopNotificationFn(DesktopNotification<'t>),
    ) |term, func| {
        func(term, unsafe { DesktopNotification::from_raw(notif) });
    }

    /// Call the given function when the running program reports progress
    /// via OSC 9;4.
    pub fn on_progress_report(
        &mut self,
        tag = PROGRESS_REPORT,
        from = TerminalProgressReportFn(
            progress: *const ffi::TerminalProgressReport
        ),
        to = <'t>ProgressReportFn(ProgressReport<'t>),
    ) |term, func| {
        func(term, unsafe { ProgressReport::from_raw(progress) });
    }

    /// Call the given function when the running program sends a program
    /// status report via OSC 7501.
    ///
    /// Called synchronously each time the running program sends a valid
    /// report. See [`ProgramStatus`] for what a report contains.
    ///
    /// The terminal doesn't store reports, so your application keeps them.
    /// To follow the specification, keep one record per id. A report with an
    /// empty id is about the root record, the program itself. The records
    /// follow these rules:
    ///
    /// - A report replaces its record completely. A value the report leaves
    ///   out is gone from the record afterwards. It doesn't keep its old
    ///   value.
    /// - A [`ProgramStatusState::Clear`] report removes the record with its
    ///   id and every record beneath it, so clearing `"build"` also removes
    ///   `"build/test"`. A clear report with an empty id removes every
    ///   record.
    /// - A record without an app takes it from its nearest ancestor that has
    ///   one. If the root record has app `"deploy"` and the record
    ///   `"us-east"` has none, show `"us-east"` with app `"deploy"` too.
    /// - When a new shell prompt starts ([`SemanticPromptKind::PromptStart`]
    ///   from the [`Terminal::on_semantic_prompt`] callback) or the program
    ///   running in the terminal exits, remove `working` and
    ///   `blocked` records. You may remove `idle` records too. Keep `done`
    ///   and `error` records until the user has seen them, for example until

    ///   they next focus the terminal.
    /// - Keep at most 256 records, and allow at least 64. When a new record
    ///   would go over your limit, remove the one that was updated longest
    ///   ago.
    ///
    /// A full reset (RIS, `ESC c`) removes every record. When that happens,
    /// the terminal calls this with a [`ProgramStatusState::Clear`] report
    /// and an empty id, and then calls the [`Terminal::on_reset`] callback.
    /// This only happens when `ESC c` is processed by [`Terminal::vt_write`]
    /// or [`Terminal::vt_write_until`]. [`Terminal::reset`] doesn't
    /// call this, so clear your records yourself after calling it.
    ///
    /// [`title`](ProgramStatus::title) and
    /// [`message`](ProgramStatus::message) are already decoded and contain
    /// no control characters, but they are still untrusted text from the
    /// program. Don't treat them as markup. If you show them outside the
    /// terminal, such as in a tab or a notification, remove invisible
    /// formatting characters like text direction overrides, and say which
    /// terminal the text came from so a program can't pretend to be one
    /// running elsewhere.
    ///
    /// Example, where `records` stands in for your application's own
    /// storage:
    ///
    /// ```rust
    /// use std::cell::RefCell;
    /// use std::collections::HashMap;
    /// use libghostty_vt::{Terminal, effect::ProgramStatusState};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // One record per id. This one keeps only the state and message.
    /// let records = RefCell::new(HashMap::<Vec<u8>, (ProgramStatusState, Vec<u8>)>::new());
    /// # let pty = RefCell::new(Vec::new());
    ///
    /// let mut terminal = Terminal::new(80, 24)?;
    /// // Set a pty write callback too, so programs that check for support
    /// // get a reply.
    /// terminal
    ///     .on_pty_write(|_term, bytes| {
    ///         // Write `bytes` back to the pty.
    /// #       pty.borrow_mut().extend_from_slice(bytes);
    ///     })?
    ///     .on_program_status(|_term, report| {
    ///         // A panic in the callback aborts the process, so ignore a
    ///         // state these bindings don't know instead of unwrapping.
    ///         let Ok(state) = report.state() else { return };
    ///         let mut records = records.borrow_mut();
    ///
    ///         if state == ProgramStatusState::Clear {
    ///             // Remove this record and every record beneath it. An empty
    ///             // id removes every record.
    ///             let id = report.id();
    ///             records.retain(|key, _| {
    ///                 let beneath = key
    ///                     .strip_prefix(id)
    ///                     .is_some_and(|rest| rest.is_empty() || rest.starts_with(b"/"));
    ///                 !(id.is_empty() || beneath)
    ///             });
    ///             return;
    ///         }
    ///
    ///         // Replace the whole record. The strings are only valid during
    ///         // this call, so copy them.
    ///         records.insert(report.id().to_vec(), (state, report.message().to_vec()));
    ///     })?;
    /// # let ids = || {
    /// #     let mut ids: Vec<_> = records.borrow().keys().cloned().collect();
    /// #     ids.sort();
    /// #     ids
    /// # };
    ///
    /// // A root record and a few beneath it. "QnVpbGRpbmc=" is "Building".
    /// terminal.vt_write(b"\x1b]7501;state=working:app=cargo:msg=QnVpbGRpbmc=\x1b\\");
    /// terminal.vt_write(b"\x1b]7501;state=working:id=build\x1b\\");
    /// terminal.vt_write(b"\x1b]7501;state=blocked:kind=question:id=build/test\x1b\\");
    /// terminal.vt_write(b"\x1b]7501;state=done:id=buildx\x1b\\");
    /// # assert_eq!(ids(), [&b""[..], b"build", b"build/test", b"buildx"]);
    /// # assert_eq!(
    /// #     records.borrow()[&b""[..]],
    /// #     (ProgramStatusState::Working, b"Building".to_vec()),
    /// # );
    ///
    /// // A report replaces its record completely, so the message is gone.
    /// terminal.vt_write(b"\x1b]7501;state=done\x1b\\");
    /// # assert_eq!(records.borrow()[&b""[..]], (ProgramStatusState::Done, Vec::new()));
    ///
    /// // Clearing "build" also removes "build/test", but not "buildx".
    /// terminal.vt_write(b"\x1b]7501;state=clear:id=build\x1b\\");
    /// # assert_eq!(ids(), [&b""[..], b"buildx"]);
    ///
    /// // A program checking for support gets a reply through the pty.
    /// terminal.vt_write(b"\x1b]7501;?\x1b\\");
    /// # assert!(pty.borrow().starts_with(b"\x1b]7501;?:states="));
    ///
    /// // A full reset removes every record.
    /// terminal.vt_write(b"\x1bc");
    /// # assert!(records.borrow().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Programs may ask whether the terminal supports the protocol by
    /// sending `OSC 7501 ; ?`. While this callback is set, the terminal
    /// answers through [`Terminal::on_pty_write`] with `?` followed by the
    /// states and kinds it accepts:
    ///
    /// ```text
    /// program:  ESC ] 7501 ; ? ST
    /// terminal: ESC ] 7501 ; ?:states=idle,working,done,blocked,error:kinds=permission,question,auth ST
    /// ```
    ///
    /// While this callback is not set, the query gets no reply, so a program
    /// that asks sees the protocol as unsupported. Asking is optional, so
    /// programs may send reports anyway. Those are dropped. Set a pty write
    /// callback too, or programs never see the reply.
    pub fn on_program_status(
        &mut self,
        tag = PROGRAM_STATUS,
        from = TerminalProgramStatusFn(
            report: *const ffi::TerminalProgramStatus
        ),
        to = <'t>ProgramStatusFn(ProgramStatus<'t>),
    ) |term, func| {
        // SAFETY: The report and its strings are only valid during the
        // callback, which `ProgramStatus`'s lifetime enforces.
        func(term, unsafe { ProgramStatus::from_raw(report) });
    }

    /// Call the given function when the shell reports a step of a command:
    /// a prompt starts, input starts, output starts, or the command ends.
    ///
    /// Each command goes through four steps, in this order: the prompt
    /// starts, input starts, output starts, and the command ends. Then the
    /// next prompt starts. See [`SemanticPromptKind`].
    ///
    /// Shells differ in what they report. Many don't send the command line
    /// or the exit code, and some skip steps, so handle each event on its
    /// own instead of expecting a strict order. A shell may also start the
    /// same prompt more than once, for example when it redraws the prompt
    /// after a resize, so treat a repeated
    /// [`SemanticPromptKind::PromptStart`] as harmless.
    ///
    /// The terminal has already updated its screen when this is called. A
    /// sequence the terminal rejects as malformed is never reported.
    ///
    /// The event and its strings are only valid during the call.
    ///
    /// Example, logging each command's result:
    ///
    /// ```rust
    /// use std::cell::RefCell;
    /// use libghostty_vt::{Terminal, effect::SemanticPromptKind};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let results = RefCell::new(Vec::new());
    ///
    /// let mut terminal = Terminal::new(80, 24)?;
    /// terminal.on_semantic_prompt(|_term, event| {
    ///     // A panic in the callback aborts the process, so ignore a kind
    ///     // these bindings don't know instead of unwrapping.
    ///     if let Ok(SemanticPromptKind::CommandEnd) = event.kind() {
    ///         // Many shells don't report the exit code, so this may be
    ///         // `None` even for a command that ran.
    ///         results.borrow_mut().push(event.exit_code());
    ///     }
    /// })?;
    ///
    /// // The shell reports a command that exited with 1, then one that
    /// // didn't say how it went.
    /// terminal.vt_write(b"\x1b]133;D;1\x07");
    /// terminal.vt_write(b"\x1b]133;D\x07");
    /// assert_eq!(*results.borrow(), [Some(1), None]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_semantic_prompt(
        &mut self,
        tag = SEMANTIC_PROMPT,
        from = TerminalSemanticPromptFn(
            event: *const ffi::TerminalSemanticPrompt
        ),
        to = <'t>SemanticPromptFn(SemanticPrompt<'t>),
    ) |term, func| {
        // SAFETY: The event and its strings are only valid during the
        // callback, which `SemanticPrompt`'s lifetime enforces.
        func(term, unsafe { SemanticPrompt::from_raw(event) });
    }

    /// Call the given function when the running program performs a full
    /// reset (RIS, `ESC c`).
    ///
    /// A full reset clears the screen and scrollback, returns modes to their
    /// defaults, and clears the title and working directory. Use this
    /// callback to reset any state your application keeps about what's
    /// running in the terminal, such as the current command.
    ///
    /// The terminal has already reset itself when this is called. The
    /// [`Terminal::on_title_changed`] and [`Terminal::on_pwd_changed`]
    /// callbacks are not called for the cleared title and working directory,
    /// so update anything you show for them here. A full reset also removes
    /// any progress report and program status records. If you set
    /// [`Terminal::on_progress_report`] or [`Terminal::on_program_status`],
    /// those callbacks are called before this one.
    ///
    /// A soft reset (DECSTR, `CSI ! p`) keeps the screen, title and working
    /// directory, and doesn't call this.
    ///
    /// [`Terminal::reset`] doesn't call this either: only a reset the
    /// running program sends through [`Terminal::vt_write`] or
    /// [`Terminal::vt_write_until`] does.
    pub fn on_reset(
        &mut self,
        tag = RESET,
        from = TerminalResetFn(),
        to = ResetFn(),
    ) |term, func| {
        func(term);
    }

    /// Call the given function when the running program asks to resize the
    /// window, such as with CSI 8 ; rows ; columns t.
    ///
    /// More kinds of requests may be reported in later versions; see
    /// [`ResizeWindow`]. Requests of a kind these bindings don't know are not
    /// passed to the closure. The terminal is not resized by this: the embedder
    /// decides whether to allow the request, and if so resizes its window and
    /// the terminal (with [`Terminal::resize`]). Letting any program resize
    /// the window is a nuisance at best, so consider making this opt-in for
    /// users.
    ///
    /// Without this callback, resize requests are ignored.
    ///
    /// ```rust
    /// use std::{cell::Cell, num::NonZeroU16};
    /// use libghostty_vt::{Terminal, effect::ResizeWindow};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let requested = Cell::new(None);
    ///
    /// let mut terminal = Terminal::new(80, 24)?;
    /// terminal.on_resize_window(|_term, request| match request {
    ///     ResizeWindow::Grid { rows, columns } => requested.set(Some((rows, columns))),
    ///     // Ignore kinds of requests you don't handle.
    ///     _ => {}
    /// })?;
    ///
    /// terminal.vt_write(b"\x1b[8;30;100t");
    /// # assert_eq!(requested.get(), Some((NonZeroU16::new(30), NonZeroU16::new(100))));
    ///
    /// // The terminal itself keeps its size. Apply the request yourself
    /// // (`None` means keep the current size of that dimension).
    /// # assert_eq!((terminal.cols()?, terminal.rows()?), (80, 24));
    /// if let Some((rows, columns)) = requested.take() {
    ///     let rows = rows.map_or(terminal.rows()?, NonZeroU16::get);
    ///     let cols = columns.map_or(terminal.cols()?, NonZeroU16::get);
    ///     terminal.resize(cols, rows, 8, 16)?;
    /// }
    /// # assert_eq!((terminal.cols()?, terminal.rows()?), (100, 30));
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_resize_window(
        &mut self,
        tag = RESIZE_WINDOW,
        from = TerminalResizeWindowFn(request: *const ffi::TerminalResizeWindow),
        to = ResizeWindowFn(ResizeWindow),
    ) |term, func| {
        // SAFETY: We trust libghostty to give us a valid, non-NULL request
        // that lives for the callback duration, with the tag and its
        // matching payload written. For a payload-less kind the union may be
        // left unwritten, which is why `from_raw` checks the tag before
        // reading anything else, instead of us copying the struct out.
        let request = unsafe { ResizeWindow::from_raw(request) };
        // Upstream asks callbacks to ignore tags they don't handle. A tag
        // these bindings don't know has no Rust value, so the closure isn't
        // called for it.
        if let Some(request) = request {
            func(term, request);
        }
    }

    /// Called once for each complete sequence that libghostty-vt does not
    /// implement. Match on [`UnknownSequence`] first, because more kinds of
    /// sequences may be reported in later versions.
    ///
    /// Sequences of a kind these bindings don't know, and OSC sequences with
    /// an [`osc::Terminator`] these bindings don't know, are not passed to the
    /// closure.
    ///
    /// These are not reported:
    ///
    /// - Sequences cut short before their terminator, either cancelled by CAN
    ///   or SUB or aborted by another 8-bit control byte.
    /// - Sequences libghostty-vt implements, even when their contents are
    ///   malformed.
    /// - Supported protocols that the embedder turned off.
    ///
    /// The callback runs during [`Self::vt_write`]. It may write a reply to
    /// the pty, and that reply stays in order with the terminal's own
    /// replies. It must not call [`Self::vt_write`] on the same terminal.
    ///
    /// Nothing is reported until [`Self::set_unknown_max_bytes`] is also set
    /// to a nonzero value. Installing the callback by itself keeps no sequence
    /// data and allocates no sequence-capture buffers.
    ///
    /// For OSC, the content is everything between `ESC ]` and the terminator,
    /// including the number that identifies the sequence. As an example,
    /// suppose your application invents its own OSC 7400 so that programs can
    /// report their status. If a program writes `ESC ] 7400;status=busy BEL`,
    /// the callback receives the content `7400;status=busy` and the
    /// terminator [`osc::Terminator::Bel`]. Match on the number followed by
    /// `;`, so that `7400;` does not also match an unrelated `74000;`
    /// sequence.
    ///
    /// Only numbers libghostty-vt does not recognize are reported. A sequence
    /// that uses a number it does implement, such as OSC 2 for the window
    /// title, is never reported, even when its contents are malformed.
    ///
    /// Use the terminator from the request in your reply, since that is what
    /// the program expects.
    ///
    /// ```rust
    /// use std::cell::RefCell;
    /// use libghostty_vt::{Terminal, effect::UnknownSequence};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let statuses = RefCell::new(Vec::new());
    ///
    /// let mut terminal = Terminal::new(80, 24)?;
    /// terminal
    ///     .on_unknown_sequence(|_term, sequence| {
    ///         let UnknownSequence::Osc { content, truncated, .. } = sequence else {
    ///             return;
    ///         };
    ///         // This protocol needs the whole sequence, so skip cut-off ones.
    ///         if truncated {
    ///             return;
    ///         }
    ///         // Only handle OSC 7400. Everything else is ignored.
    ///         if let Some(status) = content.strip_prefix(b"7400;") {
    ///             // The content is only valid during this call. Copy it if you need it
    ///             // later.
    ///             statuses.borrow_mut().push(status.to_vec());
    ///         }
    ///     })?
    ///     // Keep up to 4 KiB of each unknown sequence and report them.
    ///     .set_unknown_max_bytes(4096)?;
    ///
    /// terminal.vt_write(b"\x1b]7400;status=busy\x07");
    /// assert_eq!(*statuses.borrow(), [b"status=busy".to_vec()]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_unknown_sequence(
        &mut self,
        tag = UNKNOWN_SEQUENCE,
        from = TerminalUnknownSequenceFn(sequence: *const ffi::TerminalUnknownSequence),
        to = <'t>UnknownSequenceFn(UnknownSequence<'t>),
    ) |term, func| {
        // SAFETY: libghostty passes a valid, aligned sequence for the
        // callback duration, with the tag and the union field it names
        // written (`lib.TaggedUnion.cval`). Upstream documents the content
        // as valid only for the callback duration; the higher-ranked `'t` on
        // `UnknownSequenceFn` stops the closure from keeping it.
        if let Some(sequence) = unsafe { UnknownSequence::from_raw(sequence) } {
            func(term, sequence);
        }
    }
}

/// A synchronous request to write clipboard contents.
///
/// The request, contents array, MIME strings, and data strings are all
/// borrowed and valid only for the callback duration.
///
/// All entries in [`contents`](Self::contents) are representations of the
/// same logical value and must be committed atomically. An empty `contents`
/// requests that the destination be cleared. This is distinct from a content
/// entry whose data has zero length.
///
/// The write is answered by calling [`reply`](Self::reply). This must happen
/// within the clipboard write request callback. Returning without replying
/// denies the write.
///
#[derive(Debug)]
pub struct ClipboardWrite<'t> {
    ptr: *const ffi::ClipboardWrite,
    _phan: PhantomData<&'t ()>,
}

impl<'t> ClipboardWrite<'t> {
    /// Name of the writing program for permission prompts, if the protocol
    /// carries one. Empty otherwise.
    #[must_use]
    pub fn name(&self) -> &'t [u8] {
        // SAFETY: The request and its strings live for the callback duration.
        unsafe { (*self.ptr).name.to_bytes() }
    }

    /// True if the terminal already holds a session grant for this request.
    /// The embedder should skip any permission prompt and perform the write.
    #[must_use]
    pub fn granted(&self) -> bool {
        // SAFETY: The request lives for the callback duration.
        unsafe { (*self.ptr).granted }
    }

    /// True if the program supplied a session password, so the embedder may
    /// offer to remember the user's decision through the `remember` argument
    /// of [`reply`](Self::reply). When false, `remember` is ignored.
    #[must_use]
    pub fn can_remember(&self) -> bool {
        // SAFETY: The request lives for the callback duration.
        unsafe { (*self.ptr).can_remember }
    }

    /// Answer the write.
    ///
    /// The result answers the program with the matching protocol status for
    /// protocols with a write acknowledgement (OSC 5522: DONE, EPERM, ENOSYS,
    /// EBUSY, EINVAL, EIO); protocols without one (OSC 52, OSC 1337 Copy)
    /// discard the reply.
    ///
    /// `remember` records a session grant so future requests from the same
    /// program skip the permission prompt. It is only honored on success when
    /// [`can_remember`](Self::can_remember) is set.
    pub fn reply(self, result: std::result::Result<(), ClipboardWriteError>, remember: bool) {
        let reply = ffi::ClipboardWriteReply {
            result: result.map_or_else(Into::into, |()| ffi::ClipboardWriteResult::SUCCESS),
            remember,
            ..ffi::sized!(ffi::ClipboardWriteReply)
        };
        // SAFETY: The request lives for the callback duration.
        if let Some(callback) = unsafe { (*self.ptr).reply } {
            // SAFETY: The reply only needs to outlive this synchronous call.
            unsafe { callback(self.ptr, &raw const reply) };
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that the given pointer has the correct lifetime.
    unsafe fn from_raw(ptr: *const ffi::ClipboardWrite) -> Self {
        Self {
            ptr,
            _phan: PhantomData,
        }
    }

    /// Get the clipboard's destination.
    #[must_use]
    pub fn location(&self) -> ClipboardLocation {
        // SAFETY: We trust libghostty to give us a valid pointer
        // within the lifetime of the callback.
        unsafe { (*self.ptr).location }
            .try_into()
            .unwrap_or(ClipboardLocation::Standard)
    }
    /// Get an iterator into a borrowed array of MIME representations.
    ///
    /// The iterator is empty for a write carrying no representations, which
    /// requests that the destination be cleared (e.g. OSC 52 with an empty
    /// payload).
    #[must_use]
    pub fn contents(&self) -> ClipboardContents<'t> {
        // SAFETY: We trust libghostty to give us a valid pointer
        // within the lifetime of the callback.
        let (ptr, len) = unsafe { ((*self.ptr).contents, (*self.ptr).contents_len) };
        // The C API declares `contents` optional and sends null for a write
        // carrying no representations (the "clear the clipboard" shape);
        // `from_raw_parts` requires a non-null pointer even at length zero.
        let contents: &'t [ffi::ClipboardContent] = match (ptr, len) {
            (ptr, len) if !ptr.is_null() => {
                // SAFETY: We trust libghostty to give us a valid pointer and
                // length within the lifetime of the callback.
                unsafe { std::slice::from_raw_parts(ptr, len) }
            }
            _ => &[],
        };
        ClipboardContents(contents.iter())
    }
}

/// An iterator into a borrowed array of MIME representations.
#[derive(Clone, Debug)]
pub struct ClipboardContents<'t>(std::slice::Iter<'t, ffi::ClipboardContent>);

impl<'t> Iterator for ClipboardContents<'t> {
    type Item = ClipboardContent<'t>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|v| unsafe { ClipboardContent::from_raw(v) })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl DoubleEndedIterator for ClipboardContents<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0
            .next_back()
            .map(|v| unsafe { ClipboardContent::from_raw(v) })
    }
}
impl ExactSizeIterator for ClipboardContents<'_> {}
impl std::iter::FusedIterator for ClipboardContents<'_> {}

/// One MIME representation in a clipboard write.
///
/// Both strings are borrowed and valid only for the duration of the callback.
/// The data is binary-safe and has already been decoded from any protocol-level
/// encoding. A zero-length data string is an explicit empty representation; it
/// does not clear the clipboard.
#[derive(Clone, Copy, Debug)]
pub struct ClipboardContent<'t> {
    /// MIME type of the representation.
    ///
    /// libghostty does not promise the MIME type is UTF-8, so it is exposed as
    /// raw bytes.
    pub mime: &'t [u8],
    /// Decoded, binary-safe representation data.
    pub data: &'t [u8],
}
impl ClipboardContent<'_> {
    /// # Safety
    ///
    /// Caller must guarantee that the given raw value is valid within
    /// the given lifetime.
    unsafe fn from_raw(value: &ffi::ClipboardContent) -> Self {
        // SAFETY: Upheld by caller
        unsafe {
            Self {
                // The C API does not promise UTF-8 for the MIME type, so
                // hand the bytes over as-is and let callers validate.
                mime: value.mime.to_bytes(),
                // The data is binary-safe per the C API (e.g. an image/png
                // representation), so it must not be exposed as `str`.
                data: value.data.to_bytes(),
            }
        }
    }
}

/// Clipboard a clipboard write targets or a clipboard read reads from.
///
/// Protocol-specific destination identifiers are normalized to these values
/// before the clipboard write callback is invoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
pub enum ClipboardLocation {
    /// The standard system clipboard.
    Standard = ffi::ClipboardLocation::STANDARD,
    /// The selection clipboard.
    Selection = ffi::ClipboardLocation::SELECTION,
    /// The primary selection clipboard.
    Primary = ffi::ClipboardLocation::PRIMARY,
}

/// Result of a clipboard write reply.
///
/// Protocols with a write acknowledgement (OSC 5522) answer the program with
/// the matching status; protocols without one (OSC 52, OSC 1337 Copy) discard
/// the reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
pub enum ClipboardWriteError {
    /// The clipboard write was denied by policy or the user.
    Denied = ffi::ClipboardWriteResult::DENIED,
    /// The destination or one or more representations are unsupported.
    Unsupported = ffi::ClipboardWriteResult::UNSUPPORTED,
    /// The clipboard is temporarily unavailable.
    Busy = ffi::ClipboardWriteResult::BUSY,
    /// One or more representations contain invalid data.
    InvalidData = ffi::ClipboardWriteResult::INVALID_DATA,
    /// The clipboard write failed due to an I/O error.
    IoError = ffi::ClipboardWriteResult::IO_ERROR,
}

/// A synchronous request to read clipboard contents.
///
/// The request is borrowed and valid only for the callback duration.
///
/// The read is answered by calling [`reply`](Self::reply). This must happen
/// before the callback returns. Returning without replying answers the
/// program with an empty clipboard (OSC 52) or EPERM (OSC 5522).
///
/// As the C API requires, only the fields within the size libghostty reports
/// for the request are read. Any that don't fit read as empty or false, and
/// [`reply`](Self::reply) then does nothing.
#[derive(Debug)]
pub struct ClipboardRead<'t> {
    ptr: *const ffi::ClipboardRead,
    _phan: PhantomData<&'t ()>,
}

impl<'t> ClipboardRead<'t> {
    /// # Safety
    ///
    /// Caller must ensure that the given pointer is valid for `'t`.
    unsafe fn from_raw(ptr: *const ffi::ClipboardRead) -> Self {
        Self {
            ptr,
            _phan: PhantomData,
        }
    }

    /// Clipboard to read.
    ///
    /// Locations this version of the bindings does not know about are
    /// reported as [`ClipboardLocation::Standard`]. That only happens when
    /// linking a newer libghostty than these bindings were generated for.
    #[must_use]
    pub fn location(&self) -> ClipboardLocation {
        // SAFETY: The request lives for the callback duration.
        unsafe { (*self.ptr).location }
            .try_into()
            .unwrap_or(ClipboardLocation::Standard)
    }

    /// The MIME types the program wants, in order of preference. Protocols
    /// that only carry text (OSC 52) request `text/plain`.
    ///
    /// The values come straight from the program, so they are exposed as
    /// raw bytes.
    #[must_use]
    pub fn mimes(&self) -> impl ExactSizeIterator<Item = &'t [u8]> + use<'t> {
        // SAFETY: The request lives for the callback duration.
        let (ptr, len) = unsafe { ((*self.ptr).mimes, (*self.ptr).mimes_len) };
        // `mimes` is NULL when `mimes_len` is zero (a targets-only OSC 5522
        // read), and `from_raw_parts` requires a non-null pointer even at
        // length zero. Check both, as `ClipboardWrite::contents` does, so
        // a NULL pointer can never reach it.
        let mimes: &'t [ffi::String] = match (ptr, len) {
            (ptr, len) if !ptr.is_null() && len > 0 => {
                // SAFETY: libghostty provides `mimes_len` strings that live
                // for the callback duration.
                unsafe { std::slice::from_raw_parts(ptr, len) }
            }
            _ => &[],
        };
        // SAFETY: Each string lives for the callback duration.
        mimes.iter().map(|mime| unsafe { mime.to_bytes() })
    }

    /// True if the program also wants the list of MIME types available on
    /// the clipboard, delivered through the `available` argument of
    /// [`reply`](Self::reply).
    #[must_use]
    pub fn list(&self) -> bool {
        // SAFETY: The request lives for the callback duration.
        unsafe { (*self.ptr).list }
    }

    /// Name of the requesting program for permission prompts, if the protocol
    /// carries one. Empty otherwise.
    #[must_use]
    pub fn name(&self) -> &'t [u8] {
        // SAFETY: The request and its strings live for the callback duration.
        unsafe { (*self.ptr).name.to_bytes() }
    }

    /// True if the terminal already holds a session grant for this request
    /// (kitty clipboard protocol passwords). The embedder should skip any
    /// permission prompt and serve the read.
    ///
    /// Always false when [`mimes`](Self::mimes) is empty: such a request is
    /// served without a prompt (see [`Terminal::on_clipboard_read`]), so the
    /// terminal never consults grants for it and a one-time password is
    /// preserved for the follow-up data read.
    #[must_use]
    pub fn granted(&self) -> bool {
        // SAFETY: The request lives for the callback duration.
        unsafe { (*self.ptr).granted }
    }

    /// True if the program supplied a session password, so the embedder may
    /// offer to remember the user's decision through the `remember` argument
    /// of [`reply`](Self::reply). When false, `remember` is ignored.
    #[must_use]
    pub fn can_remember(&self) -> bool {
        // SAFETY: The request lives for the callback duration.
        unsafe { (*self.ptr).can_remember }
    }

    /// Answer the read.
    ///
    /// Any error answers the program with an empty clipboard (OSC 52) or the
    /// matching protocol status (OSC 5522: EPERM, ENOSYS, EBUSY, EIO); the
    /// other arguments are ignored in that case. On success, `contents`
    /// should carry one representation per requested MIME type
    /// ([`mimes`](Self::mimes)) that the clipboard has; unrequested
    /// representations are ignored. Protocols that carry a single text value
    /// (OSC 52) use the first entry with a text MIME type such as
    /// `text/plain`.
    ///
    /// `available` lists all MIME types available on the clipboard. It is
    /// only used when [`list`](Self::list) is set.
    ///
    /// `remember` records a session grant so future requests from the same
    /// program skip the permission prompt. It is only honored on success when
    /// [`can_remember`](Self::can_remember) is set.
    ///
    /// All arguments are borrowed only for the duration of this call.
    ///
    /// The answer is written to the pty before this returns, so the
    /// [pty write callback](Terminal::on_pty_write) runs inside this call. If
    /// both callbacks share state through a [`RefCell`](std::cell::RefCell),
    /// don't keep it borrowed across `reply`: the pty write callback's borrow
    /// would panic, and a panic in a callback aborts the process.
    pub fn reply(
        self,
        result: std::result::Result<&[ClipboardReplyContent<'_>], ClipboardReadError>,
        available: &[ClipboardMime<'_>],
        remember: bool,
    ) {
        let (result, contents) = match result {
            Ok(contents) => (ffi::ClipboardReadResult::SUCCESS, contents),
            Err(error) => (error.into(), &[][..]),
        };
        // Both wrapper types are `repr(transparent)` over their C
        // counterparts, so the slices can be handed over without copying.
        let reply = ffi::ClipboardReadReply {
            result,
            contents: contents.as_ptr().cast(),
            contents_len: contents.len(),
            available: available.as_ptr().cast(),
            available_len: available.len(),
            remember,
            ..ffi::sized!(ffi::ClipboardReadReply)
        };
        // SAFETY: The request lives for the callback duration.
        if let Some(callback) = unsafe { (*self.ptr).reply } {
            // SAFETY: The reply and the buffers it points to only need to
            // outlive this synchronous call.
            unsafe { callback(self.ptr, &raw const reply) };
        }
    }
}

/// One MIME representation in a [`ClipboardRead::reply`].
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct ClipboardReplyContent<'a> {
    raw: ffi::ClipboardContent,
    _phan: PhantomData<&'a [u8]>,
}

impl<'a> ClipboardReplyContent<'a> {
    /// A representation of the clipboard contents with the given MIME type,
    /// such as `b"text/plain"`.
    ///
    /// The data is binary-safe. libghostty takes the MIME type as a
    /// `GhosttyString` with no UTF-8 promise, so it is raw bytes too, and a
    /// MIME type read from [`ClipboardRead::mimes`] can be passed straight
    /// back.
    #[must_use]
    // Both slices are borrowed into the struct for `'a` by a `const fn`, so
    // they're `&'a [u8]` rather than `impl AsRef<[u8]>`: trait methods can't
    // run in a `const fn`.
    pub const fn new(mime: &'a [u8], data: &'a [u8]) -> Self {
        Self {
            raw: ffi::ClipboardContent {
                mime: ffi::String {
                    ptr: mime.as_ptr(),
                    len: mime.len(),
                },
                data: ffi::String {
                    ptr: data.as_ptr(),
                    len: data.len(),
                },
            },
            _phan: PhantomData,
        }
    }
}

/// A MIME type passed to libghostty without copying, as listed in a
/// [`ClipboardRead::reply`] or offered to [`Terminal::paste`].
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct ClipboardMime<'a> {
    raw: ffi::String,
    _phan: PhantomData<&'a [u8]>,
}

impl<'a> ClipboardMime<'a> {
    /// A MIME type such as `b"text/plain"`.
    ///
    /// libghostty takes MIME types as a `GhosttyString` with no UTF-8
    /// promise, so they are raw bytes, and a MIME type read from
    /// [`ClipboardRead::mimes`] can be passed straight back.
    #[must_use]
    // Borrowed into the struct for `'a` by a `const fn`, so `&'a [u8]` rather
    // than `impl AsRef<[u8]>`, as in `ClipboardReplyContent::new`.
    pub const fn new(mime: &'a [u8]) -> Self {
        Self {
            raw: ffi::String {
                ptr: mime.as_ptr(),
                len: mime.len(),
            },
            _phan: PhantomData,
        }
    }
}

/// Errors that can be returned in a [`ClipboardRead::reply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum ClipboardReadError {
    /// The clipboard read was denied by policy or the user.
    Denied = ffi::ClipboardReadResult::DENIED,
    /// The embedder cannot read this clipboard.
    Unsupported = ffi::ClipboardReadResult::UNSUPPORTED,
    /// The clipboard is temporarily unavailable.
    Busy = ffi::ClipboardReadResult::BUSY,
    /// Reading the clipboard failed due to an I/O error.
    IoError = ffi::ClipboardReadResult::IO_ERROR,
}

/// An unsupported terminal sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnknownSequence<'t> {
    /// Application Program Command (APC).
    ///
    /// The content is borrowed and valid only for the callback duration. It
    /// contains the bytes between the sequence introducer and terminator, may
    /// contain arbitrary binary data, and is not null-terminated.
    #[non_exhaustive]
    Apc {
        /// Retained sequence content.
        content: &'t [u8],
        /// Whether content was shortened by the byte limit or allocation
        /// failure.
        truncated: bool,
    },
    /// An OSC sequence whose number libghostty-vt does not implement.
    ///
    /// OSC sequences start with `ESC ]`, followed by a number that identifies
    /// the command, usually a `;`, and then the command's data. The sequence
    /// ends with either BEL or ESC followed by a backslash. For example, a
    /// program might write:
    ///
    /// ```text
    /// ESC ] 7400;status=busy BEL
    /// ```
    ///
    /// For that sequence, `content` is `7400;status=busy` and `terminator`
    /// is [`osc::Terminator::Bel`]. See [`Terminal::on_unknown_sequence`]
    /// for a complete example.
    #[non_exhaustive]
    Osc {
        /// Everything between `ESC ]` and the terminator, including the number
        /// at the start. The bytes are not null-terminated and are only valid
        /// until the callback returns. Copy them if you need them later.
        content: &'t [u8],
        /// True if the sequence was longer than
        /// [`Terminal::set_unknown_max_bytes`], or memory ran out while
        /// reading it. In that case `content` holds only the beginning of the
        /// sequence.
        truncated: bool,
        /// How the program ended the sequence. If you send a reply, end it the
        /// same way.
        terminator: osc::Terminator,
    },
}

impl UnknownSequence<'_> {
    /// Reads a sequence, or gives `None` for a kind or OSC terminator these
    /// bindings don't know.
    ///
    /// This takes a pointer rather than a reference on purpose: upstream
    /// writes only the active union field, and for a kind without a payload
    /// it may leave the whole union unwritten. Referencing or copying such a
    /// union relies on validity rules Rust hasn't settled yet. So we only
    /// read the tag, and then only the union field the tag says is active.
    ///
    /// # Safety
    ///
    /// Caller must guarantee that `raw` is valid for reads and properly
    /// aligned, that its `tag` is initialized, that the union field named by
    /// `tag` is initialized, and that the content it points to is valid for
    /// the lifetime `'t`.
    unsafe fn from_raw<'t>(
        raw: *const ffi::TerminalUnknownSequence,
    ) -> Option<UnknownSequence<'t>> {
        // SAFETY: Upheld by caller. This reads just the tag field through
        // the raw pointer, without making a reference to the whole struct.
        match unsafe { (*raw).tag } {
            ffi::TerminalUnknownSequenceTag::APC => {
                // SAFETY: Upheld by caller; the tag says `apc` is the active
                // union field.
                let apc = unsafe { (*raw).value.apc };
                Some(UnknownSequence::Apc {
                    // SAFETY: Upheld by caller; the content is valid for `'t`.
                    content: unsafe { apc.content.to_bytes() },
                    truncated: apc.truncated,
                })
            }
            ffi::TerminalUnknownSequenceTag::OSC => {
                // SAFETY: Upheld by caller; the tag says `osc` is the active
                // union field.
                let osc = unsafe { (*raw).value.osc };
                // A terminator these bindings don't know has no Rust value
                // either, so skip the sequence (like `osc::Command`, which
                // reports it as `Invalid`) instead of inventing one.
                let terminator = osc.terminator.try_into().ok()?;
                Some(UnknownSequence::Osc {
                    // SAFETY: Ditto
                    content: unsafe { osc.content.to_bytes() },
                    truncated: osc.truncated,
                    terminator,
                })
            }
            // Upstream asks callbacks to ignore tags they don't handle.
            // Sequence kinds added upstream later have no Rust value, so they
            // are skipped rather than misreported; the enum is non-exhaustive
            // to grow with them.
            _ => None,
        }
    }
}

/// A request to show a desktop notification.
#[derive(Debug, Copy, Clone)]
pub struct DesktopNotification<'t> {
    ptr: *const ffi::TerminalDesktopNotification,
    _phan: PhantomData<&'t ()>,
}

impl<'t> DesktopNotification<'t> {
    unsafe fn from_raw(raw: *const ffi::TerminalDesktopNotification) -> Self {
        Self {
            ptr: raw,
            _phan: PhantomData,
        }
    }

    /// Notification title, or empty when the protocol omits it.
    ///
    /// The bytes come straight from the program and libghostty does not
    /// validate them, so they are not guaranteed to be UTF-8.
    #[must_use]
    pub fn title(self) -> &'t [u8] {
        // SAFETY: The notification and its strings live for the callback
        // duration.
        unsafe { (*self.ptr).title.to_bytes() }
    }

    /// Notification body.
    ///
    /// Like the title, these are the program's bytes, not necessarily UTF-8.
    #[must_use]
    pub fn body(self) -> &'t [u8] {
        // SAFETY: The notification and its strings live for the callback
        // duration.
        unsafe { (*self.ptr).body.to_bytes() }
    }
}

/// A request from the running program to resize the window.
///
/// Passed to [`Terminal::on_resize_window`].
///
/// New kinds may be added in later versions. Match with a wildcard arm and
/// ignore any kind you don't handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResizeWindow {
    /// Resize the window's text area to a size in rows and columns
    /// (CSI 8 ; rows ; columns t).
    Grid {
        /// Requested height in rows, or `None` to keep the current height.
        rows: Option<NonZeroU16>,
        /// Requested width in columns, or `None` to keep the current width.
        columns: Option<NonZeroU16>,
    },
}

impl ResizeWindow {
    /// Reads a request, or gives `None` for a kind these bindings don't know.
    ///
    /// This takes a pointer rather than the struct by value on purpose: for
    /// a kind without a payload upstream may leave the whole union
    /// unwritten, and copying an entirely uninitialized union relies on
    /// validity rules Rust hasn't settled yet. So we only read the tag, and
    /// then only the union field the tag says is active.
    ///
    /// # Safety
    ///
    /// Caller must guarantee that `raw` is valid for reads and properly
    /// aligned, that its `tag` is initialized, and that the union field named
    /// by `tag` is initialized.
    unsafe fn from_raw(raw: *const ffi::TerminalResizeWindow) -> Option<Self> {
        // SAFETY: Upheld by caller. This reads just the tag field through
        // the raw pointer, without making a reference to the whole struct.
        match unsafe { (*raw).tag } {
            ffi::TerminalResizeWindowTag::GRID => {
                // SAFETY: Upheld by caller; the tag says `grid` is the
                // active union field.
                let ffi::TerminalResizeWindowGrid { rows, columns } = unsafe { (*raw).value.grid };
                // Upstream uses 0 as "keep the current size", which is
                // exactly what `None` means here.
                Some(Self::Grid {
                    rows: NonZeroU16::new(rows),
                    columns: NonZeroU16::new(columns),
                })
            }
            // Upstream may add kinds without an ABI break. We can't read
            // their payload, so there's no Rust value for them.
            _ => None,
        }
    }
}

/// A shell integration event, passed to the
/// [`Terminal::on_semantic_prompt`] callback.
///
/// [`kind`](Self::kind) says which step of the command this is. The other
/// fields only carry information for the kinds listed on each field, and are
/// zero or empty otherwise.
///
/// Strings are only valid during the callback. Copy them if you need them
/// later. The lifetime `'t` enforces this: the event cannot escape the
/// [`Terminal::on_semantic_prompt`] callback it was handed to.
#[derive(Debug, Copy, Clone)]
pub struct SemanticPrompt<'t> {
    ptr: *const ffi::TerminalSemanticPrompt,
    _phan: PhantomData<&'t ()>,
}

impl<'t> SemanticPrompt<'t> {
    /// # Safety
    ///
    /// Caller must guarantee that `raw` and the strings it points to are
    /// valid for the lifetime `'t`.
    unsafe fn from_raw(raw: *const ffi::TerminalSemanticPrompt) -> Self {
        Self {
            ptr: raw,
            _phan: PhantomData,
        }
    }

    /// The raw event.
    ///
    /// The event has a frozen layout, so every field is readable.
    fn raw(self) -> &'t ffi::TerminalSemanticPrompt {
        // SAFETY: We trust libghostty to give us a valid underlying ptr,
        // which lives for the callback duration, i.e. `'t`.
        unsafe { &*self.ptr }
    }

    /// Which step of the command this event reports.
    ///
    /// A kind value from a newer libghostty that these bindings don't know
    /// yet gives [`Error::InvalidValue`].
    pub fn kind(self) -> Result<SemanticPromptKind> {
        self.raw().kind.try_into().map_err(|_| Error::InvalidValue)
    }

    /// Which prompt is starting. Set for
    /// [`SemanticPromptKind::PromptStart`]. Always [`PromptKind::Primary`]
    /// for other kinds.
    ///
    /// A prompt kind value from a newer libghostty that these bindings don't
    /// know yet gives [`Error::InvalidValue`].
    pub fn prompt_kind(self) -> Result<PromptKind> {
        self.raw()
            .prompt_kind
            .try_into()
            .map_err(|_| Error::InvalidValue)
    }

    /// The command's exit code, or `None` if the shell didn't report one.
    /// Only ever `Some` for [`SemanticPromptKind::CommandEnd`].
    ///
    /// Exit codes can be negative, for example on Windows.
    #[must_use]
    pub fn exit_code(self) -> Option<i32> {
        // Upstream says to trust `has_exit_code` rather than any special
        // `exit_code` value (it writes 0 when there is none, which is also a
        // perfectly good exit code), so that's the only thing we look at.
        let raw = self.raw();
        raw.has_exit_code.then_some(raw.exit_code)
    }

    /// The command line that is about to run, for
    /// [`SemanticPromptKind::OutputStart`]. The shell sends it encoded, and
    /// this is the decoded text. Empty if the shell didn't send one or it
    /// couldn't be decoded.
    #[must_use]
    pub fn command(self) -> &'t [u8] {
        // Upstream decodes the command line but never promises the result
        // is UTF-8 (percent-encoding can produce any bytes), so hand out
        // the raw bytes and let callers validate.
        //
        // SAFETY: The event and its strings live for the callback
        // duration, i.e. `'t`.
        unsafe { self.raw().command.to_bytes() }
    }

    /// A description of what went wrong, for
    /// [`SemanticPromptKind::CommandEnd`] when the shell sent one. Empty
    /// otherwise. Few shells send this. The [exit code](Self::exit_code) is
    /// the usual way to tell whether a command failed.
    #[must_use]
    pub fn error(self) -> &'t [u8] {
        // SAFETY: Ditto.
        unsafe { self.raw().error.to_bytes() }
    }
}

/// The step of a command that a shell integration event reports.
///
/// More kinds may be added in later versions, so ignore any kind you don't
/// handle.
// Upstream's `GHOSTTY_SEMANTIC_PROMPT_INVALID` (0) is left out on purpose:
// it is "never reported" and only exists so a zeroed struct isn't mistaken
// for a real event. If it ever shows up, `kind()` gives `Err` like any other
// unknown value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum SemanticPromptKind {
    /// The shell started drawing a prompt.
    /// [`prompt_kind`](SemanticPrompt::prompt_kind) says which one.
    PromptStart = ffi::SemanticPromptKind::PROMPT_START,
    /// The prompt is drawn and the user can start typing a command.
    InputStart = ffi::SemanticPromptKind::INPUT_START,
    /// The user submitted the command and it started running. Anything the
    /// terminal receives after this is the command's output.
    OutputStart = ffi::SemanticPromptKind::OUTPUT_START,
    /// The command finished running.
    CommandEnd = ffi::SemanticPromptKind::COMMAND_END,
}

/// Which prompt a [`SemanticPromptKind::PromptStart`] event starts.
///
/// Most shells only draw a primary prompt. Some also draw a prompt on the
/// right side of the line, or a prompt at the start of each extra line
/// when a command spans several lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum PromptKind {
    /// The main prompt shown before each command. This is used when the
    /// shell doesn't say which prompt it is drawing.
    Primary = ffi::SemanticPromptPromptKind::PRIMARY,
    /// A prompt drawn at the right edge of the line, such as zsh's
    /// RPROMPT.
    Right = ffi::SemanticPromptPromptKind::RIGHT,
    /// A prompt at the start of an extra line of a command that spans
    /// several lines.
    Continuation = ffi::SemanticPromptPromptKind::CONTINUATION,
    /// Another prompt for an extra line of input, such as bash's PS2.
    /// Shells differ in whether they report extra lines as continuation or
    /// secondary prompts, so most applications should treat the two the
    /// same.
    Secondary = ffi::SemanticPromptPromptKind::SECONDARY,
}

/// A program status report (OSC 7501).
///
/// The program status protocol lets a program tell the terminal what it is
/// doing: idle, working, done, waiting on the user, or failed, and why. It
/// is meant for long-running work like builds, deploys, and coding agents,
/// where the user is often looking at something else and wants to know when
/// the work finishes or needs them. The protocol only describes state. How
/// to show it, if at all, is up to your application.
///
/// The full specification is at
/// <https://www.superlogical.com/rex/docs/build/program-status>
///
/// For example, a program waiting for the user to approve a change sends
/// this, where ST is the string terminator (ESC \ or BEL):
///
/// ```text
/// ESC ] 7501 ; state=blocked:kind=permission:app=terraform:msg=QXBwbHk/ ST
/// ```
///
/// The callback then receives a report with:
///
/// - [`state`](Self::state): [`ProgramStatusState::Blocked`]
/// - [`kind`](Self::kind): [`ProgramStatusKind::Permission`]
/// - [`progress`](Self::progress): `None`, because the program didn't send
///   one, so the work is indeterminate
/// - [`id`](Self::id): empty, because this is the root record
/// - [`app`](Self::app): `"terraform"`
/// - [`title`](Self::title): empty
/// - [`message`](Self::message): `"Apply?"`, decoded from the base64 in `msg`
///
/// Only reports that pass every check in the specification reach the
/// callback. Text that the program didn't send is an empty string. All
/// strings are only valid during the callback, so copy any you want to keep.
/// The lifetime `'t` enforces this: the report cannot escape the
/// [`Terminal::on_program_status`] callback it was handed to.
#[derive(Debug, Copy, Clone)]
pub struct ProgramStatus<'t> {
    ptr: *const ffi::TerminalProgramStatus,
    _phan: PhantomData<&'t ()>,
}

impl<'t> ProgramStatus<'t> {
    /// # Safety
    ///
    /// Caller must guarantee that `raw` and the strings it points to are
    /// valid for the lifetime `'t`.
    unsafe fn from_raw(raw: *const ffi::TerminalProgramStatus) -> Self {
        Self {
            ptr: raw,
            _phan: PhantomData,
        }
    }

    /// The raw report.
    ///
    /// The report has a frozen layout, so every field is readable.
    fn raw(self) -> &'t ffi::TerminalProgramStatus {
        // SAFETY: We trust libghostty to give us a valid underlying ptr,
        // which lives for the callback duration, i.e. `'t`.
        unsafe { &*self.ptr }
    }

    /// What the program is doing.
    ///
    /// A state value from a newer libghostty that these bindings don't know
    /// yet gives [`Error::InvalidValue`].
    pub fn state(self) -> Result<ProgramStatusState> {
        self.raw().state.try_into().map_err(|_| Error::InvalidValue)
    }

    /// What the program needs from the user. Only set for
    /// [`ProgramStatusState::Blocked`]. It is [`ProgramStatusKind::None`]
    /// for other states, when the program didn't say, or when it sent a kind
    /// this version of libghostty doesn't know.
    ///
    /// A kind value from a newer libghostty that these bindings don't know
    /// yet gives [`Error::InvalidValue`].
    pub fn kind(self) -> Result<ProgramStatusKind> {
        self.raw().kind.try_into().map_err(|_| Error::InvalidValue)
    }

    /// How far along the work is, from 0 through 100, or `None` when there
    /// is no percentage. Only set for [`ProgramStatusState::Working`] and
    /// [`ProgramStatusState::Blocked`]. For those states, `None` means the
    /// work is indeterminate: the program is busy but has no percentage to
    /// report. It is also `None` for other states and when the program sent
    /// a value outside that range.
    #[must_use]
    pub fn progress(self) -> Option<u8> {
        // Upstream uses -1 as the "no percentage" sentinel and otherwise
        // only ever sends 0 through 100, so every negative value is `None`.
        u8::try_from(self.raw().progress).ok()
    }

    /// Which record this report is about. Empty for the root record.
    ///
    /// A program that only reports on itself leaves this empty. A program
    /// that reports on several things at once gives each its own id, such as
    /// `"us-east"` and `"eu-west"` for a deploy to two regions. A `/` makes
    /// one record the child of another, so `"build/test"` is a child of
    /// `"build"`. The parent record doesn't have to exist.
    #[must_use]
    pub fn id(self) -> &'t [u8] {
        // SAFETY: The report and its strings live for the callback
        // duration, i.e. `'t`.
        unsafe { self.raw().id.to_bytes() }
    }

    /// A stable name for the program that a machine can match on, such as
    /// `"cargo"`, `"terraform"`, or `"claude-code"`. Use it as a key for
    /// grouping, filtering, or choosing an icon. It is not a label. The label
    /// is [`title`](Self::title). Empty when the report didn't include one.
    /// See [`Terminal::on_program_status`] for how an empty app is filled in
    /// from a parent record.
    #[must_use]
    pub fn app(self) -> &'t [u8] {
        // SAFETY: Ditto.
        unsafe { self.raw().app.to_bytes() }
    }

    /// A short label for the record, meant for people. Programs that report
    /// several records use this to tell them apart.
    #[must_use]
    pub fn title(self) -> &'t [u8] {
        // SAFETY: Ditto.
        unsafe { self.raw().title.to_bytes() }
    }

    /// One line of text for people, saying what the record is doing,
    /// waiting for, or has finished. You may shorten it to fit, but don't
    /// try to read meaning into it.
    #[must_use]
    pub fn message(self) -> &'t [u8] {
        // SAFETY: Ditto.
        unsafe { self.raw().message.to_bytes() }
    }
}

/// What a program says it is doing, in a program status report (OSC 7501).
///
/// See [`ProgramStatus`] for an overview of the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum ProgramStatusState {
    /// At rest, waiting for the user's next instruction. For example, an
    /// interactive tool sitting at its own prompt.
    Idle = ffi::ProgramStatusState::IDLE,
    /// Running on its own. The report may include a progress percentage.
    Working = ffi::ProgramStatusState::WORKING,
    /// Finished a piece of work, and the result is ready for the user to
    /// look at.
    Done = ffi::ProgramStatusState::DONE,
    /// Can't continue until the user does something.
    /// [`kind`](ProgramStatus::kind) says what the program needs and
    /// [`message`](ProgramStatus::message) says why. The report may include
    /// a progress percentage.
    Blocked = ffi::ProgramStatusState::BLOCKED,
    /// Failed and stopped.
    Error = ffi::ProgramStatusState::ERROR,
    /// Not a real state. Remove the record with this report's id and every
    /// record beneath it. If the id is empty, remove every record.
    Clear = ffi::ProgramStatusState::CLEAR,
}

/// What a blocked program needs from the user, in a program status report
/// (OSC 7501).
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum ProgramStatusKind {
    /// The program didn't say, or the state isn't
    /// [`ProgramStatusState::Blocked`].
    None = ffi::ProgramStatusKind::NONE,
    /// Approval to do something, such as "Apply these changes?".
    Permission = ffi::ProgramStatusKind::PERMISSION,
    /// An answer the user has to type.
    Question = ffi::ProgramStatusKind::QUESTION,
    /// A login, password, token, or other credential.
    Auth = ffi::ProgramStatusKind::AUTH,
}

/// A progress report emitted by the running program.
#[derive(Debug, Copy, Clone)]
pub struct ProgressReport<'t> {
    ptr: *const ffi::TerminalProgressReport,
    _phan: PhantomData<&'t ()>,
}

impl ProgressReport<'_> {
    unsafe fn from_raw(raw: *const ffi::TerminalProgressReport) -> Self {
        Self {
            ptr: raw,
            _phan: PhantomData,
        }
    }

    /// Literal progress state reported by the running program.
    pub fn state(self) -> Result<ProgressState> {
        // SAFETY: We trust libghostty to give us a valid underlying ptr
        unsafe { *self.ptr }
            .state
            .try_into()
            .map_err(|_| Error::InvalidValue)
    }

    /// Progress percentage from 0 through 100, or `None` when omitted.
    #[must_use]
    pub fn progress(self) -> Option<u8> {
        // SAFETY: We trust libghostty to give us a valid underlying ptr
        // Negative means omitted.
        u8::try_from(unsafe { *self.ptr }.progress).ok()
    }
}

/// State of a terminal progress report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
#[non_exhaustive]
pub enum ProgressState {
    /// Remove any visible progress indication.
    Remove = ffi::TerminalProgressState::REMOVE,
    /// Show determinate progress.
    Set = ffi::TerminalProgressState::SET,
    /// Show a failed progress state.
    Error = ffi::TerminalProgressState::ERROR,
    /// Show indeterminate progress.
    Indeterminate = ffi::TerminalProgressState::INDETERMINATE,
    /// Show paused progress.
    Pause = ffi::TerminalProgressState::PAUSE,
}

/// Device attributes response data for all three DA levels.
/// Filled by the [`Terminal::on_device_attributes`] callback in response
/// to CSI c, CSI > c, or CSI = c queries. The terminal uses whichever
/// sub-struct matches the request type.
#[derive(Debug, Clone, Copy)]
pub struct DeviceAttributes {
    /// Primary device attributes (DA1).
    pub primary: PrimaryDeviceAttributes,
    /// Secondary device attributes (DA2).
    pub secondary: SecondaryDeviceAttributes,
    /// Tertiary device attributes (DA3).
    pub tertiary: TertiaryDeviceAttributes,
}

impl From<DeviceAttributes> for ffi::DeviceAttributes {
    fn from(value: DeviceAttributes) -> Self {
        Self {
            primary: value.primary.into(),
            secondary: value.secondary.into(),
            tertiary: value.tertiary.into(),
        }
    }
}

/// Primary device attributes (DA1) response data.
///
/// Returned as part of [`DeviceAttributes`] in response to a CSI c query.
#[derive(Debug, Clone, Copy)]
pub struct PrimaryDeviceAttributes(ffi::DeviceAttributesPrimary);

impl PrimaryDeviceAttributes {
    /// Construct primary device attributes from a conformance level
    /// and an array of device attribute features.
    ///
    /// Prefer defining primary device attributes as a `const` when the feature
    /// list is statically known. That makes the 64-feature limit fail during
    /// compilation instead of panicking at runtime.
    ///
    /// # Panics
    ///
    /// **Panics** when more than 64 features are given.
    #[must_use]
    pub const fn new(
        conformance_level: ConformanceLevel,
        features: &[DeviceAttributeFeature],
    ) -> Self {
        assert!(features.len() <= 64);

        let mut f = [0u16; 64];
        let mut i = 0;
        while i < features.len() {
            f[i] = features[i].0;
            i += 1;
        }

        Self(ffi::DeviceAttributesPrimary {
            conformance_level: conformance_level.0,
            features: f,
            num_features: features.len(),
        })
    }
}

impl From<PrimaryDeviceAttributes> for ffi::DeviceAttributesPrimary {
    fn from(value: PrimaryDeviceAttributes) -> Self {
        value.0
    }
}

/// The level of conformance to the behavior of a specific or a family of
/// physical terminal models.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConformanceLevel(pub u16);

impl ConformanceLevel {
    #![expect(clippy::doc_markdown, reason = "false positive")]
    #![expect(missing_docs, reason = "self-explanatory")]
    pub const VT100: Self = Self(ffi::DA_CONFORMANCE_VT100);
    pub const VT101: Self = Self(ffi::DA_CONFORMANCE_VT101);
    pub const VT102: Self = Self(ffi::DA_CONFORMANCE_VT102);
    pub const VT125: Self = Self(ffi::DA_CONFORMANCE_VT125);
    pub const VT131: Self = Self(ffi::DA_CONFORMANCE_VT131);
    pub const VT132: Self = Self(ffi::DA_CONFORMANCE_VT132);
    pub const VT220: Self = Self(ffi::DA_CONFORMANCE_VT220);
    pub const VT240: Self = Self(ffi::DA_CONFORMANCE_VT240);
    pub const VT320: Self = Self(ffi::DA_CONFORMANCE_VT320);
    pub const VT340: Self = Self(ffi::DA_CONFORMANCE_VT340);
    pub const VT420: Self = Self(ffi::DA_CONFORMANCE_VT420);
    pub const VT510: Self = Self(ffi::DA_CONFORMANCE_VT510);
    pub const VT520: Self = Self(ffi::DA_CONFORMANCE_VT520);
    pub const VT525: Self = Self(ffi::DA_CONFORMANCE_VT525);
    /// Equivalent to a VT2xx terminal.
    pub const LEVEL_2: Self = Self(ffi::DA_CONFORMANCE_LEVEL_2);
    /// Equivalent to a VT3xx terminal.
    pub const LEVEL_3: Self = Self(ffi::DA_CONFORMANCE_LEVEL_3);
    /// Equivalent to a VT4xx terminal.
    pub const LEVEL_4: Self = Self(ffi::DA_CONFORMANCE_LEVEL_4);
    /// Equivalent to a VT5xx terminal.
    pub const LEVEL_5: Self = Self(ffi::DA_CONFORMANCE_LEVEL_5);
}

/// A feature that a terminal can report to support.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceAttributeFeature(pub u16);

impl DeviceAttributeFeature {
    #![expect(missing_docs, reason = "no upstream documentation provided")]
    pub const COLUMNS_132: Self = Self(ffi::DA_FEATURE_COLUMNS_132);
    pub const PRINTER: Self = Self(ffi::DA_FEATURE_PRINTER);
    pub const REGIS: Self = Self(ffi::DA_FEATURE_REGIS);
    pub const SIXEL: Self = Self(ffi::DA_FEATURE_SIXEL);
    pub const SELECTIVE_ERASE: Self = Self(ffi::DA_FEATURE_SELECTIVE_ERASE);
    pub const USER_DEFINED_KEYS: Self = Self(ffi::DA_FEATURE_USER_DEFINED_KEYS);
    pub const NATIONAL_REPLACEMENT: Self = Self(ffi::DA_FEATURE_NATIONAL_REPLACEMENT);
    pub const TECHNICAL_CHARACTERS: Self = Self(ffi::DA_FEATURE_TECHNICAL_CHARACTERS);
    pub const LOCATOR: Self = Self(ffi::DA_FEATURE_LOCATOR);
    pub const TERMINAL_STATE: Self = Self(ffi::DA_FEATURE_TERMINAL_STATE);
    pub const WINDOWING: Self = Self(ffi::DA_FEATURE_WINDOWING);
    pub const HORIZONTAL_SCROLLING: Self = Self(ffi::DA_FEATURE_HORIZONTAL_SCROLLING);
    pub const ANSI_COLOR: Self = Self(ffi::DA_FEATURE_ANSI_COLOR);
    pub const RECTANGULAR_EDITING: Self = Self(ffi::DA_FEATURE_RECTANGULAR_EDITING);
    pub const ANSI_TEXT_LOCATOR: Self = Self(ffi::DA_FEATURE_ANSI_TEXT_LOCATOR);
    pub const CLIPBOARD: Self = Self(ffi::DA_FEATURE_CLIPBOARD);
}

/// Secondary device attributes (DA2) response data.
///
/// Returned as part of [`DeviceAttributes`] in response to a CSI > c query.
/// Response format: CSI > Pp ; Pv ; Pc c
#[derive(Debug, Copy, Clone)]
pub struct SecondaryDeviceAttributes {
    /// Terminal type identifier (Pp).
    pub device_type: DeviceType,
    /// Firmware/patch version number (Pv).
    pub firmware_version: u16,
    /// ROM cartridge registration number (Pc). Always 0 for emulators.
    pub rom_cartridge: u16,
}

impl From<SecondaryDeviceAttributes> for ffi::DeviceAttributesSecondary {
    fn from(value: SecondaryDeviceAttributes) -> Self {
        Self {
            device_type: value.device_type.0,
            firmware_version: value.firmware_version,
            rom_cartridge: value.rom_cartridge,
        }
    }
}

/// The type of terminal device being emulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceType(pub u16);

impl DeviceType {
    #![expect(missing_docs, reason = "self-explanatory")]
    pub const VT100: Self = Self(ffi::DA_DEVICE_TYPE_VT100);
    pub const VT220: Self = Self(ffi::DA_DEVICE_TYPE_VT220);
    pub const VT240: Self = Self(ffi::DA_DEVICE_TYPE_VT240);
    pub const VT330: Self = Self(ffi::DA_DEVICE_TYPE_VT330);
    pub const VT340: Self = Self(ffi::DA_DEVICE_TYPE_VT340);
    pub const VT320: Self = Self(ffi::DA_DEVICE_TYPE_VT320);
    pub const VT382: Self = Self(ffi::DA_DEVICE_TYPE_VT382);
    pub const VT420: Self = Self(ffi::DA_DEVICE_TYPE_VT420);
    pub const VT510: Self = Self(ffi::DA_DEVICE_TYPE_VT510);
    pub const VT520: Self = Self(ffi::DA_DEVICE_TYPE_VT520);
    pub const VT525: Self = Self(ffi::DA_DEVICE_TYPE_VT525);
}

/// Tertiary device attributes (DA3) response data.
///
/// Returned as part of [`DeviceAttributes`] in response to a CSI = c query.
/// Response format: DCS ! | D...D ST (DECRPTUI).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TertiaryDeviceAttributes {
    /// Unit ID encoded as 8 uppercase hex digits in the response.
    pub unit_id: u32,
}

impl From<TertiaryDeviceAttributes> for ffi::DeviceAttributesTertiary {
    fn from(value: TertiaryDeviceAttributes) -> Self {
        Self {
            unit_id: value.unit_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenderState, terminal::Boundary};
    use std::{
        cell::{Cell, RefCell},
        mem::ManuallyDrop,
    };

    #[inline(never)]
    fn build_terminal(callback_count: &RefCell<usize>) -> Terminal<'static, '_> {
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");

        terminal
            .on_device_attributes(move |_term| {
                *callback_count.borrow_mut() += 1;
                Some(DeviceAttributes {
                    primary: PrimaryDeviceAttributes::new(
                        ConformanceLevel::VT220,
                        &[DeviceAttributeFeature::ANSI_COLOR],
                    ),
                    secondary: SecondaryDeviceAttributes {
                        device_type: DeviceType::VT220,
                        firmware_version: 1,
                        rom_cartridge: 0,
                    },
                    tertiary: TertiaryDeviceAttributes { unit_id: 0 },
                })
            })
            .expect("callback should register");

        terminal
    }

    /// Move a value into distinct heap storage with an explicit byte-for-byte
    /// relocation so the test does not rely on optimizer or allocator behavior.
    fn relocate_into_new_box<T>(value: T) -> (Box<T>, usize, usize) {
        // Keep the source allocation alive without running T's destructor.
        // We need the bytes to remain initialized until after the copy.
        let src = Box::new(ManuallyDrop::new(value));
        let src_addr = std::ptr::from_ref(&**src).cast::<T>() as usize;

        unsafe {
            let dst_layout = std::alloc::Layout::new::<T>();
            let dst_ptr = std::alloc::alloc(dst_layout).cast::<T>();
            if dst_ptr.is_null() {
                std::alloc::handle_alloc_error(dst_layout);
            }

            let dst_addr = dst_ptr as usize;
            assert_ne!(
                src_addr, dst_addr,
                "test setup failed: source and destination storage unexpectedly match"
            );

            // SAFETY: src points to a fully initialized T wrapped in
            // ManuallyDrop, dst points to distinct uninitialized storage for
            // exactly one T, and the regions do not overlap.
            std::ptr::copy_nonoverlapping(std::ptr::from_ref(&**src).cast::<T>(), dst_ptr, 1);

            // SAFETY: src was allocated as Box<ManuallyDrop<T>> and must be
            // freed without dropping T because ownership was transferred by
            // the raw byte copy above.
            std::alloc::dealloc(
                Box::into_raw(src).cast::<u8>(),
                std::alloc::Layout::new::<ManuallyDrop<T>>(),
            );

            // SAFETY: We just initialized dst_ptr by copying a valid T into it,
            // so it now owns exactly one initialized T allocation.
            (Box::from_raw(dst_ptr), src_addr, dst_addr)
        }
    }

    /// Explicitly relocate the Terminal into distinct storage, then verify the
    /// callback still fires through the stable `VTable` userdata pointer.
    #[test]
    fn callbacks_survive_explicit_relocation() {
        let callback_count = RefCell::new(0usize);
        let terminal = build_terminal(&callback_count);
        let (mut terminal, addr_before, addr_after) = relocate_into_new_box(terminal);
        assert_ne!(addr_before, addr_after);

        // Primary DA request (CSI c) should invoke on_device_attributes.
        terminal.vt_write(b"\x1b[c");
        assert_eq!(*callback_count.borrow(), 1);
    }

    /// What a clipboard read callback observed about its request.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct SeenRead {
        location: Option<ClipboardLocation>,
        mimes: Vec<Vec<u8>>,
        list: bool,
        name: Vec<u8>,
        granted: bool,
        can_remember: bool,
    }

    /// Feed `input` to a terminal whose clipboard read callback records the
    /// request and hands it to `answer`, returning what was seen and what
    /// the terminal wrote back to the pty.
    ///
    /// Assertions happen outside the callback: a panic inside it would
    /// abort the whole test binary instead of failing the test.
    fn clipboard_read(
        input: &[u8],
        answer: impl Fn(ClipboardRead<'_>),
    ) -> (Vec<SeenRead>, Vec<u8>) {
        let seen = RefCell::new(Vec::new());
        let output = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .on_pty_write(|_term, bytes| output.borrow_mut().extend_from_slice(bytes))
            .expect("callback should register");
        terminal
            .on_clipboard_read(|_term, request| {
                seen.borrow_mut().push(SeenRead {
                    location: Some(request.location()),
                    mimes: request.mimes().map(<[u8]>::to_vec).collect(),
                    list: request.list(),
                    name: request.name().to_vec(),
                    granted: request.granted(),
                    can_remember: request.can_remember(),
                });
                answer(request);
            })
            .expect("callback should register");
        terminal.vt_write(input);
        drop(terminal);
        (seen.into_inner(), output.into_inner())
    }

    #[test]
    fn osc52_clipboard_read() {
        let (seen, output) = clipboard_read(b"\x1b]52;c;?\x1b\\", |request| {
            request.reply(
                Ok(&[ClipboardReplyContent::new(b"text/plain", b"hello")]),
                &[],
                false,
            );
        });
        assert_eq!(
            seen,
            [SeenRead {
                location: Some(ClipboardLocation::Standard),
                mimes: vec![b"text/plain".to_vec()],
                ..SeenRead::default()
            }]
        );
        assert_eq!(output, b"\x1b]52;c;aGVsbG8=\x1b\\");

        // Errors and missing replies both answer with an empty clipboard.
        let (_, output) = clipboard_read(b"\x1b]52;c;?\x1b\\", |request| {
            request.reply(Err(ClipboardReadError::Denied), &[], false);
        });
        assert_eq!(output, b"\x1b]52;c;\x1b\\");
        let (_, output) = clipboard_read(b"\x1b]52;c;?\x1b\\", |_request| {});
        assert_eq!(output, b"\x1b]52;c;\x1b\\");
    }

    #[test]
    fn osc5522_clipboard_read() {
        // Requests text/plain plus the listing ("."), from a program named
        // "app" without a password.
        let (seen, output) = clipboard_read(
            b"\x1b]5522;type=read:id=r1:name=YXBw;dGV4dC9wbGFpbiAu\x1b\\",
            |request| {
                request.reply(
                    Ok(&[
                        ClipboardReplyContent::new(b"text/plain", b"hello"),
                        // Not requested, so it must be ignored.
                        ClipboardReplyContent::new(b"image/png", b"png"),
                    ]),
                    &[
                        ClipboardMime::new(b"text/plain"),
                        ClipboardMime::new(b"image/png"),
                        // Not UTF-8: libghostty lists MIME types verbatim,
                        // so this must reach the program unchanged.
                        ClipboardMime::new(b"\xFF"),
                    ],
                    false,
                );
            },
        );
        assert_eq!(
            seen,
            [SeenRead {
                location: Some(ClipboardLocation::Standard),
                mimes: vec![b"text/plain".to_vec()],
                list: true,
                name: b"app".to_vec(),
                ..SeenRead::default()
            }]
        );
        assert_eq!(
            output,
            concat!(
                "\x1b]5522;type=read:status=OK:id=r1\x1b\\",
                // "text/plain image/png \xFF\n"
                "\x1b]5522;type=read:status=DATA:id=r1:mime=Lg==;dGV4dC9wbGFpbiBpbWFnZS9wbmcg/wo=\x1b\\",
                "\x1b]5522;type=read:status=DATA:id=r1:mime=dGV4dC9wbGFpbg==;aGVsbG8=\x1b\\",
                "\x1b]5522;type=read:status=DONE:id=r1\x1b\\",
            )
            .as_bytes()
        );

        // Errors map to protocol statuses, and a missing reply is EPERM.
        let (_, output) = clipboard_read(
            b"\x1b]5522;type=read:id=r2;dGV4dC9wbGFpbg==\x1b\\",
            |request| request.reply(Err(ClipboardReadError::Busy), &[], false),
        );
        assert_eq!(output, b"\x1b]5522;type=read:status=EBUSY:id=r2\x1b\\");
        let (_, output) = clipboard_read(
            b"\x1b]5522;type=read:id=r3;dGV4dC9wbGFpbg==\x1b\\",
            |_request| {},
        );
        assert_eq!(output, b"\x1b]5522;type=read:status=EPERM:id=r3\x1b\\");
    }

    #[test]
    fn osc5522_clipboard_read_remembers_password_grants() {
        // Two reads from "app" with the same password ("secret"): remembering
        // the first grant makes the second arrive already granted. Per the
        // spec, a password without a name is no password.
        let read = "\x1b]5522;type=read:id=a:name=YXBw:pw=c2VjcmV0;dGV4dC9wbGFpbg==\x1b\\";
        let (seen, _) = clipboard_read(read.repeat(2).as_bytes(), |request| {
            request.reply(
                Ok(&[ClipboardReplyContent::new(b"text/plain", b"hi")]),
                &[],
                true,
            );
        });
        let grants: Vec<_> = seen.iter().map(|s| (s.can_remember, s.granted)).collect();
        assert_eq!(grants, [(true, false), (true, true)]);
    }

    #[test]
    fn clipboard_read_empty_reply_contents() {
        // Empty MIME and data strings hand C dangling non-null pointers with
        // length 0. An empty representation produces no DATA packets, which
        // is how the protocol reports an unavailable type.
        let (_, output) = clipboard_read(
            b"\x1b]5522;type=read:id=e;dGV4dC9wbGFpbg==\x1b\\",
            |request| {
                request.reply(
                    Ok(&[
                        ClipboardReplyContent::new(b"", b""),
                        ClipboardReplyContent::new(b"text/plain", b""),
                    ]),
                    &[ClipboardMime::new(b"")],
                    false,
                );
            },
        );
        assert_eq!(
            output,
            concat!(
                "\x1b]5522;type=read:status=OK:id=e\x1b\\",
                "\x1b]5522;type=read:status=DONE:id=e\x1b\\",
            )
            .as_bytes()
        );

        // OSC 52 has no text representation to use, so the clipboard is empty.
        let (_, output) = clipboard_read(b"\x1b]52;c;?\x1b\\", |request| {
            request.reply(Ok(&[ClipboardReplyContent::new(b"", b"")]), &[], false);
        });
        assert_eq!(output, b"\x1b]52;c;\x1b\\");
    }

    #[test]
    fn osc5522_clipboard_read_errors() {
        for (error, status) in [
            (ClipboardReadError::Denied, "EPERM"),
            (ClipboardReadError::Unsupported, "ENOSYS"),
            (ClipboardReadError::Busy, "EBUSY"),
            (ClipboardReadError::IoError, "EIO"),
        ] {
            let (_, output) = clipboard_read(
                b"\x1b]5522;type=read:id=x;dGV4dC9wbGFpbg==\x1b\\",
                |request| request.reply(Err(error), &[], false),
            );
            let expected = format!("\x1b]5522;type=read:status={status}:id=x\x1b\\");
            assert_eq!(output, expected.as_bytes(), "{error:?}");
        }
    }

    #[test]
    fn clipboard_read_primary_location() {
        let reply = |request: ClipboardRead<'_>| {
            request.reply(
                Ok(&[ClipboardReplyContent::new(b"text/plain", b"hello")]),
                &[],
                false,
            );
        };

        let (seen, output) = clipboard_read(b"\x1b]52;p;?\x1b\\", reply);
        assert_eq!(seen[0].location, Some(ClipboardLocation::Primary));
        assert_eq!(output, b"\x1b]52;p;aGVsbG8=\x1b\\");

        let (seen, output) = clipboard_read(
            b"\x1b]5522;type=read:loc=primary:id=p;dGV4dC9wbGFpbg==\x1b\\",
            reply,
        );
        assert_eq!(seen[0].location, Some(ClipboardLocation::Primary));
        assert_eq!(
            output,
            concat!(
                "\x1b]5522;type=read:status=OK:loc=primary:id=p\x1b\\",
                "\x1b]5522;type=read:status=DATA:id=p:mime=dGV4dC9wbGFpbg==;aGVsbG8=\x1b\\",
                "\x1b]5522;type=read:status=DONE:id=p\x1b\\",
            )
            .as_bytes()
        );
    }

    // The next test reaches simdutf, which libghostty-vt DLLs on Windows
    // used to crash in, since their C++ global constructors never ran. Pure
    // ASCII never reaches it.
    #[test]
    fn clipboard_write_decodes_base64() {
        let written = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 3).unwrap();
        terminal
            .on_clipboard_write(|_, request| {
                // Record rather than assert here: a panic in a callback
                // aborts the whole test binary.
                for content in request.contents() {
                    written.borrow_mut().push(content.data.to_vec());
                }
                request.reply(Ok(()), false);
            })
            .unwrap();

        terminal.vt_write(b"\x1b]52;c;//4=\x1b\\");
        assert_eq!(*written.borrow(), [vec![0xff, 0xfe]]);
    }

    #[test]
    fn osc5522_clipboard_read_targets_only() {
        // A read for only the targets listing (payload "." = "Lg==") carries
        // no MIME types, so libghostty hands the callback `mimes = NULL,
        // mimes_len = 0`. The first read records a grant for the password;
        // the listing-only read must still arrive ungranted (it is
        // prompt-exempt and never consults grants), and the follow-up data
        // read still sees the grant.
        let remember = "\x1b]5522;type=read:id=a:name=YXBw:pw=c2VjcmV0;dGV4dC9wbGFpbg==\x1b\\";
        let list = "\x1b]5522;type=read:id=l:name=YXBw:pw=c2VjcmV0;Lg==\x1b\\";
        let input = [remember, list, remember].concat();
        let (seen, output) = clipboard_read(input.as_bytes(), |request| {
            // Empty slices hand C dangling non-null pointers with length 0.
            request.reply(Ok(&[]), &[], true);
        });
        let observed: Vec<_> = seen
            .iter()
            .map(|s| (s.mimes.len(), s.list, s.granted))
            .collect();
        assert_eq!(
            observed,
            [(1, false, false), (0, true, false), (1, false, true)]
        );

        // An empty listing is a DATA packet with no payload.
        let listing = concat!(
            "\x1b]5522;type=read:status=OK:id=l\x1b\\",
            "\x1b]5522;type=read:status=DATA:id=l:mime=Lg==\x1b\\",
            "\x1b]5522;type=read:status=DONE:id=l\x1b\\",
        );
        let output = String::from_utf8(output).expect("responses are ASCII");
        assert!(output.contains(listing), "{output:?}");
    }

    #[test]
    fn clipboard_write_max_bytes_bounds_kitty_writes() {
        let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
        let default = terminal.clipboard_write_max_bytes().unwrap();
        assert_eq!(default, 64 * 1024 * 1024);
        terminal.set_clipboard_write_max_bytes(Some(4)).unwrap();
        assert_eq!(terminal.clipboard_write_max_bytes().unwrap(), 4);
        terminal.set_clipboard_write_max_bytes(None).unwrap();
        assert_eq!(terminal.clipboard_write_max_bytes().unwrap(), default);

        // A 5-byte ("Hello") OSC 5522 write transaction.
        let write = concat!(
            "\x1b]5522;type=write:id=c1\x1b\\",
            "\x1b]5522;type=wdata:mime=dGV4dC9wbGFpbg==;SGVsbA==\x1b\\",
            "\x1b]5522;type=wdata:mime=dGV4dC9wbGFpbg==;bw==\x1b\\",
            "\x1b]5522;type=wdata\x1b\\",
        );
        let run = |limit| {
            let written = RefCell::new(Vec::new());
            let output = RefCell::new(Vec::new());
            let mut terminal = Terminal::new(8, 2).expect("terminal should initialize");
            terminal
                .on_pty_write(|_term, bytes| output.borrow_mut().extend_from_slice(bytes))
                .unwrap()
                .on_clipboard_write(|_term, request| {
                    written
                        .borrow_mut()
                        .extend(request.contents().map(|c| c.data.to_vec()));
                    request.reply(Ok(()), false);
                })
                .unwrap()
                .set_clipboard_write_max_bytes(Some(limit))
                .unwrap();
            terminal.vt_write(write.as_bytes());
            drop(terminal);
            (output.into_inner(), written.into_inner())
        };

        // Exceeding the limit fails the transaction and never reaches the callback.
        let (output, written) = run(4);
        assert_eq!(output, b"\x1b]5522;type=write:status=EFBIG:id=c1\x1b\\");
        assert!(written.is_empty());

        let (output, written) = run(5);
        assert_eq!(output, b"\x1b]5522;type=write:status=DONE:id=c1\x1b\\");
        assert_eq!(written, [b"Hello".to_vec()]);
    }

    #[test]
    fn render_hold_reports_start_and_end_in_pairs() {
        // Mirrors upstream's "set render_hold callback" test in
        // src/terminal/c/terminal.zig.
        let events = RefCell::new(Vec::new());
        let take = || std::mem::take(&mut *events.borrow_mut());
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .on_render_hold(|_term, held| events.borrow_mut().push(held))
            .expect("callback should register");

        // A set during a hold and a reset without a hold are ignored.
        terminal.vt_write(b"\x1b[?2026h\x1b[?2026hA\x1b[?2026l\x1b[?2026l");
        assert_eq!(take(), [true, false]);

        // Neither a reset nor a resize reports anything without a hold.
        terminal.reset();
        terminal
            .resize(100, 30, 8, 16)
            .expect("resize should succeed");
        assert_eq!(take(), []);

        // Reset and resize end an active hold, and so does a resize that
        // keeps the dimensions: upstream turns synchronized output off
        // before it checks whether the grid size changed.
        terminal.vt_write(b"\x1b[?2026h");
        terminal.reset();
        terminal.reset();
        terminal.vt_write(b"\x1b[?2026h");
        terminal
            .resize(80, 24, 8, 16)
            .expect("resize should succeed");
        terminal.vt_write(b"\x1b[?2026h");
        terminal
            .resize(80, 24, 8, 16)
            .expect("resize should succeed");
        assert_eq!(take(), [true, false, true, false, true, false]);
        assert!(!terminal.mode(Mode::SYNC_OUTPUT).unwrap());

        // A resize that fails leaves the mode, and so the hold, in place.
        terminal.vt_write(b"\x1b[?2026h");
        assert!(terminal.resize(0, 24, 8, 16).is_err());
        assert!(terminal.mode(Mode::SYNC_OUTPUT).unwrap());
        terminal.vt_write(b"\x1b[?2026l");
        assert_eq!(take(), [true, false]);

        // Changing the mode ourselves, e.g. when a hold times out, is never
        // reported in either direction. The hold is simply over, so the
        // callback sees `true` without a matching `false`.
        terminal.vt_write(b"\x1b[?2026h");
        terminal
            .set_mode(Mode::SYNC_OUTPUT, false)
            .expect("mode should be settable");
        assert!(!terminal.mode(Mode::SYNC_OUTPUT).unwrap());
        terminal
            .set_mode(Mode::SYNC_OUTPUT, true)
            .expect("mode should be settable")
            .set_mode(Mode::SYNC_OUTPUT, false)
            .expect("mode should be settable");
        assert_eq!(take(), [true]);
        // ...and the program can start a new hold afterwards.
        terminal.vt_write(b"\x1b[?2026h");
        assert_eq!(take(), [true]);
    }

    /// Read the text of the first row of a render state snapshot.
    fn first_row_text(snapshot: &crate::render::Snapshot<'_, '_>) -> Result<String> {
        let mut rows = crate::render::RowIterator::new()?;
        let mut cells = crate::render::CellIterator::new()?;
        let mut row_iter = rows.update(snapshot)?;
        let Some(row) = row_iter.next() else {
            return Ok(String::new());
        };
        let mut cell_iter = cells.update(row)?;
        let mut text = String::new();
        while let Some(cell) = cell_iter.next() {
            text.extend(cell.graphemes()?);
        }
        Ok(text)
    }

    #[test]
    fn render_hold_captures_frame_before_hold() {
        // A renderer that refreshes its render state whenever a hold starts
        // or ends, recording the first row it captured each time. Failures
        // are recorded as `None` instead of panicking, since a panic here
        // would abort the whole test binary.
        let render_state =
            RefCell::new(RenderState::new().expect("render state should initialize"));
        let frames = RefCell::new(Vec::new());
        let take = || std::mem::take(&mut *frames.borrow_mut());
        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");
        terminal
            .on_render_hold(|term, held| {
                let text = render_state.try_borrow_mut().ok().and_then(|mut state| {
                    let snapshot = state.update(term).ok()?;
                    first_row_text(&snapshot).ok()
                });
                frames.borrow_mut().push((held, text));
            })
            .expect("callback should register");

        // The hold starts before `B` is processed, even though it's in the
        // same write, so the captured frame only has `A`.
        terminal.vt_write(b"A\x1b[?2026hB");
        assert_eq!(take(), [(true, Some("A".to_owned()))]);
        // The terminal itself has moved on, though.
        {
            let mut state = render_state.borrow_mut();
            let snapshot = state.update(&terminal).expect("render state should update");
            assert_eq!(first_row_text(&snapshot).unwrap(), "AB");
        }
        terminal.vt_write(b"\x1b[?2026l");
        assert_eq!(take(), [(false, Some("AB".to_owned()))]);

        // Updating the render state also works when the hold ends from
        // `resize` and `reset`, which invoke the callback while the outer
        // call holds `&mut Terminal`.
        terminal.vt_write(b"\x1b[?2026hC");
        terminal
            .resize(100, 30, 8, 16)
            .expect("resize should succeed");
        assert_eq!(
            take(),
            [
                (true, Some("AB".to_owned())),
                (false, Some("ABC".to_owned()))
            ]
        );
        terminal.vt_write(b"\x1b[?2026hD");
        terminal.reset();
        assert_eq!(
            take(),
            [(true, Some("ABC".to_owned())), (false, Some(String::new()))]
        );
    }

    #[test]
    fn unknown_sequences_are_reported() {
        use osc::Terminator::{Bel, St};

        let seen = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal
            .on_unknown_sequence(|_term, sequence| {
                // Record rather than assert: a panic in a callback aborts the
                // whole test binary.
                seen.borrow_mut().push(match sequence {
                    UnknownSequence::Apc {
                        content, truncated, ..
                    } => (content.to_vec(), truncated, None),
                    UnknownSequence::Osc {
                        content,
                        truncated,
                        terminator,
                        ..
                    } => (content.to_vec(), truncated, Some(terminator)),
                });
            })
            .unwrap();
        // Registration alone leaves reporting disabled.
        terminal.vt_write(b"\x1b_unknown\x1b\\");
        terminal.set_unknown_max_bytes(4).unwrap();
        terminal.vt_write(b"\x1b_unknown\x1b\\");
        terminal.vt_write(b"\x1b]7400;status=busy\x07");
        terminal.set_unknown_max_bytes(64).unwrap();
        terminal.vt_write(b"\x1b_unknown\x1b\\");
        terminal.vt_write(b"\x1b]7400;status=busy\x1b\\");
        // Numbers libghostty-vt implements are never reported, even with no
        // callback for them: OSC 7501 is program status, OSC 2 the title.
        terminal.vt_write(b"\x1b]7501;state=working\x1b\\");
        terminal.vt_write(b"\x1b]2;title\x07");
        drop(terminal);
        assert_eq!(
            seen.into_inner(),
            [
                (b"unkn".to_vec(), true, None),
                (b"7400".to_vec(), true, Some(Bel)),
                (b"unknown".to_vec(), false, None),
                (b"7400;status=busy".to_vec(), false, Some(St)),
            ]
        );
    }

    /// Send an OSC 2 title sequence, then verify `term.title()` returns the
    /// correct value inside the `on_title_changed` callback.
    #[test]
    fn title_changed_callback_returns_correct_title() {
        // The callback bound on `on_title_changed` is `'cb`, not `'static`,
        // so the closure can borrow stack locals directly – no Rc needed.
        let captured_title: RefCell<Vec<u8>> = RefCell::new(Vec::new());
        let callback_count: Cell<usize> = Cell::new(0);

        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");

        terminal
            .on_title_changed(|term| {
                callback_count.set(callback_count.get() + 1);
                let title = term
                    .title()
                    .expect("title() should succeed inside callback");
                *captured_title.borrow_mut() = title.to_owned();
            })
            .expect("callback should register");

        // OSC 2 (set title) should invoke on_title_changed.
        terminal.vt_write(b"\x1b]2;Hello Effects\x1b\\");
        assert_eq!(callback_count.get(), 1);
        assert_eq!(*captured_title.borrow(), b"Hello Effects");

        // A second title change should fire the callback again.
        terminal.vt_write(b"\x1b]2;Second Title\x1b\\");
        assert_eq!(callback_count.get(), 2);
        assert_eq!(*captured_title.borrow(), b"Second Title");
    }

    /// Send an OSC 7 current-directory sequence, then verify `term.pwd()`
    /// returns the correct value inside the `on_pwd_changed` callback.
    #[test]
    fn pwd_changed_callback_returns_correct_pwd() {
        let captured_pwd: RefCell<Vec<u8>> = RefCell::new(Vec::new());
        let callback_count: Cell<usize> = Cell::new(0);

        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");

        terminal
            .on_pwd_changed(|term| {
                callback_count.set(callback_count.get() + 1);
                let pwd = term.pwd().expect("pwd() should succeed inside callback");
                *captured_pwd.borrow_mut() = pwd.to_owned();
            })
            .expect("callback should register");

        terminal.vt_write(b"\x1b]7;file://localhost/tmp/project\x1b\\");
        assert_eq!(callback_count.get(), 1);
        assert_eq!(*captured_pwd.borrow(), b"file://localhost/tmp/project");

        terminal.vt_write(b"\x1b]7;file://localhost/tmp/other\x1b\\");
        assert_eq!(callback_count.get(), 2);
        assert_eq!(*captured_pwd.borrow(), b"file://localhost/tmp/other");

        // libghostty stores the pwd bytes as-is, so non-UTF-8 input must come
        // back verbatim instead of being rejected.
        terminal.vt_write(b"\x1b]7;file://localhost/tmp/\xff\x1b\\");
        assert_eq!(callback_count.get(), 3);
        assert_eq!(*captured_pwd.borrow(), b"file://localhost/tmp/\xff");
    }

    /// Only a full reset sent by the program (RIS) fires `on_reset`, and the
    /// terminal has already reset itself by then.
    #[test]
    fn reset_callback_fires_only_on_ris() {
        let callback_count: Cell<usize> = Cell::new(0);
        let title_in_callback: RefCell<Option<Vec<u8>>> = RefCell::new(None);

        let mut terminal = Terminal::new(80, 24).expect("terminal should initialize");

        terminal
            .on_reset(|term| {
                callback_count.set(callback_count.get() + 1);
                // Record rather than assert here: a panic in a callback
                // aborts the whole test binary.
                *title_in_callback.borrow_mut() = term.title().ok().map(<[u8]>::to_owned);
            })
            .expect("callback should register");

        // Set a title first so we can tell whether RIS already cleared it
        // when the callback ran, as upstream documents.
        terminal.vt_write(b"\x1b]2;Before Reset\x1b\\");
        assert_eq!(terminal.title().unwrap(), b"Before Reset");
        // A progress report and a program status report fire their own
        // callbacks, and RIS fires both again just before this one. Sending
        // them here catches this callback being registered under either
        // sibling's option instead.
        terminal.vt_write(b"\x1b]9;4;1;50\x1b\\");
        terminal.vt_write(b"\x1b]7501;state=working:id=build\x1b\\");
        assert_eq!(callback_count.get(), 0);
        terminal.vt_write(b"\x1bc");
        assert_eq!(callback_count.get(), 1);
        assert_eq!(*title_in_callback.borrow(), Some(Vec::new()));

        // `vt_write_until` feeds the same stream, so it fires too.
        terminal.vt_write(b"\x1b");
        terminal.vt_write_until(b"c", Boundary::GROUND).unwrap();
        assert_eq!(callback_count.get(), 2);

        // A soft reset (DECSTR) doesn't count as a full reset.
        terminal.vt_write(b"\x1b[!p");
        assert_eq!(callback_count.get(), 2);

        // Upstream's `ghostty_terminal_reset` resets the terminal directly
        // and only the stream handling `ESC c` calls the callback.
        terminal.reset();
        assert_eq!(callback_count.get(), 2);
    }

    /// OSC 9 and OSC 777 carry the program's raw bytes, which libghostty
    /// passes on unvalidated. They used to be exposed as `&str`.
    #[test]
    fn desktop_notifications_are_not_assumed_to_be_utf8() {
        let seen = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 3).unwrap();
        terminal
            .on_desktop_notification(|_, notification| {
                seen.borrow_mut()
                    .push((notification.title().to_vec(), notification.body().to_vec()));
            })
            .unwrap();
        terminal.vt_write(b"\x1b]9;\xff\xfe\x07");
        terminal.vt_write(b"\x1b]777;notify;\xc3\x28;ok\x07");
        assert_eq!(
            *seen.borrow(),
            [
                (b"".to_vec(), b"\xff\xfe".to_vec()),
                (b"\xc3\x28".to_vec(), b"ok".to_vec()),
            ]
        );
    }

    /// What a program status callback observed about one report, copied
    /// out since the report is only valid during the callback.
    ///
    /// `state` and `kind` are `None` when the raw value didn't convert, so
    /// that shows up as a failed comparison outside the callback.
    #[derive(Debug, PartialEq, Eq)]
    struct SeenStatus {
        state: Option<ProgramStatusState>,
        kind: Option<ProgramStatusKind>,
        progress: Option<u8>,
        id: Vec<u8>,
        app: Vec<u8>,
        title: Vec<u8>,
        message: Vec<u8>,
    }

    impl SeenStatus {
        /// A report with only `state` set, as upstream fills in every
        /// value the program didn't send.
        fn bare(state: ProgramStatusState) -> Self {
            Self {
                state: Some(state),
                kind: Some(ProgramStatusKind::None),
                progress: None,
                id: Vec::new(),
                app: Vec::new(),
                title: Vec::new(),
                message: Vec::new(),
            }
        }
    }

    /// Feed `input` to a terminal with a program status callback, returning
    /// the reports it saw and what the terminal wrote back to the pty.
    ///
    /// Assertions happen outside the callback: a panic inside it would
    /// abort the whole test binary instead of failing the test.
    fn program_status(input: &[u8]) -> (Vec<SeenStatus>, Vec<u8>) {
        let seen = RefCell::new(Vec::new());
        let output = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 3).unwrap();
        terminal
            .on_pty_write(|_, bytes| output.borrow_mut().extend_from_slice(bytes))
            .unwrap()
            .on_program_status(|_, report| {
                seen.borrow_mut().push(SeenStatus {
                    state: report.state().ok(),
                    kind: report.kind().ok(),
                    progress: report.progress(),
                    id: report.id().to_vec(),
                    app: report.app().to_vec(),
                    title: report.title().to_vec(),
                    message: report.message().to_vec(),
                });
            })
            .unwrap();
        terminal.vt_write(input);
        drop(terminal);
        (seen.into_inner(), output.into_inner())
    }

    #[test]
    fn program_status_delivers_every_field() {
        // title is "Plan" and msg is "Apply?", both base64.
        let (seen, output) = program_status(
            b"\x1b]7501;state=blocked:kind=permission:progress=40:id=a/b\
              :app=terraform:title=UGxhbg==:msg=QXBwbHk/\x07",
        );
        assert_eq!(
            seen,
            [SeenStatus {
                state: Some(ProgramStatusState::Blocked),
                kind: Some(ProgramStatusKind::Permission),
                progress: Some(40),
                id: b"a/b".to_vec(),
                app: b"terraform".to_vec(),
                title: b"Plan".to_vec(),
                message: b"Apply?".to_vec(),
            }]
        );
        assert!(output.is_empty());
    }

    #[test]
    fn program_status_leaves_unsent_values_empty() {
        // The example from the upstream docs: no progress means the work is
        // indeterminate, and no id means the root record.
        let (seen, _) = program_status(
            b"\x1b]7501;state=blocked:kind=permission:app=terraform:msg=QXBwbHk/\x1b\\",
        );
        assert_eq!(
            seen,
            [SeenStatus {
                kind: Some(ProgramStatusKind::Permission),
                app: b"terraform".to_vec(),
                message: b"Apply?".to_vec(),
                ..SeenStatus::bare(ProgramStatusState::Blocked)
            }]
        );

        let (seen, _) = program_status(b"\x1b]7501;state=done\x1b\\");
        assert_eq!(seen, [SeenStatus::bare(ProgramStatusState::Done)]);
    }

    #[test]
    fn program_status_progress_and_kind_follow_state() {
        let (seen, _) = program_status(
            &[
                // 0 is a real percentage, not "no progress".
                &b"\x1b]7501;state=working:progress=0\x07"[..],
                b"\x1b]7501;state=working:progress=100\x07",
                // Out of range reads as no percentage.
                b"\x1b]7501;state=working:progress=101\x07",
                // Progress and kind only count for the states that use them.
                b"\x1b]7501;state=idle:progress=50:kind=question\x07",
                b"\x1b]7501;state=blocked:kind=auth\x07",
                // A kind libghostty doesn't know reads as none.
                b"\x1b]7501;state=blocked:kind=telepathy\x07",
            ]
            .concat(),
        );
        assert_eq!(
            seen,
            [
                SeenStatus {
                    progress: Some(0),
                    ..SeenStatus::bare(ProgramStatusState::Working)
                },
                SeenStatus {
                    progress: Some(100),
                    ..SeenStatus::bare(ProgramStatusState::Working)
                },
                SeenStatus::bare(ProgramStatusState::Working),
                SeenStatus::bare(ProgramStatusState::Idle),
                SeenStatus {
                    kind: Some(ProgramStatusKind::Auth),
                    ..SeenStatus::bare(ProgramStatusState::Blocked)
                },
                SeenStatus::bare(ProgramStatusState::Blocked),
            ]
        );
    }

    #[test]
    fn program_status_drops_invalid_reports() {
        // An unknown state and a bad id each discard the whole report.
        let (seen, output) =
            program_status(b"\x1b]7501;state=sleeping\x07\x1b]7501;state=idle:id=a//b\x07");
        assert!(seen.is_empty());
        assert!(output.is_empty());
    }

    #[test]
    fn program_status_clears_every_record_on_full_reset() {
        let (seen, _) = program_status(b"\x1b]7501;state=error:id=x\x07\x1bc");
        assert_eq!(
            seen,
            [
                SeenStatus {
                    id: b"x".to_vec(),
                    ..SeenStatus::bare(ProgramStatusState::Error)
                },
                SeenStatus::bare(ProgramStatusState::Clear),
            ]
        );
    }

    #[test]
    fn program_status_is_not_cleared_by_reset() {
        // Upstream's `ghostty_terminal_reset` only resets the terminal; the
        // clear report comes from the stream handling `ESC c`.
        let seen = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 3).unwrap();
        terminal
            .on_program_status(|_, report| {
                seen.borrow_mut()
                    .push((report.state().ok(), report.id().to_vec()));
            })
            .unwrap();

        terminal.vt_write(b"\x1b]7501;state=error:id=x\x07");
        assert_eq!(
            seen.take(),
            [(Some(ProgramStatusState::Error), b"x".to_vec())]
        );

        terminal.reset();
        assert!(seen.borrow().is_empty());

        terminal.vt_write(b"\x1bc");
        assert_eq!(seen.take(), [(Some(ProgramStatusState::Clear), Vec::new())]);
    }

    #[test]
    fn program_status_query_is_answered_only_with_a_callback() {
        const REPLY: &[u8] =
            b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth";

        // The reply uses the same terminator as the query, and the query
        // itself is not a report.
        let (seen, output) = program_status(b"\x1b]7501;?\x1b\\\x1b]7501;?\x07");
        assert!(seen.is_empty());
        assert_eq!(output, [REPLY, b"\x1b\\", REPLY, b"\x07"].concat());

        // Without the callback the query goes unanswered, so the program
        // sees the protocol as unsupported.
        let output = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 3).unwrap();
        terminal
            .on_pty_write(|_, bytes| output.borrow_mut().extend_from_slice(bytes))
            .unwrap();
        terminal.vt_write(b"\x1b]7501;?\x1b\\");
        drop(terminal);
        assert!(output.into_inner().is_empty());
    }

    /// What a semantic prompt callback observed about one event, copied out
    /// since the event is only valid during the callback.
    ///
    /// `kind` and `prompt_kind` are `None` when the raw value didn't
    /// convert, so that shows up as a failed comparison outside the
    /// callback.
    #[derive(Debug, PartialEq, Eq)]
    struct SeenPrompt {
        kind: Option<SemanticPromptKind>,
        prompt_kind: Option<PromptKind>,
        exit_code: Option<i32>,
        command: Vec<u8>,
        error: Vec<u8>,
    }

    impl SeenPrompt {
        /// An event with only `kind` set, as upstream leaves every other
        /// field zero or empty (and `prompt_kind` primary) unless the kind
        /// carries it.
        fn bare(kind: SemanticPromptKind) -> Self {
            Self {
                kind: Some(kind),
                prompt_kind: Some(PromptKind::Primary),
                exit_code: None,
                command: Vec::new(),
                error: Vec::new(),
            }
        }
    }

    /// Feed `input` to a terminal with a semantic prompt callback, returning
    /// the events it saw.
    ///
    /// Assertions happen outside the callback: a panic inside it would
    /// abort the whole test binary instead of failing the test.
    fn semantic_prompt(input: &[u8]) -> Vec<SeenPrompt> {
        let seen = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 3).unwrap();
        terminal
            .on_semantic_prompt(|_, event| {
                seen.borrow_mut().push(SeenPrompt {
                    kind: event.kind().ok(),
                    prompt_kind: event.prompt_kind().ok(),
                    exit_code: event.exit_code(),
                    command: event.command().to_vec(),
                    error: event.error().to_vec(),
                });
            })
            .unwrap();
        terminal.vt_write(input);
        drop(terminal);
        seen.into_inner()
    }

    #[test]
    fn semantic_prompt_reports_each_command_step() {
        // Mirrors upstream's "set semantic_prompt callback" test in
        // src/terminal/c/terminal.zig, plus a few cases from the stream
        // handler test: OSC 133 A/P/N start a prompt (P's `k=` picks right,
        // continuation or secondary, and an unknown kind falls back to
        // primary), B/I start input, C starts output with a percent-encoded
        // `cmdline_url`, and D ends the command with an optional exit code
        // and `err=`.
        let seen = semantic_prompt(
            b"\x1b]133;A\x07\
              \x1b]133;P;k=r\x07\
              \x1b]133;P;k=c\x07\
              \x1b]133;P;k=s\x07\
              \x1b]133;P;k=x\x07\
              \x1b]133;N\x07\
              \x1b]133;B\x07\
              \x1b]133;I\x07\
              \x1b]133;C;cmdline_url=ls%20-la\x07\
              \x1b]133;D;0\x07\
              \x1b]133;D;-1;err=boom\x07\
              \x1b]133;D\x07",
        );
        assert_eq!(
            seen,
            [
                SeenPrompt::bare(SemanticPromptKind::PromptStart),
                SeenPrompt {
                    prompt_kind: Some(PromptKind::Right),
                    ..SeenPrompt::bare(SemanticPromptKind::PromptStart)
                },
                SeenPrompt {
                    prompt_kind: Some(PromptKind::Continuation),
                    ..SeenPrompt::bare(SemanticPromptKind::PromptStart)
                },
                SeenPrompt {
                    prompt_kind: Some(PromptKind::Secondary),
                    ..SeenPrompt::bare(SemanticPromptKind::PromptStart)
                },
                SeenPrompt::bare(SemanticPromptKind::PromptStart),
                SeenPrompt::bare(SemanticPromptKind::PromptStart),
                SeenPrompt::bare(SemanticPromptKind::InputStart),
                SeenPrompt::bare(SemanticPromptKind::InputStart),
                SeenPrompt {
                    command: b"ls -la".to_vec(),
                    ..SeenPrompt::bare(SemanticPromptKind::OutputStart)
                },
                // 0 is a real exit code, distinct from "not reported".
                SeenPrompt {
                    exit_code: Some(0),
                    ..SeenPrompt::bare(SemanticPromptKind::CommandEnd)
                },
                SeenPrompt {
                    exit_code: Some(-1),
                    error: b"boom".to_vec(),
                    ..SeenPrompt::bare(SemanticPromptKind::CommandEnd)
                },
                SeenPrompt::bare(SemanticPromptKind::CommandEnd),
            ]
        );
    }

    #[test]
    fn semantic_prompt_command_is_raw_bytes() {
        // Percent-decoding can produce any bytes, and upstream makes no
        // UTF-8 promise, so non-UTF-8 command lines come through untouched.
        // An undecodable command line and a fresh line alone (OSC 133 L,
        // layout rather than a command step) are covered too.
        let seen = semantic_prompt(
            b"\x1b]133;C;cmdline_url=%ff%fe\x07\
              \x1b]133;C;cmdline='bad\x07\
              \x1b]133;L\x07",
        );
        assert_eq!(
            seen,
            [
                SeenPrompt {
                    command: b"\xff\xfe".to_vec(),
                    ..SeenPrompt::bare(SemanticPromptKind::OutputStart)
                },
                SeenPrompt::bare(SemanticPromptKind::OutputStart),
            ]
        );
    }

    /// Feed `input` to an 80x24 terminal with a resize window callback,
    /// returning the requests it saw and the terminal's size afterwards as
    /// `(cols, rows)`.
    ///
    /// Assertions happen outside the callback: a panic inside it would
    /// abort the whole test binary instead of failing the test.
    fn resize_window(input: &[u8]) -> (Vec<ResizeWindow>, (u16, u16)) {
        let seen = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal
            .on_resize_window(|_, request| seen.borrow_mut().push(request))
            .unwrap();
        terminal.vt_write(input);
        let size = (terminal.cols().unwrap(), terminal.rows().unwrap());
        drop(terminal);
        (seen.into_inner(), size)
    }

    #[test]
    fn resize_window_delivers_rows_then_columns() {
        let (seen, size) = resize_window(b"\x1b[8;30;100t");
        // Asymmetric values catch a swapped rows/columns mapping.
        assert_eq!(
            seen,
            [ResizeWindow::Grid {
                rows: NonZeroU16::new(30),
                columns: NonZeroU16::new(100),
            }]
        );
        // The request alone doesn't resize the terminal.
        assert_eq!(size, (80, 24));
    }

    #[test]
    fn resize_window_reports_omitted_and_zero_params_as_none() {
        // Upstream can't tell an explicit 0 from an omitted parameter, and
        // both mean "keep the current size" for that dimension.
        let (seen, size) = resize_window(b"\x1b[8;;120t\x1b[8;30t\x1b[8t\x1b[8;0;0t");
        assert_eq!(
            seen,
            [
                ResizeWindow::Grid {
                    rows: None,
                    columns: NonZeroU16::new(120),
                },
                ResizeWindow::Grid {
                    rows: NonZeroU16::new(30),
                    columns: None,
                },
                ResizeWindow::Grid {
                    rows: None,
                    columns: None,
                },
                ResizeWindow::Grid {
                    rows: None,
                    columns: None,
                },
            ]
        );
        assert_eq!(size, (80, 24));
    }

    #[test]
    fn resize_window_from_raw_skips_unknown_tags() {
        let mut raw = ffi::TerminalResizeWindow {
            tag: ffi::TerminalResizeWindowTag::GRID,
            ..Default::default()
        };
        raw.value.grid = ffi::TerminalResizeWindowGrid {
            rows: 30,
            columns: 100,
        };
        assert_eq!(
            // SAFETY: `raw` is a live local with the tag and `grid` written.
            unsafe { ResizeWindow::from_raw(&raw const raw) },
            Some(ResizeWindow::Grid {
                rows: NonZeroU16::new(30),
                columns: NonZeroU16::new(100),
            })
        );

        // A kind added upstream later has no Rust value. That the trampoline
        // then skips the closure is checked by the miri mock in
        // `miri_soundness::resize_window_skips_unknown_tags`.
        raw.tag = ffi::TerminalResizeWindowTag::GRID + 1;
        // SAFETY: Ditto; an unknown tag makes `from_raw` read nothing else.
        assert_eq!(unsafe { ResizeWindow::from_raw(&raw const raw) }, None);
    }

    #[test]
    fn unknown_sequence_from_raw_skips_unknown_tags_and_terminators() {
        let apc_bytes = *b"Gf=100";
        let osc_bytes = *b"7400;status=busy";
        let mut raw = ffi::TerminalUnknownSequence {
            tag: ffi::TerminalUnknownSequenceTag::APC,
            ..Default::default()
        };
        raw.value.apc = ffi::TerminalUnknownStringSequence {
            truncated: true,
            content: ffi::String {
                ptr: apc_bytes.as_ptr(),
                len: apc_bytes.len(),
            },
        };
        assert_eq!(
            // SAFETY: `raw` is a live local with the tag and `apc` written,
            // and its content points into `apc_bytes`, which outlives it.
            unsafe { UnknownSequence::from_raw(&raw const raw) },
            Some(UnknownSequence::Apc {
                content: b"Gf=100",
                truncated: true,
            })
        );

        raw.tag = ffi::TerminalUnknownSequenceTag::OSC;
        raw.value.osc = ffi::TerminalUnknownOscSequence {
            truncated: false,
            content: ffi::String {
                ptr: osc_bytes.as_ptr(),
                len: osc_bytes.len(),
            },
            terminator: ffi::OscTerminator::ST,
        };
        assert_eq!(
            // SAFETY: Ditto, with `osc` written and pointing into `osc_bytes`.
            unsafe { UnknownSequence::from_raw(&raw const raw) },
            Some(UnknownSequence::Osc {
                content: b"7400;status=busy",
                truncated: false,
                terminator: osc::Terminator::St,
            })
        );

        // A terminator added upstream later has no Rust value, so the
        // sequence is skipped rather than given an invented one.
        raw.value.osc.terminator = ffi::OscTerminator::BEL + 1;
        // SAFETY: Ditto
        assert_eq!(unsafe { UnknownSequence::from_raw(&raw const raw) }, None);

        // A kind added upstream later has no Rust value either. That the
        // trampoline then skips the closure is checked by the miri mock in
        // `miri_soundness::unknown_sequence_skips_unknown_tags_and_terminators`.
        raw.tag = ffi::TerminalUnknownSequenceTag::OSC + 1;
        // SAFETY: Ditto; an unknown tag makes `from_raw` read nothing else.
        assert_eq!(unsafe { UnknownSequence::from_raw(&raw const raw) }, None);
    }
}

/// Soundness regression tests for
/// <https://github.com/Uzaaft/libghostty-rs/issues/74>.
///
/// These tests are gated on `cfg(miri)` because they construct the exact
/// shapes the C API produces and feed them into the safe wrappers, which was
/// UB before the wrappers stopped building slices and `&str` from unvalidated
/// FFI input. The module also holds tests that need the mock C API to produce
/// shapes real libghostty can't yet (e.g. unknown upstream tags). Run with:
///
/// ```sh
/// cargo +nightly miri test -p libghostty-vt miri_soundness
/// ```
#[cfg(all(test, miri))]
mod miri_soundness {
    use super::*;
    use crate::Terminal;
    use std::{mem::ManuallyDrop, num::NonZeroU16};

    /// Upstream may add resize request kinds without an ABI break, and asks
    /// callbacks to ignore tags they don't handle. The trampoline must skip
    /// the closure for such a tag rather than abort (e.g. by `expect`ing the
    /// conversion inside `extern "C"`), while still delivering known tags.
    #[test]
    fn resize_window_skips_unknown_tags() {
        let seen = std::cell::RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal
            .on_resize_window(|_, request| seen.borrow_mut().push(request))
            .unwrap();
        // `GRID + 1`, then `GRID` as the positive control.
        terminal.vt_write(b"resize:unknown");
        terminal.vt_write(b"resize:grid");
        drop(terminal);
        assert_eq!(
            seen.into_inner(),
            [ResizeWindow::Grid {
                rows: NonZeroU16::new(30),
                columns: NonZeroU16::new(100),
            }]
        );
    }

    /// Upstream may add unknown-sequence kinds and OSC terminators without an
    /// ABI break, and fills the union with `lib.TaggedUnion.cval`, which
    /// leaves bytes outside the active field (the whole union, for a kind
    /// without a payload) undefined. The trampoline must read only the tag
    /// and then the field it names, and skip the closure for a kind or
    /// terminator it doesn't know, while still delivering known ones.
    #[test]
    fn unknown_sequence_skips_unknown_tags_and_terminators() {
        let seen = std::cell::RefCell::new(Vec::new());
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal
            .on_unknown_sequence(|_, sequence| {
                // Copy the content out; it is only borrowed for the callback.
                seen.borrow_mut().push(match sequence {
                    UnknownSequence::Apc { content, truncated } => {
                        (None, content.to_vec(), truncated)
                    }
                    UnknownSequence::Osc {
                        content,
                        truncated,
                        terminator,
                    } => (Some(terminator), content.to_vec(), truncated),
                });
            })
            .unwrap();
        // `OSC + 1` and an OSC ending in `BEL + 1`, then APC and OSC as the
        // positive controls.
        terminal.vt_write(b"unknown:future");
        terminal.vt_write(b"unknown:osc-badterm");
        terminal.vt_write(b"unknown:apc");
        terminal.vt_write(b"unknown:osc");
        drop(terminal);
        assert_eq!(
            seen.into_inner(),
            [
                (None, b"payload".to_vec(), true),
                (Some(osc::Terminator::Bel), b"payload".to_vec(), false),
            ]
        );
    }

    /// The C trampoline declares `contents: ?[*]const ClipboardContent` and
    /// sends `contents = NULL, contents_len = 0` for a write carrying no
    /// representations (e.g. OSC 52 with an empty payload, the documented
    /// "clear the clipboard" shape). `slice::from_raw_parts` requires a
    /// non-null pointer even at length zero, so `contents()` used to be UB
    /// here; it must yield an empty iterator so hosts can observe "clear".
    #[test]
    fn clipboard_write_with_no_representations() {
        let raw = ffi::ClipboardWrite {
            location: ffi::ClipboardLocation::STANDARD,
            contents: std::ptr::null(),
            contents_len: 0,
            ..ffi::ClipboardWrite::default()
        };
        // SAFETY: `raw` outlives the borrow, matching the callback contract.
        let write = unsafe { ClipboardWrite::from_raw(&raw) };
        assert_eq!(write.contents().count(), 0);
    }

    /// OSC 52 payloads are base64-decoded arbitrary bytes ("binary-safe" per
    /// the C header), but `ClipboardContent` used to expose them as `&str`
    /// built with `str::from_utf8_unchecked` in the sys crate, so decoding
    /// the invalid `&str` entered unreachable code in std's UTF-8 decoder.
    /// The data is exposed as `&[u8]` now; check it round-trips verbatim.
    #[test]
    fn clipboard_content_with_non_utf8_data() {
        // OSC 52 payload "//4=" base64-decodes to FF FE, which is not UTF-8.
        let data = [0xFF_u8, 0xFE];
        let raw = ffi::ClipboardContent {
            mime: ffi::String::from("text/plain"),
            data: ffi::String {
                ptr: data.as_ptr(),
                len: data.len(),
            },
        };
        // SAFETY: `data` outlives the borrow, matching the callback contract.
        let content = unsafe { ClipboardContent::from_raw(&raw) };
        assert_eq!(content.mime, b"text/plain");
        assert_eq!(content.data, &data);
    }

    /// The C API doesn't promise UTF-8 for `mime` either. It used to be
    /// validated into a `&str`, with non-UTF-8 values replaced by
    /// `application/octet-stream`; it is raw bytes now, so check a non-UTF-8
    /// mime string comes back verbatim instead of being substituted.
    #[test]
    fn clipboard_content_with_non_utf8_mime() {
        let mime = [0xFF_u8, 0xFE];
        let data = *b"hello";
        let raw = ffi::ClipboardContent {
            mime: ffi::String {
                ptr: mime.as_ptr(),
                len: mime.len(),
            },
            data: ffi::String {
                ptr: data.as_ptr(),
                len: data.len(),
            },
        };
        // SAFETY: `mime` and `data` outlive the borrow, matching the
        // callback contract.
        let content = unsafe { ClipboardContent::from_raw(&raw) };
        assert_eq!(content.mime, &mime);
        assert_eq!(content.data, b"hello");
    }
}
