// X11 window-manager property configuration (EWMH) via direct Xlib FFI, split out of
// overlay.rs. Sets the overlay windows as above-everything, sticky, focus-less notification
// windows on all desktops, and moves them into position.

use crate::debug_log;
use gtk4::prelude::*;
use gtk4::ApplicationWindow;

/// Set the window's X11 type/state/desktop properties and move it to `(target_x, target_y)`.
pub fn configure(win: &ApplicationWindow, target_x: i32, target_y: i32) {
    let surface = match win.surface() {
        Some(s) => s,
        None => return,
    };

    let xid = unsafe { gdk_x11_surface_get_xid(surface.as_ptr() as *mut _) };
    if xid == 0 {
        debug_log!("[🐛DEBUG] Failed to get X11 window ID");
        return;
    }

    let display = match gdk4::Display::default() {
        Some(d) => d,
        None => {
            eprintln!("Error: display became unavailable during X11 property configuration");
            return;
        }
    };

    let xdisplay = unsafe { gdk_x11_display_get_xdisplay(display.as_ptr() as *mut _) };
    if xdisplay.is_null() {
        eprintln!("Error: failed to get X11 display pointer (is the X11 backend active?)");
        return;
    }

    // Intern atoms
    let (wm_type, type_notification, wm_state, state_above,
         state_skip_taskbar, state_skip_pager, state_sticky,
         wm_user_time, wm_desktop, kde_activities, xa_string) = unsafe {
        (
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_WINDOW_TYPE\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_WINDOW_TYPE_NOTIFICATION\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_STATE\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_STATE_ABOVE\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_STATE_SKIP_TASKBAR\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_STATE_SKIP_PAGER\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_STATE_STICKY\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_USER_TIME\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_NET_WM_DESKTOP\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"_KDE_NET_WM_ACTIVITIES\0".as_ptr() as *const _, 0),
            x11::xlib::XInternAtom(xdisplay, b"STRING\0".as_ptr() as *const _, 0),
        )
    };

    // Set window type and state properties
    unsafe {
        let type_data: [x11::xlib::Atom; 1] = [type_notification];
        x11::xlib::XChangeProperty(xdisplay, xid, wm_type,
            x11::xlib::XA_ATOM, 32, x11::xlib::PropModeReplace,
            type_data.as_ptr() as *const u8, 1);

        let state_data: [x11::xlib::Atom; 4] =
            [state_above, state_skip_taskbar, state_skip_pager, state_sticky];
        x11::xlib::XChangeProperty(xdisplay, xid, wm_state,
            x11::xlib::XA_ATOM, 32, x11::xlib::PropModeReplace,
            state_data.as_ptr() as *const u8, 4);

        let user_time: [std::ffi::c_long; 1] = [0];
        x11::xlib::XChangeProperty(xdisplay, xid, wm_user_time,
            x11::xlib::XA_CARDINAL, 32, x11::xlib::PropModeReplace,
            user_time.as_ptr() as *const u8, 1);

        let hints = x11::xlib::XAllocWMHints();
        if !hints.is_null() {
            (*hints).flags = x11::xlib::InputHint;
            (*hints).input = x11::xlib::False;
            x11::xlib::XSetWMHints(xdisplay, xid, hints);
            x11::xlib::XFree(hints as *mut _);
        }

        let all_desktops: [std::ffi::c_long; 1] = [0xFFFFFFFFu32 as std::ffi::c_long];
        x11::xlib::XChangeProperty(xdisplay, xid, wm_desktop,
            x11::xlib::XA_CARDINAL, 32, x11::xlib::PropModeReplace,
            all_desktops.as_ptr() as *const u8, 1);

        x11::xlib::XChangeProperty(xdisplay, xid, kde_activities,
            xa_string, 8, x11::xlib::PropModeReplace,
            std::ptr::null(), 0);
    }

    // Send EWMH ClientMessages to root (required for already-mapped windows)
    let root = unsafe { x11::xlib::XDefaultRootWindow(xdisplay) };
    let mask = x11::xlib::SubstructureRedirectMask | x11::xlib::SubstructureNotifyMask;

    unsafe {
        let mut ev: x11::xlib::XEvent = std::mem::zeroed();
        ev.type_ = x11::xlib::ClientMessage;
        ev.client_message.window = xid;
        ev.client_message.message_type = wm_desktop;
        ev.client_message.format = 32;
        let p = &mut ev.client_message.data as *mut _ as *mut std::ffi::c_long;
        std::ptr::write(p.add(0), 0xFFFFFFFFu32 as std::ffi::c_long);
        std::ptr::write(p.add(1), 1);
        x11::xlib::XSendEvent(xdisplay, root, 0, mask, &mut ev);
    }

    send_wm_add(xdisplay, root, mask, xid, wm_state, state_above, state_sticky);
    send_wm_add(xdisplay, root, mask, xid, wm_state, state_skip_taskbar, state_skip_pager);

    unsafe {
        x11::xlib::XMoveWindow(xdisplay, xid, target_x, target_y);
        x11::xlib::XFlush(xdisplay);
    }

    debug_log!("[🐛DEBUG] Configured X11 window 0x{:x} at ({}, {})", xid, target_x, target_y);
}

/// Send a _NET_WM_STATE ClientMessage to the root window to add two state atoms.
/// Required by EWMH for windows that are already mapped.
fn send_wm_add(
    xdisplay: *mut x11::xlib::Display,
    root: x11::xlib::Window,
    mask: std::os::raw::c_long,
    xid: x11::xlib::Window,
    wm_state: x11::xlib::Atom,
    one: x11::xlib::Atom,
    two: x11::xlib::Atom,
) {
    unsafe {
        let mut ev: x11::xlib::XEvent = std::mem::zeroed();
        ev.type_ = x11::xlib::ClientMessage;
        ev.client_message.window = xid;
        ev.client_message.message_type = wm_state;
        ev.client_message.format = 32;
        let p = &mut ev.client_message.data as *mut _ as *mut std::ffi::c_long;
        std::ptr::write(p.add(0), 1); // _NET_WM_STATE_ADD
        std::ptr::write(p.add(1), one as std::ffi::c_long);
        std::ptr::write(p.add(2), two as std::ffi::c_long);
        std::ptr::write(p.add(3), 1); // source indication
        x11::xlib::XSendEvent(xdisplay, root, 0, mask, &mut ev);
    }
}

// FFI declarations for the GDK X11 backend.
extern "C" {
    fn gdk_x11_surface_get_xid(surface: *mut std::ffi::c_void) -> x11::xlib::Window;
    fn gdk_x11_display_get_xdisplay(display: *mut std::ffi::c_void) -> *mut x11::xlib::Display;
}
