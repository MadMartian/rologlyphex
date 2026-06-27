// Focus-gated AWT detection for the non-BMP clipboard path (see sdd/PLAN.non-BMP.md).
//
// Java/AWT (notably JetBrains IDEs) truncates non-BMP keysyms via `(int)(keysym & 0xFFFF)`,
// so emoji typed through the XTest/keysym path render as wrong CJK glyphs there — but the
// clipboard *paste* path is unaffected. This module decides, per keystroke, whether the
// focused top-level window is one of those apps, so the grab loop can route non-BMP glyphs
// through the clipboard ONLY then and leave the keysym path untouched everywhere else.
//
// Detection is a config whitelist of WM_CLASS globs (not a /proc cmdline JVM probe, which is
// brittle): read `_NET_ACTIVE_WINDOW` (the managed top-level, which carries WM_CLASS), then
// `XGetClassHint`, and match either res_name or res_class against the whitelist.

use std::ffi::CStr;
use std::ptr;
use x11::xlib;

/// Anchored glob match. The pattern must match the WHOLE `text` (both ends anchored); `*`
/// is the only wildcard and matches any run including empty; every other character is
/// literal. So `jetbrains-*` matches `jetbrains-pycharm` but not `not-jetbrains-foo`, and
/// `SmartGit` matches only `SmartGit` exactly — never a class that merely contains it.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Backtracking point for the most recent '*': where it sits in `p`, and how far we've
    // tentatively let it consume in `t`.
    let mut star: Option<usize> = None;
    let mut star_match = 0usize;
    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_match = ti;
            pi += 1;
        } else if let Some(s) = star {
            // Mismatch: let the last '*' swallow one more char and retry.
            pi = s + 1;
            star_match += 1;
            ti = star_match;
        } else {
            return false;
        }
    }
    // Trailing '*'s in the pattern can match the empty remainder.
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Classifies the focused window against a WM_CLASS whitelist. Holds the interned
/// `_NET_ACTIVE_WINDOW` atom; the caller passes the X display it owns (single-threaded use
/// on the grab thread's connection — no second connection is opened).
pub struct AwtDetector {
    patterns: Vec<String>,
    net_active_window: xlib::Atom,
}

impl AwtDetector {
    pub fn new(display: *mut xlib::Display, patterns: Vec<String>) -> Self {
        let net_active_window = unsafe {
            xlib::XInternAtom(display, b"_NET_ACTIVE_WINDOW\0".as_ptr() as *const _, xlib::False)
        };
        AwtDetector { patterns, net_active_window }
    }

    /// No patterns configured -> the clipboard path is disabled entirely.
    pub fn is_enabled(&self) -> bool {
        !self.patterns.is_empty()
    }

    /// True if the currently focused top-level window's `WM_CLASS` (res_name OR res_class)
    /// matches any whitelist glob.
    pub fn focused_is_awt(&self, display: *mut xlib::Display) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let win = self.focused_window(display);
        if win == 0 {
            return false;
        }
        self.window_matches(display, win)
    }

    /// The managed top-level window: prefer `_NET_ACTIVE_WINDOW` (set by the WM on the
    /// top-level, which is where WM_CLASS lives); fall back to `XGetInputFocus`.
    fn focused_window(&self, display: *mut xlib::Display) -> xlib::Window {
        let root = unsafe { xlib::XDefaultRootWindow(display) };
        let mut actual_type: xlib::Atom = 0;
        let mut actual_format: i32 = 0;
        let mut nitems: u64 = 0;
        let mut bytes_after: u64 = 0;
        let mut prop: *mut u8 = ptr::null_mut();
        let status = unsafe {
            xlib::XGetWindowProperty(
                display, root, self.net_active_window,
                0, 1, xlib::False, xlib::XA_WINDOW,
                &mut actual_type, &mut actual_format, &mut nitems, &mut bytes_after, &mut prop,
            )
        };
        if status == xlib::Success as i32 && !prop.is_null() && nitems >= 1 {
            let w = unsafe { *(prop as *const xlib::Window) };
            unsafe { xlib::XFree(prop as *mut _) };
            if w != 0 {
                return w;
            }
        } else if !prop.is_null() {
            unsafe { xlib::XFree(prop as *mut _) };
        }

        let mut focus: xlib::Window = 0;
        let mut revert: i32 = 0;
        unsafe { xlib::XGetInputFocus(display, &mut focus, &mut revert) };
        focus
    }

    fn window_matches(&self, display: *mut xlib::Display, win: xlib::Window) -> bool {
        let mut hint = xlib::XClassHint {
            res_name: ptr::null_mut(),
            res_class: ptr::null_mut(),
        };
        if unsafe { xlib::XGetClassHint(display, win, &mut hint) } == 0 {
            return false;
        }
        let res_name = unsafe { cstr_opt(hint.res_name) };
        let res_class = unsafe { cstr_opt(hint.res_class) };
        unsafe {
            if !hint.res_name.is_null() {
                xlib::XFree(hint.res_name as *mut _);
            }
            if !hint.res_class.is_null() {
                xlib::XFree(hint.res_class as *mut _);
            }
        }
        self.patterns.iter().any(|p| {
            res_name.as_deref().is_some_and(|n| glob_match(p, n))
                || res_class.as_deref().is_some_and(|c| glob_match(p, c))
        })
    }
}

/// Copy a (possibly null) C string from Xlib into an owned String.
unsafe fn cstr_opt(p: *mut std::os::raw::c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok().map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn exact_match_is_anchored_both_ends() {
        assert!(glob_match("SmartGit", "SmartGit"));
        assert!(!glob_match("SmartGit", "SmartGitExtra"));
        assert!(!glob_match("SmartGit", "PreSmartGit"));
        assert!(!glob_match("SmartGit", "smartgit")); // case-sensitive
    }

    #[test]
    fn trailing_star_is_prefix() {
        assert!(glob_match("jetbrains-*", "jetbrains-pycharm"));
        assert!(glob_match("jetbrains-*", "jetbrains-rustrover"));
        assert!(glob_match("jetbrains-*", "jetbrains-")); // star matches empty
        assert!(!glob_match("jetbrains-*", "not-jetbrains-foo"));
        assert!(!glob_match("jetbrains-*", "jetbrain")); // literal prefix must be present
    }

    #[test]
    fn middle_star_anchors_both_literals() {
        assert!(glob_match("some*text", "sometext"));
        assert!(glob_match("some*text", "some-middle-text"));
        assert!(!glob_match("some*text", "some-text-trailer"));
        assert!(!glob_match("some*text", "lead-some-text"));
    }

    #[test]
    fn leading_star_is_suffix() {
        assert!(glob_match("*-X11", "sun-awt-X11"));
        assert!(glob_match("*-X11", "-X11"));
        assert!(!glob_match("*-X11", "sun-awt-X11-frame"));
    }

    #[test]
    fn bare_star_matches_anything() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything-at-all"));
    }

    #[test]
    fn multiple_stars() {
        assert!(glob_match("*awt*", "sun-awt-X11"));
        assert!(glob_match("*awt*", "awt"));
        assert!(!glob_match("*awt*", "AWT")); // case-sensitive
    }
}
