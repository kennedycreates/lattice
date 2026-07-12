//! Minimal, dependency-free logging helpers.
//!
//! All diagnostics go to stderr with a consistent `[lattice][level]` tag so
//! they are easy to grep and distinguish from GTK/GLib output. These replace
//! the ad-hoc `eprintln!` calls scattered across the crate. For failures that
//! the user should also see, pair `log_err!` with a status-bar message.

/// Log an error-level message to stderr.
#[macro_export]
macro_rules! log_err {
    ($($arg:tt)*) => {
        eprintln!("[lattice][error] {}", format_args!($($arg)*))
    };
}

/// Log a warning-level message to stderr.
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        eprintln!("[lattice][warn] {}", format_args!($($arg)*))
    };
}

/// Log an informational message to stderr.
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        eprintln!("[lattice][info] {}", format_args!($($arg)*))
    };
}
