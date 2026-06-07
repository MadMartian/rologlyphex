use x11::xlib;
use x11::xtest;
use std::collections::{HashMap, VecDeque};
use std::ptr;
use crate::debug_log;

/// Persistent X11 connection for typing characters via XTest.
/// Characters are typed via keyboard remapping + XTest key synthesis.
/// Non-BMP character support is a known limitation; see sdd/ISSUES.md entry D.
pub struct XTyper {
    display: *mut xlib::Display,
    cache: HashMap<xlib::KeySym, u8>,
    free_keycodes: Vec<u8>,
    /// LRU order for cache eviction when free_keycodes is exhausted.
    /// Front = least recently used (evict first). Back = most recently used.
    lru_order: VecDeque<xlib::KeySym>,
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

        let mut typer = XTyper {
            display,
            cache: HashMap::new(),
            free_keycodes: Vec::new(),
            lru_order: VecDeque::new(),
        };

        typer.scan_keycodes();
        Ok(typer)
    }

    pub fn rescan(&mut self) {
        self.cache.clear();
        self.free_keycodes.clear();
        self.lru_order.clear();
        self.scan_keycodes();
        debug_log!(
            "[🐛DEBUG] XTyper rescan: {} free keycodes, {} reclaimed",
            self.free_keycodes.len(),
            self.cache.len()
        );
    }

    fn scan_keycodes(&mut self) {
        let mut min_kc: i32 = 0;
        let mut max_kc: i32 = 0;
        unsafe {
            xlib::XDisplayKeycodes(self.display, &mut min_kc, &mut max_kc);
        }

        for kc in min_kc..=max_kc {
            let mut keysyms_per_kc: i32 = 0;
            let mapping = unsafe { xlib::XGetKeyboardMapping(self.display, kc as u8, 1, &mut keysyms_per_kc) };
            if mapping.is_null() {
                continue;
            }

            let first_sym = unsafe { *mapping };
            let all_empty = (0..keysyms_per_kc as usize)
                .all(|i| unsafe { *mapping.add(i) == xlib::NoSymbol as xlib::KeySym });
            unsafe {
                xlib::XFree(mapping as *mut _);
            }

            if all_empty {
                self.free_keycodes.push(kc as u8);
            } else if first_sym >= 0x01000000 && first_sym <= 0x0110FFFF {
                // Reclaim keycode previously mapped by rologlyphex — pre-populate
                // cache so this keysym is found instantly without a new mapping slot.
                self.cache.insert(first_sym, kc as u8);
                self.lru_order.push_back(first_sym);
            }
        }
    }

    pub fn type_char(&mut self, ch: char) {
        let keysym = unicode_to_keysym(ch);

        if let Some(&keycode) = self.cache.get(&keysym) {
            // Move to back of LRU (most recently used) so it's last to be evicted.
            if let Some(pos) = self.lru_order.iter().position(|&s| s == keysym) {
                self.lru_order.remove(pos);
                self.lru_order.push_back(keysym);
            }
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

        // Prefer a free keycode; if the pool is exhausted, evict the LRU cached entry.
        let (free_kc, consumed_free_slot) = if let Some(&kc) = self.free_keycodes.last() {
            (kc, true)
        } else {
            let evicted_sym = match self.lru_order.pop_front() {
                Some(s) => s,
                None => {
                    eprintln!("Error: no keycodes available for keysym 0x{:x}", keysym);
                    return;
                }
            };
            let kc = match self.cache.remove(&evicted_sym) {
                Some(k) => k,
                None => {
                    eprintln!("Error: LRU eviction cache miss for 0x{:x}", evicted_sym);
                    return;
                }
            };
            debug_log!("[🐛DEBUG] Evicting LRU keysym 0x{:x} from keycode {} for 0x{:x}",
                evicted_sym, kc, keysym);
            (kc, false)
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
        self.lru_order.push_back(keysym);
        if consumed_free_slot {
            self.free_keycodes.pop();
        }

        debug_log!("[🐛DEBUG] Remapped keysym 0x{:x} -> keycode {} ({:?}), {} free + {} cached",
            keysym, free_kc, t0.elapsed(), self.free_keycodes.len(), self.cache.len());
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
