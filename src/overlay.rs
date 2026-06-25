use crate::config::LayoutInfo;
use crate::debug_log;
use glib::timeout_add_local_once;
use gtk4::prelude::*;
use gtk4::{Align, Application, ApplicationWindow, Box, CssProvider, FlowBox, Image, Label, Orientation};

/// App icon (a rotary deck-selector), embedded so the daemon needs no install-path lookup.
const APP_ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");

/// Build the shared overlay header: the app icon followed by "Rologlyphex!", small and
/// centered at the top. Returned as a horizontal box to prepend to a window's content.
fn make_header() -> Box {
    let header = Box::new(Orientation::Horizontal, 6);
    header.set_halign(Align::Center);
    header.set_margin_bottom(10);
    if let Some(icon) = app_icon() {
        header.append(&icon);
    }
    let label = Label::new(Some("Rologlyphex!"));
    label.add_css_class("overlay-header-label");
    header.append(&label);
    header
}

/// Render the embedded SVG into a small GTK image, or None if the SVG pixbuf loader is
/// unavailable (the header then shows the title text alone).
fn app_icon() -> Option<Image> {
    use gtk4::gdk_pixbuf::prelude::PixbufLoaderExt;
    let loader = gtk4::gdk_pixbuf::PixbufLoader::with_type("svg").ok()?;
    loader.set_size(22, 22);
    loader.write(APP_ICON_SVG).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
    let image = Image::from_paintable(Some(&texture));
    image.set_pixel_size(22);
    Some(image)
}
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::monitor::{self, Corner};
use crate::wmprops;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct OverlayWindow {
    window: ApplicationWindow,
    /// Separate "Please Wait" window, shown during a lazy-mode keymap remap.
    please_wait: ApplicationWindow,
    /// Title and legend widgets, held directly so content updates don't depend on child order
    /// (the header is prepended ahead of them).
    title_label: Label,
    legend_box: Box,
    layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
    dismiss_generation: Arc<AtomicU64>,
    dismiss_timeout_ms: u64,
    window_width: i32,
    /// User preference for which monitor to display on (connector/model/index), if any.
    monitor_pref: Option<String>,
    /// Corner of the target monitor the overlay aligns to.
    corner: Corner,
}

impl OverlayWindow {
    pub fn new(
        app: &Application,
        layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
        dismiss_timeout_ms: u64,
        window_width: i32,
        monitor_pref: Option<String>,
        corner_pref: Option<String>,
    ) -> Self {
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

        // Resolve the corner preference once; default to top-right (legacy behaviour).
        let corner = corner_pref.as_deref().and_then(Corner::parse).unwrap_or_else(|| {
            if let Some(c) = &corner_pref {
                eprintln!("Warning: unrecognized corner '{}', using top-right. \
                    Valid values: top-left, top-right, bottom-left, bottom-right", c);
            }
            Corner::TopRight
        });

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
        title_label.set_xalign(0.5); // layer title is centered

        let legend_box = Box::new(Orientation::Vertical, 12);
        legend_box.set_halign(Align::Fill);
        legend_box.set_valign(Align::Start);

        content_box.append(&title_label);
        content_box.append(&legend_box);

        // The app header (icon + "Rologlyphex!") hugs the same corner inside the window that the
        // window hugs on its display: horizontal via halign, vertical via top/bottom placement.
        let header = make_header();
        header.set_halign(corner.header_halign());
        if corner.is_top() {
            content_box.prepend(&header);
        } else {
            content_box.append(&header);
        }

        window.set_child(Some(&content_box));
        Self::apply_css();
        window.set_visible(false);

        // Initial position with a zero-height estimate; the window is invisible until the
        // first show_layout, which recomputes the position from the measured height. The
        // target monitor and corner are re-resolved on every show, so monitor hotplug needs
        // no separate subscription.
        let mon = monitor::select_monitor(&monitor_pref);
        let (tx, ty) = corner.position(&mon, window_width, 0);
        WidgetExt::realize(&window); // Force realization to set X11 properties before first map

        if let Some(surface) = window.surface() {
            // Set empty input region -- clicks pass through like Patrick Swayze
            let empty_region = cairo::Region::create();
            surface.set_input_region(&empty_region);
        }
        wmprops::configure(&window, tx, ty);

        let dismiss_generation = Arc::new(AtomicU64::new(0));

        // Separate "Please Wait" indicator window, used in lazy remap mode while the typist
        // rebuilds the keymap for a newly-entered layer (slow on keycode-heavy systems).
        let please_wait = ApplicationWindow::new(app);
        please_wait.set_decorated(false);
        please_wait.set_resizable(false);
        please_wait.set_default_size(320, -1);
        please_wait.set_title(Some("rologlyphex-wait"));
        please_wait.set_focusable(false);
        please_wait.set_focus_on_click(false);
        please_wait.set_can_focus(false);
        please_wait.set_can_target(false);
        let pw_box = Box::new(Orientation::Vertical, 0);
        pw_box.set_margin_top(20);
        pw_box.set_margin_bottom(20);
        pw_box.set_margin_start(28);
        pw_box.set_margin_end(28);
        pw_box.set_halign(Align::Center);
        let pw_label = Label::new(Some("Please Wait…"));
        pw_label.add_css_class("please-wait");
        pw_box.append(&pw_label);
        pw_box.prepend(&make_header());
        please_wait.set_child(Some(&pw_box));
        // Concrete size; set_default_size is ignored once realized (ANTI-PATTERNS #16).
        please_wait.set_size_request(320, 90);
        please_wait.set_visible(false);
        WidgetExt::realize(&please_wait);
        if let Some(surface) = please_wait.surface() {
            let empty_region = cairo::Region::create();
            surface.set_input_region(&empty_region);
        }

        OverlayWindow {
            window,
            please_wait,
            title_label,
            legend_box,
            layout_map,
            dismiss_generation,
            dismiss_timeout_ms,
            window_width,
            monitor_pref,
            corner,
        }
    }

    /// Show the "Please Wait" window, centered across the whole desktop (the union of all
    /// monitors), so it's distinct from the corner-aligned layer overlay.
    pub fn show_please_wait(&self) {
        self.please_wait.set_visible(true);
        let (w, h) = (320, 90); // fixed estimate; the box is small and brief
        let (x, y) = monitor::desktop_center(w, h);
        wmprops::configure(&self.please_wait, x, y);
        debug_log!("[🐛DEBUG] show_please_wait at ({}, {})", x, y);
    }

    /// Hide the "Please Wait" window.
    pub fn hide_please_wait(&self) {
        self.please_wait.set_visible(false);
    }

    pub fn show_layout(&self, layout_name: &str) {
        let mut window_height = 0;
        if let Some(child) = self.window.child() {
            if let Ok(content_box) = child.downcast::<Box>() {
                self.update_layout_content(layout_name);
                let (_, natural_h, _, _) = content_box.measure(Orientation::Vertical, self.window_width);
                if natural_h > 0 {
                    self.window.set_size_request(self.window_width, natural_h);
                    window_height = natural_h;
                }
            }
        }

        self.window.set_visible(true);

        // Re-resolve monitor and corner on every show so the overlay follows monitor
        // hotplug and uses the current measured height for bottom-aligned corners.
        let mon = monitor::select_monitor(&self.monitor_pref);
        let (tx, ty) = self.corner.position(&mon, self.window_width, window_height);
        let win = self.window.clone();
        wmprops::configure(&win, tx, ty);

        self.reset_dismiss_timer();
    }


    fn update_layout_content(&self, layout_name: &str) {
        let layouts = match self.layout_map.read() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error: layout map lock poisoned: {}", e);
                return;
            }
        };
        let available_width = self.window_width - 32; // 16px margin each side

        if let Some(layout_info) = layouts.get(layout_name) {
            {
                let title_label = &self.title_label;
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

            {
                let legend_widget = &self.legend_box;
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
            self.title_label.set_label("");
            while let Some(child) = self.legend_box.first_child() {
                self.legend_box.remove(&child);
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
            .please-wait {
                color: white;
                font-weight: bold;
                font-size: 36px;
            }
            .overlay-header-label {
                color: rgba(255, 255, 255, 0.85);
                font-weight: bold;
                font-size: 18px;
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
