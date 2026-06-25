// Process-global Xlib error handlers (addresses former ISSUES.md "I").
//
// Xlib's default protocol-error handler calls exit(), bypassing Rust's panic hook and
// leaving the daemon socket stale. The daemon makes X requests that can legitimately fail
// without being fatal — most importantly XGrabKey, which raises BadAccess if another client
// already holds a key. We install a handler that logs the error and returns (non-fatal), and
// an IO-error handler that performs an orderly exit when the X server connection is lost.
//
// Xlib error handlers are process-global (not per-Display), so installing once covers every
// connection: GTK's GDK display, the XTyper display, and the key-grab display. GTK installs
// its own handler during initialization; call install() after that (and/or from the grab
// thread once its Display is open) so ours wins.

use std::os::raw::c_int;
use x11::xlib;

fn error_code_name(code: u8) -> &'static str {
    match code {
        1 => "BadRequest",
        2 => "BadValue",
        3 => "BadWindow",
        4 => "BadPixmap",
        5 => "BadAtom",
        6 => "BadCursor",
        7 => "BadFont",
        8 => "BadMatch",
        9 => "BadDrawable",
        10 => "BadAccess",
        11 => "BadAlloc",
        13 => "BadGC",
        17 => "BadLength",
        18 => "BadImplementation",
        _ => "X error",
    }
}

/// Non-fatal protocol error handler: log details and return 0 so Xlib does not exit().
unsafe extern "C" fn error_handler(
    _display: *mut xlib::Display,
    event: *mut xlib::XErrorEvent,
) -> c_int {
    if let Some(e) = event.as_ref() {
        eprintln!(
            "[X error] {} (code {}), request {}.{}, resource 0x{:x}, serial {}",
            error_code_name(e.error_code),
            e.error_code,
            e.request_code,
            e.minor_code,
            e.resourceid,
            e.serial
        );
    } else {
        eprintln!("[X error] (null error event)");
    }
    0
}

/// Fatal I/O error handler: the X server connection is gone. Exit cleanly rather than
/// letting Xlib's default handler abort.
unsafe extern "C" fn io_error_handler(_display: *mut xlib::Display) -> c_int {
    eprintln!("[X IO error] connection to the X server was lost; shutting down");
    std::process::exit(1);
}

/// Install the process-global Xlib error handlers. Idempotent; safe to call more than once.
pub fn install() {
    unsafe {
        xlib::XSetErrorHandler(Some(error_handler));
        xlib::XSetIOErrorHandler(Some(io_error_handler));
    }
}
