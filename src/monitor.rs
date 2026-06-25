// Monitor enumeration and overlay corner positioning (split out of overlay.rs).

use crate::debug_log;
use gtk4::prelude::*;
use gtk4::Align;

/// Inset of the overlay from the monitor edge, in pixels.
pub const WINDOW_MARGIN: i32 = 20;

#[derive(Clone, Copy)]
pub struct MonitorGeometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl MonitorGeometry {
    fn fallback() -> Self {
        MonitorGeometry { x: 0, y: 0, width: 1920, height: 1080 }
    }
}

/// Which corner of the target monitor the overlay aligns to.
#[derive(Clone, Copy)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    /// Parse a config/CLI value. Accepts "top-left", "top_right", "TR", etc.
    /// Returns None for unrecognized values so the caller can warn and default.
    pub fn parse(s: &str) -> Option<Corner> {
        match s.trim().to_lowercase().replace('_', "-").as_str() {
            "top-left" | "tl" => Some(Corner::TopLeft),
            "top-right" | "tr" => Some(Corner::TopRight),
            "bottom-left" | "bl" => Some(Corner::BottomLeft),
            "bottom-right" | "br" => Some(Corner::BottomRight),
            _ => None,
        }
    }

    /// True for the top corners (header sits at the top); false for the bottom corners.
    pub fn is_top(&self) -> bool {
        matches!(self, Corner::TopLeft | Corner::TopRight)
    }

    /// Horizontal alignment matching the corner's side, for the app header.
    pub fn header_halign(&self) -> Align {
        match self {
            Corner::TopLeft | Corner::BottomLeft => Align::Start,
            Corner::TopRight | Corner::BottomRight => Align::End,
        }
    }

    /// Top-left pixel for a `win_w`×`win_h` overlay on `mon`, inset by WINDOW_MARGIN.
    pub fn position(&self, mon: &MonitorGeometry, win_w: i32, win_h: i32) -> (i32, i32) {
        let left = mon.x + WINDOW_MARGIN;
        let right = mon.x + mon.width - win_w - WINDOW_MARGIN;
        let top = mon.y + WINDOW_MARGIN;
        let bottom = mon.y + mon.height - win_h - WINDOW_MARGIN;
        match self {
            Corner::TopLeft => (left, top),
            Corner::TopRight => (right, top),
            Corner::BottomLeft => (left, bottom),
            Corner::BottomRight => (right, bottom),
        }
    }
}

fn geometry_of(monitor: &gdk4::Monitor) -> MonitorGeometry {
    let g = monitor.geometry();
    MonitorGeometry { x: g.x(), y: g.y(), width: g.width(), height: g.height() }
}

/// Select the target monitor. When `pref` is set, it is matched (case-insensitive,
/// in precedence order) against the connector name (e.g. "DP-1"), then the model
/// name, then a numeric index into the monitor list. When `pref` is absent or no
/// match is found, falls back to the rightmost monitor — the legacy default.
pub fn select_monitor(pref: &Option<String>) -> MonitorGeometry {
    let display = match gdk4::Display::default() {
        Some(d) => d,
        None => {
            eprintln!("Error: no display available for monitor enumeration; using 1920x1080 fallback");
            return MonitorGeometry::fallback();
        }
    };
    let monitors = display.monitors();
    let n = monitors.n_items();
    let monitor_at = |i: u32| monitors.item(i).and_then(|o| o.downcast::<gdk4::Monitor>().ok());

    if let Some(want) = pref.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // 1. connector name
        for i in 0..n {
            if let Some(m) = monitor_at(i) {
                if m.connector().is_some_and(|c| c.eq_ignore_ascii_case(want)) {
                    debug_log!("[🐛DEBUG] Selected monitor by connector '{}'", want);
                    return geometry_of(&m);
                }
            }
        }
        // 2. model name
        for i in 0..n {
            if let Some(m) = monitor_at(i) {
                if m.model().is_some_and(|c| c.eq_ignore_ascii_case(want)) {
                    debug_log!("[🐛DEBUG] Selected monitor by model '{}'", want);
                    return geometry_of(&m);
                }
            }
        }
        // 3. numeric index into the monitor list
        if let Ok(idx) = want.parse::<u32>() {
            if let Some(m) = monitor_at(idx) {
                debug_log!("[🐛DEBUG] Selected monitor by index {}", idx);
                return geometry_of(&m);
            }
        }
        eprintln!("Warning: monitor '{}' not found; falling back to rightmost monitor", want);
    }

    // Rightmost monitor: largest x + width.
    let mut best: Option<MonitorGeometry> = None;
    let mut max_x = i32::MIN;
    for i in 0..n {
        if let Some(m) = monitor_at(i) {
            let geom = geometry_of(&m);
            let x_end = geom.x + geom.width;
            if x_end > max_x {
                max_x = x_end;
                best = Some(geom);
            }
        }
    }

    best.unwrap_or_else(|| {
        eprintln!("Error: no monitors found; using 1920x1080 fallback");
        MonitorGeometry::fallback()
    })
}

/// Top-left position to center a `w`×`h` box across the union bounding box of all monitors.
pub fn desktop_center(w: i32, h: i32) -> (i32, i32) {
    let display = match gdk4::Display::default() {
        Some(d) => d,
        None => return (0, 0),
    };
    let monitors = display.monitors();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut found = false;
    for i in 0..monitors.n_items() {
        if let Some(m) = monitors.item(i).and_then(|o| o.downcast::<gdk4::Monitor>().ok()) {
            let g = m.geometry();
            min_x = min_x.min(g.x());
            min_y = min_y.min(g.y());
            max_x = max_x.max(g.x() + g.width());
            max_y = max_y.max(g.y() + g.height());
            found = true;
        }
    }
    if !found {
        let fb = MonitorGeometry::fallback();
        return (fb.x + (fb.width - w) / 2, fb.y + (fb.height - h) / 2);
    }
    (min_x + ((max_x - min_x) - w) / 2, min_y + ((max_y - min_y) - h) / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_parse_accepts_variants() {
        assert!(matches!(Corner::parse("top-left"), Some(Corner::TopLeft)));
        assert!(matches!(Corner::parse("TOP_RIGHT"), Some(Corner::TopRight)));
        assert!(matches!(Corner::parse("  bottom-left "), Some(Corner::BottomLeft)));
        assert!(matches!(Corner::parse("br"), Some(Corner::BottomRight)));
        assert!(Corner::parse("middle").is_none());
        assert!(Corner::parse("").is_none());
    }

    #[test]
    fn corner_position_insets_each_corner_by_margin() {
        // Monitor offset to (100, 50) to confirm the monitor origin is honored.
        let mon = MonitorGeometry { x: 100, y: 50, width: 1000, height: 800 };
        let (w, h) = (600, 400);
        let m = WINDOW_MARGIN;

        assert_eq!(Corner::TopLeft.position(&mon, w, h), (100 + m, 50 + m));
        assert_eq!(Corner::TopRight.position(&mon, w, h), (100 + 1000 - w - m, 50 + m));
        assert_eq!(Corner::BottomLeft.position(&mon, w, h), (100 + m, 50 + 800 - h - m));
        assert_eq!(
            Corner::BottomRight.position(&mon, w, h),
            (100 + 1000 - w - m, 50 + 800 - h - m)
        );
    }
}
