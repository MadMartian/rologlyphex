use crate::config::LayoutInfo;
use crate::debug_log;
use glib::timeout_add_local_once;
use gtk4::prelude::*;
use gtk4::{Align, Application, ApplicationWindow, Box, CssProvider, FlowBox, Label, Orientation};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

const WINDOW_MARGIN: i32 = 20;

struct MonitorGeometry {
    x: i32,
    y: i32,
    width: i32,
    #[allow(dead_code)]
    height: i32,
}

impl MonitorGeometry {
    fn fallback() -> Self {
        MonitorGeometry { x: 0, y: 0, width: 1920, height: 1080 }
    }
}

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct OverlayWindow {
    window: ApplicationWindow,
    layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
    dismiss_generation: Arc<AtomicU64>,
    dismiss_timeout_ms: u64,
    window_width: i32,
    target_x: Rc<Cell<i32>>,
    target_y: Rc<Cell<i32>>,
}

impl OverlayWindow {
    pub fn new(app: &Application, layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>, dismiss_timeout_ms: u64, window_width: i32) -> Self {
        // Verify display is available and running on X11 backend
        let display = gdk4::Display::default().unwrap_or_else(|| {
            eprintln!("Error: no display available. Ensure a graphical session is running.");
            std::process::exit(1);
        });
        let backend = display.type_().name();
        if !backend.contains("X11") {
            eprintln!("Error: Rologlyphex requires an X11 display server (detected backend: {}).", backend);
            eprintln!("If running under Wayland, try: GDK_BACKEND=x11 rologlyphex ...");
            std::process::exit(1);
        }

        let window = ApplicationWindow::new(app);

        window.set_decorated(false);
        window.set_resizable(false);
        window.set_default_size(window_width, -1);
        window.set_title(Some("rologlyphex"));
        window.set_focusable(false);
        window.set_focus_on_click(false);
        window.set_can_focus(false);
        window.set_can_target(false);

        let mon = Self::find_rightmost_monitor();
        let target_x = Rc::new(Cell::new(mon.x + mon.width - window_width - WINDOW_MARGIN));
        let target_y = Rc::new(Cell::new(mon.y + WINDOW_MARGIN));

        // Content
        let content_box = Box::new(Orientation::Vertical, 12);
        content_box.set_margin_top(16);
        content_box.set_margin_bottom(16);
        content_box.set_margin_start(16);
        content_box.set_margin_end(16);
        content_box.set_halign(Align::Fill);
        content_box.set_valign(Align::Start);

        let title_label = Label::new(None);
        title_label.add_css_class("overlay-title");
        title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title_label.set_hexpand(true);
        content_box.append(&title_label);

        let legend_box = Box::new(Orientation::Vertical, 12);
        legend_box.set_halign(Align::Fill);
        legend_box.set_valign(Align::Start);
        content_box.append(&legend_box);

        window.set_child(Some(&content_box));
        Self::apply_css();
        window.set_visible(false);

        let tx = target_x.get();
        let ty = target_y.get();
        WidgetExt::realize(&window); // Force realization to set X11 properties before first map

        if let Some(surface) = window.surface() {
            // Set empty input region -- clicks pass through like Patrick Swayze
            let empty_region = cairo::Region::create();
            surface.set_input_region(&empty_region);
        }
        Self::configure_x11_properties(&window, tx, ty);

        let dismiss_generation = Arc::new(AtomicU64::new(0));

        // Subscribe to monitor changes for hotplug support
        let tx_hotplug = target_x.clone();
        let ty_hotplug = target_y.clone();
        let ww = window_width;
        let monitors = display.monitors();
        monitors.connect_items_changed(move |_, _, _, _| {
            let mon = Self::find_rightmost_monitor();
            let new_x = mon.x + mon.width - ww - WINDOW_MARGIN;
            let new_y = mon.y + WINDOW_MARGIN;
            tx_hotplug.set(new_x);
            ty_hotplug.set(new_y);
            debug_log!("[🐛DEBUG] Monitor change detected, new overlay position: ({}, {})", new_x, new_y);
        });

        OverlayWindow {
            window,
            layout_map,
            dismiss_generation,
            dismiss_timeout_ms,
            window_width,
            target_x,
            target_y,
        }
    }

    pub fn show_layout(&self, layout_name: &str) {
        if let Some(child) = self.window.child() {
            if let Ok(content_box) = child.downcast::<Box>() {
                self.update_layout_content(&content_box, layout_name);
                let (_, natural_h, _, _) = content_box.measure(Orientation::Vertical, self.window_width);
                if natural_h > 0 {
                    self.window.set_size_request(self.window_width, natural_h);
                }
            }
        }

        self.window.set_visible(true);

        // Re-apply position and properties every show
        let win = self.window.clone();
        let tx = self.target_x.get();
        let ty = self.target_y.get();
        Self::configure_x11_properties(&win, tx, ty);

        self.reset_dismiss_timer();
    }

    /// Set X11 window type, state, and position directly via Xlib
    fn configure_x11_properties(win: &ApplicationWindow, target_x: i32, target_y: i32) {
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

    fn update_layout_content(&self, content_box: &Box, layout_name: &str) {
        let layouts = match self.layout_map.read() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error: layout map lock poisoned: {}", e);
                return;
            }
        };
        let available_width = self.window_width - 32; // 16px margin each side

        if let Some(layout_info) = layouts.get(layout_name) {
            if let Some(title_label) = content_box.first_child().and_then(|w| w.downcast::<Label>().ok()) {
                title_label.set_label(&layout_info.label);

                // Try full size first
                title_label.remove_css_class("overlay-title-small");
                title_label.add_css_class("overlay-title");
                let (min_w, natural, _, _) = title_label.measure(Orientation::Horizontal, -1);

                debug_log!("[🐛DEBUG] Title '{}': natural={}px, available={}px, min={}px",
                    layout_info.label, natural, available_width, min_w);

                // If it overflows, fall back to 2/3 font size
                if natural > available_width {
                    title_label.remove_css_class("overlay-title");
                    title_label.add_css_class("overlay-title-small");
                    let (_, natural2, _, _) = title_label.measure(Orientation::Horizontal, -1);
                    debug_log!("[🐛DEBUG] Title reduced: natural={}px at 48px font", natural2);
                }
            }

            if let Some(legend_widget) = content_box
                .first_child()
                .and_then(|w| w.next_sibling())
                .and_then(|w| w.downcast::<Box>().ok())
            {
                while let Some(child) = legend_widget.first_child() {
                    legend_widget.remove(&child);
                }

                for group in &layout_info.groups {
                    let group_box = Box::new(Orientation::Vertical, 4);
                    group_box.add_css_class("overlay-group");
                    group_box.set_valign(Align::Start);

                    if let Some(label_text) = &group.label {
                        let group_label = Label::new(Some(label_text));
                        group_label.add_css_class("overlay-group-label");
                        group_label.set_halign(Align::Start);
                        group_box.append(&group_label);
                    }

                    let buttons_box = FlowBox::new();
                    buttons_box.set_valign(Align::Start);
                    buttons_box.set_halign(Align::Fill);
                    buttons_box.set_selection_mode(gtk4::SelectionMode::None);
                    buttons_box.set_min_children_per_line(1);
                    buttons_box.set_max_children_per_line(20);

                    for button in &group.buttons {
                        let lbl = Label::new(Some(&button.display));
                        lbl.add_css_class("overlay-legend");
                        buttons_box.append(&lbl);
                    }
                    group_box.append(&buttons_box);
                    legend_widget.append(&group_box);
                }
            }
        } else {
            debug_log!("[🐛DEBUG] Warning: Layout name '{}' not found in map, clearing overlay content", layout_name);
            if let Some(title_label) = content_box.first_child().and_then(|w| w.downcast::<Label>().ok()) {
                title_label.set_label("");
            }
            if let Some(legend_widget) = content_box
                .first_child()
                .and_then(|w| w.next_sibling())
                .and_then(|w| w.downcast::<Box>().ok())
            {
                while let Some(child) = legend_widget.first_child() {
                    legend_widget.remove(&child);
                }
            }
        }
    }

    fn reset_dismiss_timer(&self) {
        let gen = self.dismiss_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let gen_ref = self.dismiss_generation.clone();
        let window_clone = self.window.clone();

        timeout_add_local_once(
            std::time::Duration::from_millis(self.dismiss_timeout_ms),
            move || {
                if gen_ref.load(Ordering::SeqCst) == gen {
                    window_clone.set_visible(false);
                }
            }
        );
    }

    fn find_rightmost_monitor() -> MonitorGeometry {
        let display = match gdk4::Display::default() {
            Some(d) => d,
            None => {
                eprintln!("Error: no display available for monitor enumeration; using 1920x1080 fallback");
                return MonitorGeometry::fallback();
            }
        };
        let monitors = display.monitors();

        let mut best: Option<MonitorGeometry> = None;
        let mut max_x = i32::MIN;

        for i in 0..monitors.n_items() {
            if let Some(obj) = monitors.item(i) {
                if let Ok(monitor) = obj.downcast::<gdk4::Monitor>() {
                    let geom = monitor.geometry();
                    let x_end = geom.x() + geom.width();
                    if x_end > max_x {
                        max_x = x_end;
                        best = Some(MonitorGeometry {
                            x: geom.x(),
                            y: geom.y(),
                            width: geom.width(),
                            height: geom.height(),
                        });
                    }
                }
            }
        }

        best.unwrap_or_else(|| {
            eprintln!("Error: no monitors found; using 1920x1080 fallback");
            MonitorGeometry::fallback()
        })
    }

    fn apply_css() {
        let css = "
            window {
                background-color: rgba(0, 0, 0, 0.85);
                border-radius: 8px;
            }
            .overlay-title {
                color: white;
                font-weight: bold;
                font-size: 72px;
            }
            .overlay-title-small {
                color: white;
                font-weight: bold;
                font-size: 48px;
            }
            .overlay-legend {
                background-color: rgba(100, 150, 255, 0.3);
                color: white;
                border-radius: 4px;
                padding: 8px 12px;
                font-size: 84px;
            }
            .overlay-group {
                border: 2px solid rgba(255, 255, 255, 0.2);
                border-radius: 6px;
                padding: 8px;
                margin: 4px;
            }
            .overlay-group-label {
                color: rgba(255, 255, 255, 0.7);
                font-size: 24px;
                font-weight: bold;
                margin-bottom: 4px;
                margin-left: 4px;
            }
            flowboxchild {
                padding: 0;
                margin: 0;
            }
        ";

        let provider = CssProvider::new();
        provider.load_from_data(css);

        let display = match gdk4::Display::default() {
            Some(d) => d,
            None => {
                eprintln!("Error: no display available for CSS provider");
                return;
            }
        };

        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
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

// FFI declarations for GDK X11 backend
extern "C" {
    fn gdk_x11_surface_get_xid(surface: *mut std::ffi::c_void) -> x11::xlib::Window;
    fn gdk_x11_display_get_xdisplay(display: *mut std::ffi::c_void) -> *mut x11::xlib::Display;
}
