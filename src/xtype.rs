use x11::xlib;
use x11::xtest;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ptr;
use std::time::{Duration, Instant};
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

    pub fn type_char(&mut self, ch: char) -> Result<(), String> {
        let keysym = unicode_to_keysym(ch);

        if let Some(&keycode) = self.cache.get(&keysym) {
            // Move to back of LRU (most recently used) so it's last to be evicted.
            if let Some(pos) = self.lru_order.iter().position(|&s| s == keysym) {
                self.lru_order.remove(pos);
                self.lru_order.push_back(keysym);
            }
            self.send_key(keycode as u32);
            return Ok(());
        }

        let keycode = unsafe { xlib::XKeysymToKeycode(self.display, keysym) };
        if keycode != 0 {
            self.send_key(keycode as u32);
            return Ok(());
        }

        self.remap_and_type(keysym)
    }

    fn send_key(&self, keycode: u32) {
        unsafe {
            xtest::XTestFakeKeyEvent(self.display, keycode, xlib::True, 0);
            xtest::XTestFakeKeyEvent(self.display, keycode, xlib::False, 0);
            xlib::XFlush(self.display);
        }
    }

    fn remap_and_type(&mut self, keysym: xlib::KeySym) -> Result<(), String> {
        let t0 = std::time::Instant::now();

        // Prefer a free keycode; if the pool is exhausted, evict the LRU cached entry.
        let (free_kc, consumed_free_slot) = if let Some(&kc) = self.free_keycodes.last() {
            (kc, true)
        } else {
            let evicted_sym = self.lru_order.pop_front()
                .ok_or_else(|| format!("no keycodes available for keysym 0x{:x}", keysym))?;
            let kc = self.cache.remove(&evicted_sym)
                .ok_or_else(|| format!("LRU eviction cache miss for 0x{:x}", evicted_sym))?;
            debug_log!("[🐛DEBUG] Evicting LRU keysym 0x{:x} from keycode {} for 0x{:x}",
                evicted_sym, kc, keysym);
            (kc, false)
        };

        let keysyms_per_kc = unsafe {
            let mut n: i32 = 0;
            let mapping = xlib::XGetKeyboardMapping(self.display, free_kc, 1, &mut n);
            if !mapping.is_null() {
                xlib::XFree(mapping as *mut _);
            }
            n
        };

        if keysyms_per_kc <= 0 {
            return Err(format!("invalid keyboard mapping width: {}", keysyms_per_kc));
        }

        unsafe {
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
        Ok(())
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

/// Batch-remap typist for the key-grab model (see sdd/ANTI-PATTERNS.md #18).
/// Only one layer (≤ the number of logical keys) is active at a
/// time, so instead of remapping a keycode on every keypress — which thrashes when the
/// secondary keyboard leaves ~0 free keycodes, storming MappingNotify (#9) — each logical button is
/// assigned a stable scratch keycode once, and the whole set is remapped to the active
/// layer's glyphs in a single batch per layer change. A button press then just fires its
/// scratch keycode: no remap, no MappingNotify.
pub struct LayerTyper {
    display: *mut xlib::Display,
    /// logical key name -> its dedicated scratch keycode.
    index: HashMap<String, u8>,
}

// SAFETY: owns a dedicated Xlib Display connection, used only by the key-grab thread.
unsafe impl Send for LayerTyper {}

impl LayerTyper {
    /// Open a dedicated display and assign each logical key a scratch keycode. `exclude` lists
    /// keycodes that must never be used (the grabbed F13–F24 keycodes — one of which, F24,
    /// carries the Unicode keysym U2764 and would otherwise be reclaimed; see #14).
    pub fn open(logical_keys: &[String], exclude: &HashSet<u8>) -> Result<Self, String> {
        let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
        if display.is_null() {
            return Err("LayerTyper: cannot open X11 display".to_string());
        }

        let (scratch, _width) = Self::scan_scratch(display, exclude, logical_keys.len())
            .map_err(|e| {
                unsafe { xlib::XCloseDisplay(display) };
                e
            })?;

        let index: HashMap<String, u8> = logical_keys
            .iter()
            .cloned()
            .zip(scratch.iter().copied())
            .collect();

        debug_log!("[🐛DEBUG] LayerTyper: assigned {} scratch keycodes {:?}", index.len(), scratch);
        Ok(LayerTyper { display, index })
    }

    /// Find `needed` usable scratch keycodes, preferring free (all-NoSymbol) ones, then
    /// keycodes already holding our Unicode mappings (reclaimable). Scans high→low to avoid
    /// the low keycodes near min_kc that XTest silently swallows (#13), skipping `exclude`.
    fn scan_scratch(
        display: *mut xlib::Display,
        exclude: &HashSet<u8>,
        needed: usize,
    ) -> Result<(Vec<u8>, i32), String> {
        let (mut min_kc, mut max_kc): (i32, i32) = (0, 0);
        unsafe { xlib::XDisplayKeycodes(display, &mut min_kc, &mut max_kc) };

        let mut width = 0i32;
        let mut free = Vec::new();
        let mut reclaim = Vec::new();

        for kc in (min_kc..=max_kc).rev() {
            if kc <= min_kc {
                continue; // avoid min_kc (#13)
            }
            let kcu = kc as u8;
            if exclude.contains(&kcu) {
                continue;
            }
            let mut n = 0;
            let mapping = unsafe { xlib::XGetKeyboardMapping(display, kcu, 1, &mut n) };
            if mapping.is_null() {
                continue;
            }
            if width == 0 {
                width = n;
            }
            let first = unsafe { *mapping };
            let all_empty =
                (0..n as usize).all(|i| unsafe { *mapping.add(i) == xlib::NoSymbol as xlib::KeySym });
            unsafe { xlib::XFree(mapping as *mut _) };

            if all_empty {
                free.push(kcu);
            } else if (0x01000000..=0x0110FFFF).contains(&first) {
                reclaim.push(kcu); // a Unicode keysym -> one of ours from a prior session
            }
        }

        let scratch: Vec<u8> = free.into_iter().chain(reclaim).take(needed).collect();
        if scratch.len() < needed {
            return Err(format!(
                "LayerTyper: only {} scratch keycodes available, need {} (X11 keycode pool exhausted — see ANTI-PATTERNS #18)",
                scratch.len(),
                needed
            ));
        }
        Ok((scratch, width.max(1)))
    }

    /// Remap the scratch keycodes to the active layer's glyphs in a **single**
    /// `XChangeKeyboardMapping` over the contiguous keycode range that spans them, preserving
    /// every non-scratch keycode in that range. One call = one `MappingNotify` broadcast.
    ///
    /// This is critical: issuing one `XChangeKeyboardMapping` per scratch keycode produced a
    /// `MappingNotify` storm that forced every X client to reprocess the keymap, degrading
    /// whole-system performance (mouse, scrolling). See sdd/ANTI-PATTERNS.md.
    pub fn set_layer(&self, glyphs: &HashMap<String, char>) {
        if self.index.is_empty() {
            return;
        }
        let t0 = Instant::now();
        let min = *self.index.values().min().unwrap();
        let max = *self.index.values().max().unwrap();
        let count = max as i32 - min as i32 + 1;

        unsafe {
            // Read the current keysyms for the whole range so non-scratch keycodes are
            // preserved exactly (the grabbed F13–F24 and any real keys in the range).
            let mut width = 0;
            let existing = xlib::XGetKeyboardMapping(self.display, min, count, &mut width);
            if existing.is_null() || width <= 0 {
                return;
            }
            let w = width as usize;
            let mut syms: Vec<xlib::KeySym> =
                std::slice::from_raw_parts(existing, count as usize * w).to_vec();
            xlib::XFree(existing as *mut _);

            for (logical, &kc) in &self.index {
                let row = (kc - min) as usize * w;
                let keysym = match glyphs.get(logical) {
                    Some(&ch) => unicode_to_keysym(ch),
                    None => xlib::NoSymbol as xlib::KeySym,
                };
                syms[row] = keysym;
                // Clear higher levels so Shift/etc. can't yield a different glyph.
                for level in 1..w {
                    syms[row + level] = xlib::NoSymbol as xlib::KeySym;
                }
            }

            xlib::XChangeKeyboardMapping(self.display, min as i32, w as i32, syms.as_ptr() as *mut _, count);
            xlib::XSync(self.display, xlib::False);
        }
        // Let clients process the single MappingNotify before the next keypress (#9).
        std::thread::sleep(Duration::from_millis(MAPPING_NOTIFY_SETTLE_MS));
        debug_log!("[🐛DEBUG] set_layer: remapped {} keys over range {}..={} in {:?}",
            self.index.len(), min, max, t0.elapsed());
    }

    /// Fire the scratch keycode bound to `logical` (no remap).
    pub fn press(&self, logical: &str) {
        if let Some(&kc) = self.index.get(logical) {
            unsafe {
                xtest::XTestFakeKeyEvent(self.display, kc as u32, xlib::True, 0);
                xtest::XTestFakeKeyEvent(self.display, kc as u32, xlib::False, 0);
                xlib::XFlush(self.display);
            }
        }
    }
}

impl Drop for LayerTyper {
    fn drop(&mut self) {
        unsafe { xlib::XCloseDisplay(self.display) };
    }
}
