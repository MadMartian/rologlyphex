use x11::xlib;
use x11::xtest;
use std::collections::HashMap;
use std::ptr;
use crate::debug_log;

/// Persistent X11 connection for typing characters via XTest.
/// Characters are typed via keyboard remapping + XTest key synthesis.
/// See sdd/PLAN.non-BMP.md for planned non-BMP character support.
pub struct XTyper {
    display: *mut xlib::Display,
    cache: HashMap<xlib::KeySym, u8>,
    free_keycodes: Vec<u8>,
}

// SAFETY: XTyper owns a dedicated Xlib Display connection opened in XTyper::open().
// It is never shared with the GTK thread (which uses its own GDK Display). The raw
// pointer is only accessed from the socket server thread that owns the XTyper instance.
unsafe impl Send for XTyper {}

/// Delay to allow other X11 clients to process MappingNotify events
/// before we attempt to use newly mapped keycodes.
/// See sdd/ANTI-PATTERNS.md #9.
const MAPPING_NOTIFY_SETTLE_MS: u64 = 30;

impl XTyper {
    pub fn open() -> Result<Self, String> {
        let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
        if display.is_null() {
            return Err("Cannot open X11 display".to_string());
        }

        // Collect unused keycodes
        let mut min_kc: i32 = 0;
        let mut max_kc: i32 = 0;
        unsafe { xlib::XDisplayKeycodes(display, &mut min_kc, &mut max_kc); }

        let mut free_keycodes = Vec::new();
        let mut cache = HashMap::new();

        for kc in (min_kc..=max_kc).rev() {
            let mut keysyms_per_kc: i32 = 0;
            let mapping = unsafe {
                xlib::XGetKeyboardMapping(display, kc as u8, 1, &mut keysyms_per_kc)
            };
            if mapping.is_null() { continue; }

            let first_sym = unsafe { *mapping };
            let all_empty = (0..keysyms_per_kc as usize).all(|i| unsafe {
                *mapping.add(i) == xlib::NoSymbol as xlib::KeySym
            });
            unsafe { xlib::XFree(mapping as *mut _); }

            if all_empty {
                free_keycodes.push(kc as u8);
            } else if first_sym >= 0x01000000 {
                // Reclaim keycode previously mapped by rologlyphex — pre-populate
                // cache so this keysym is found instantly without a new mapping slot.
                cache.insert(first_sym, kc as u8);
            }
        }

        debug_log!("[🐛DEBUG] XTyper: {} free keycodes, {} reclaimed from previous session",
            free_keycodes.len(), cache.len());

        Ok(XTyper {
            display,
            cache,
            free_keycodes,
        })
    }

    pub fn type_char(&mut self, ch: char) {
        let keysym = unicode_to_keysym(ch);

        if let Some(&keycode) = self.cache.get(&keysym) {
            self.send_key(keycode as u32);
            return;
        }

        let keycode = unsafe { xlib::XKeysymToKeycode(self.display, keysym) };
        if keycode != 0 {
            self.send_key(keycode as u32);
            return;
        }

        self.remap_and_type(keysym);
    }

    fn send_key(&self, keycode: u32) {
        unsafe {
            xtest::XTestFakeKeyEvent(self.display, keycode, xlib::True, 0);
            xtest::XTestFakeKeyEvent(self.display, keycode, xlib::False, 0);
            xlib::XFlush(self.display);
        }
    }

    fn remap_and_type(&mut self, keysym: xlib::KeySym) {
        let t0 = std::time::Instant::now();

        let free_kc = match self.free_keycodes.last() {
            Some(&kc) => kc,
            None => {
                eprintln!("Error: no free keycodes for keysym 0x{:x}", keysym);
                return;
            }
        };

        unsafe {
            let mut keysyms_per_kc: i32 = 0;
            let mapping = xlib::XGetKeyboardMapping(self.display, free_kc, 1, &mut keysyms_per_kc);
            if !mapping.is_null() {
                xlib::XFree(mapping as *mut _);
            }

            if keysyms_per_kc <= 0 {
                eprintln!("Error: invalid keyboard mapping width: {}", keysyms_per_kc);
                return;
            }

            let mut new_syms = vec![xlib::NoSymbol as xlib::KeySym; keysyms_per_kc as usize];
            new_syms[0] = keysym;

            xlib::XChangeKeyboardMapping(
                self.display,
                free_kc as i32,
                keysyms_per_kc,
                new_syms.as_ptr() as *mut _,
                1,
            );
            xlib::XSync(self.display, xlib::False);
        }

        // Wait for clients to process MappingNotify.
        // Race condition: pkexec apps (elevated) may be slower to refresh their keymap.
        std::thread::sleep(std::time::Duration::from_millis(MAPPING_NOTIFY_SETTLE_MS));

        self.send_key(free_kc as u32);

        self.cache.insert(keysym, free_kc);
        self.free_keycodes.pop();

        debug_log!("[🐛DEBUG] Remapped keysym 0x{:x} -> keycode {} ({:?}), {} free remaining",
            keysym, free_kc, t0.elapsed(), self.free_keycodes.len());
    }
}

impl Drop for XTyper {
    fn drop(&mut self) {
        unsafe {
            xlib::XCloseDisplay(self.display);
        }
    }
}

fn unicode_to_keysym(ch: char) -> xlib::KeySym {
    let cp = ch as u64;
    if cp >= 0x20 && cp <= 0xFF {
        cp as xlib::KeySym
    } else {
        (0x01000000 + cp) as xlib::KeySym
    }
}
