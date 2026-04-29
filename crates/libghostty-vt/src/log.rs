//! Logging functionality.
use std::sync::RwLock;

use crate::{
    error::{Error, Result},
    ffi,
};

/// Callback type for logging.
///
/// When installed, internal library log messages are delivered through
/// this callback instead of being discarded. The embedder is responsible
/// for formatting and routing log output.
///
/// When the log is unscoped (default scope), scope has zero length.
///
/// The callback must be safe to call from any thread.
///
/// See [`set_logger`] for more details.
pub trait Logger: Send + Sync + 'static {
    /// Log a message with the given level and scope.
    fn log(&self, level: Level, scope: &str, message: &str);
}

/// Built-in log callback that writes to stderr.
///
/// Formats each message as `[level](scope): message\n`.
///
/// Can be passed directly to [`set_logger`]:
///
/// ```
/// use libghostty_vt::log;
/// log::set_logger(Some(Box::new(log::LogStderr)));
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LogStderr;
impl Logger for LogStderr {
    fn log(&self, level: Level, scope: &str, message: &str) {
        unsafe {
            ffi::ghostty_sys_log_stderr(
                std::ptr::null_mut(),
                level.into(),
                scope.as_ptr(),
                scope.len(),
                message.as_ptr(),
                message.len(),
            );
        }
    }
}

/// Adapt a `log` implementation to be used by `libghostty`.
///
/// `libghostty` log scopes are translated directly to `log`'s metadata
/// target, and `log` implementation can choose to filter specific
/// `libghostty` logs to be emitted.
#[cfg(feature = "log")]
impl<L: log::Log + 'static> Logger for L {
    fn log(&self, level: Level, scope: &str, message: &str) {
        let level = match level {
            Level::Error => log::Level::Error,
            Level::Warning => log::Level::Warn,
            Level::Info => log::Level::Info,
            Level::Debug => log::Level::Debug,
        };
        let args = format_args!("{message}");
        let record = log::Record::builder()
            .level(level)
            .target(scope)
            .args(args)
            .build();

        log::Log::log(&self, &record);
    }
}

/// Log severity levels for the log callback.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, int_enum::IntEnum)]
#[repr(u32)]
#[non_exhaustive]
#[expect(missing_docs, reason = "missing upstream docs")]
pub enum Level {
    Error = ffi::SysLogLevel::ERROR,
    Warning = ffi::SysLogLevel::WARNING,
    Info = ffi::SysLogLevel::INFO,
    Debug = ffi::SysLogLevel::DEBUG,
}

static LOGGER: RwLock<Option<Box<dyn Logger>>> = RwLock::new(None);

/// Set the log callback.
///
/// When set, internal library log messages are delivered to this
/// callback. When cleared (`None` value), log messages are silently
/// discarded.
///
/// Use [`LogStderr`] as a convenience callback that writes formatted messages
/// to stderr.
///
/// Which log levels are emitted depends on the build mode of the library and
/// is not configurable at runtime. Debug builds emit all levels (debug and
/// above). Release builds emit info and above; debug-level messages are
/// compiled out entirely and will never reach the callback.
///
/// ```
/// use libghostty_vt::log;
/// log::set_logger(None).unwrap();
/// ```
pub fn set_logger(f: Option<Box<dyn Logger>>) -> Result<()> {
    unsafe extern "C" fn callback(
        _userdata: *mut std::ffi::c_void,
        level: ffi::SysLogLevel::Type,
        scope: *const u8,
        scope_len: usize,
        message: *const u8,
        message_len: usize,
    ) {
        let scope = unsafe { std::slice::from_raw_parts(scope, scope_len) };
        let Ok(scope) = std::str::from_utf8(scope) else {
            return;
        };
        let message = unsafe { std::slice::from_raw_parts(message, message_len) };
        let Ok(message) = std::str::from_utf8(message) else {
            return;
        };
        let Ok(level) = Level::try_from(level) else {
            return;
        };

        let Ok(log) = LOGGER.read() else {
            return;
        };
        let Some(log) = log.as_deref() else {
            return;
        };
        log.log(level, scope, message);
    }

    // Write out the matches here to coerce function items into function
    // pointers, and trait impls into boxed trait objects. Yes, this is
    // the simplest way to do so.
    let ptr: ffi::SysLogFn = match f {
        None => None,
        Some(_) => Some(callback),
    };
    {
        let Ok(mut logger) = LOGGER.write() else {
            return Err(Error::InvalidValue);
        };
        *logger = f;
    }
    crate::sys_set(
        ffi::SysOption::GHOSTTY_SYS_OPT_LOG,
        ptr.map_or(std::ptr::null(), |p| p as *const std::ffi::c_void),
    )
}
