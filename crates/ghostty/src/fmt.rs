use std::{marker::PhantomData, ptr::NonNull};

use crate::{
    error::{Error, from_result},
    ffi,
    terminal::Terminal,
};

pub struct Formatter<'t, 'alloc> {
    ptr: NonNull<ffi::GhosttyFormatter>,
    _terminal: PhantomData<&'t Terminal<'alloc>>,
}

pub struct FormatterOptions {
    format: FormatterFormat,

    /// Whether to trim trailing whitespace on non-blank lines.
    trim: bool,

    /// Whether to unwrap soft-wrapped lines.
    unwrap: bool,
}

impl<'t, 'alloc> Formatter<'t, 'alloc> {
    pub fn new(terminal: &'t Terminal<'alloc>, opts: FormatterOptions) -> Result<Self, Error> {
        let mut raw: ffi::GhosttyFormatter_ptr = std::ptr::null_mut();
        let result = unsafe {
            ffi::ghostty_formatter_terminal_new(
                std::ptr::null(),
                &mut raw,
                terminal.as_raw(),
                opts.into(),
            )
        };
        from_result(result)?;
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _terminal: PhantomData,
        })
    }
}

impl<'t, 'alloc> Drop for Formatter<'t, 'alloc> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.ptr.as_ptr()) }
    }
}

impl From<FormatterOptions> for ffi::GhosttyFormatterTerminalOptions {
    fn from(value: FormatterOptions) -> Self {
        Self {
            size: std::mem::size_of::<ffi::GhosttyFormatterTerminalOptions>(),
            emit: value.format.into(),
            trim: value.trim,
            extra: ffi::GhosttyFormatterTerminalExtra::default(),
            unwrap: value.unwrap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatterFormat {
    Plain,
    Vt,
    Html,
}

impl From<FormatterFormat> for ffi::GhosttyFormatterFormat {
    fn from(value: FormatterFormat) -> ffi::GhosttyFormatterFormat {
        match value {
            FormatterFormat::Plain => ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_PLAIN,
            FormatterFormat::Vt => ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_VT,
            FormatterFormat::Html => ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_HTML,
        }
    }
}
