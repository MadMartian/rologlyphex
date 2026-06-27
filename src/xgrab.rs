// Key-grab input thread — the heart of the keyd-free input path.
//
// Both devices already deliver F13–F24 to the X server (macropad in hardware; the
// secondary keyboard macro keys via a keyboard-management daemon). This thread XGrabKeys those keys on the root window,
// owns the layer-ring state machine, and types the active layer's glyph via XTest — replacing
// keyd's command()/macro() dispatch and IPC entirely.
//
// F-keys live only here, at the grab boundary: an incoming keycode maps to a physical F-key
// name, which is either a navigation key (raw) or translates to a logical name via the
// LayersConfig. Everything downstream (typing, the published layer name, logging) is logical.
//
// This thread owns its own Xlib Display and XTyper, matching the daemon's one-Display-per-
// thread model (Xlib is not thread-safe across shared connections; see xtype.rs).
//
use crate::awtfocus::AwtDetector;
use crate::debug_log;
use crate::layers::LayersConfig;
use crate::xerror;
use crate::xtype::LayerTyper;
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use x11::xlib;

/// What a grabbed key does.
#[derive(Clone)]
enum Action {
    NavPrev,
    NavNext,
    /// Type the glyph bound to this logical key in the active layer.
    Type(String),
}

/// Modifier combinations to grab so the lock keys (CapsLock = `LockMask`, NumLock =
/// `Mod2Mask`) don't suppress the grab. The macropad emits F-keys without modifiers, so the
/// four combinations of these two cover real-world states.
const IGNORED_MOD_COMBOS: &[u32] = &[
    0,
    xlib::LockMask,
    xlib::Mod2Mask,
    xlib::LockMask | xlib::Mod2Mask,
];

/// Lead time the "Please Wait" flag is held BEFORE the remap, giving the GTK poll time to
/// display the indicator while the GTK main loop is still free — the remap's MappingNotify
/// then blocks GTK (keymap rebuild) on keycode-heavy systems, so showing it afterwards is too
/// late. Must exceed the 100ms GTK poll interval.
const PWO_LEAD_MS: u64 = 220;

/// Run the grab loop. `current_layout` is shared with the GTK overlay poll, which shows the
/// overlay whenever the published layer name changes. Never returns under normal operation.
/// When the keymap remap for a newly-entered layer happens.
#[derive(Clone, Copy, PartialEq)]
pub enum RemapMode {
    /// Remap during the idle gap after the knob settles; no indicator.
    Debounce,
    /// Remap on the first keypress in the layer, showing a "Please Wait" overlay.
    Lazy,
}

pub fn run(
    cfg: Arc<LayersConfig>,
    current_layout: Arc<Mutex<String>>,
    settle_ms: i32,
    mode: RemapMode,
    please_wait: Arc<AtomicBool>,
    awt_classes: Vec<String>,
) -> Result<(), String> {
    let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
    if display.is_null() {
        return Err("key-grab: cannot open X display".to_string());
    }
    // Re-assert our non-fatal error handlers on this connection so XGrabKey's BadAccess is
    // logged, not fatal (GTK set its own handler during init; this wins process-wide).
    xerror::install();

    let root = unsafe { xlib::XDefaultRootWindow(display) };

    // Resolve every key we care about (raw nav keys + aliased typeable keys) to its keycode.
    let mut to_register: Vec<(String, Action)> = vec![
        (cfg.nav_prev.clone(), Action::NavPrev),
        (cfg.nav_next.clone(), Action::NavNext),
    ];
    for (fkey, logical) in &cfg.phys_to_logical {
        to_register.push((fkey.clone(), Action::Type(logical.clone())));
    }

    let mut actions: HashMap<u8, Action> = HashMap::new();
    for (fkey, action) in to_register {
        match keycode_for(display, &fkey) {
            Some(kc) => {
                actions.insert(kc, action);
            }
            None => eprintln!("Warning: key-grab: no keycode for {}; skipping", fkey),
        }
    }
    if actions.is_empty() {
        return Err("key-grab: no keys resolved to grab".to_string());
    }

    // Grab each keycode for the ignored-modifier combinations.
    unsafe {
        for &kc in actions.keys() {
            for &m in IGNORED_MOD_COMBOS {
                xlib::XGrabKey(
                    display,
                    kc as i32,
                    m,
                    root,
                    xlib::False,
                    xlib::GrabModeAsync,
                    xlib::GrabModeAsync,
                );
            }
        }
        xlib::XSync(display, xlib::False);
    }
    debug_log!("[🐛DEBUG] key-grab: grabbed {} keys on root window", actions.len());

    // Batch-remap typist. Exclude the grabbed keycodes from its scratch pool so it never
    // remaps a key we are grabbing (notably F24 = keycode 202, which carries U2764; see #14).
    let exclude: HashSet<u8> = actions.keys().copied().collect();
    let logical_keys: Vec<String> = cfg.phys_to_logical.values().cloned().collect();
    let typer = LayerTyper::open(&logical_keys, &exclude)?;

    // Non-BMP clipboard routing: classify the focused window against the WM_CLASS whitelist.
    // The check is sampled per keystroke at delivery time (below), not cached at a layer
    // boundary — focus can drift during the lazy keymap remap (see sdd/PLAN.non-BMP.md).
    let awt = AwtDetector::new(display, awt_classes);
    debug_log!("[🐛DEBUG] key-grab: non-BMP clipboard path {}",
        if awt.is_enabled() { "enabled (AWT WM_CLASS whitelist set)" } else { "disabled (no awt_clipboard_classes)" });

    // Layer ring. The grab thread owns the index. On each knob step it only publishes the
    // active layer name (instant — the overlay tracks the knob in real time) and flags that a
    // remap is due. The expensive batch remap (XChangeKeyboardMapping + MappingNotify settle)
    // is deferred to the first keypress in the new layer, so spinning the knob never blocks
    // the event loop and the overlay never lags behind it.
    let ring = &cfg.layer_order;
    let empty: HashMap<String, char> = HashMap::new();
    let mut idx: usize = 0;
    let mut pending_remap = true;
    publish_layer(&current_layout, &ring[idx]);

    // Two remap modes (config `remap_mode`):
    //   Debounce: after the knob settles (`settle_ms` with no events), remap in the idle gap so
    //             the keymap change is done before the user presses a key. No indicator.
    //   Lazy:     remap on the first keypress in a layer, showing a "Please Wait" overlay while
    //             the (slow) keymap change runs.
    // Either way, spinning the knob never remaps mid-spin (no MappingNotify storm).
    debug_log!("[🐛DEBUG] key-grab: remap mode = {}, navigation settle = {}ms",
        if mode == RemapMode::Lazy { "lazy" } else { "debounce" }, settle_ms);
    let xfd = unsafe { xlib::XConnectionNumber(display) };

    loop {
        // Drain everything currently queued before waiting.
        while unsafe { xlib::XPending(display) } > 0 {
            let mut ev: xlib::XEvent = unsafe { std::mem::zeroed() };
            unsafe { xlib::XNextEvent(display, &mut ev) };
            let etype = unsafe { ev.type_ };
            if etype != xlib::KeyPress && etype != xlib::KeyRelease {
                continue;
            }
            let keycode = unsafe { ev.key.keycode } as u8;
            match actions.get(&keycode) {
                // Navigation does no injection, so handle it on press for responsiveness.
                Some(Action::NavPrev) if etype == xlib::KeyPress => {
                    idx = (idx + ring.len() - 1) % ring.len();
                    debug_log!("[🐛DEBUG] prev -> layer {}", ring[idx]);
                    publish_layer(&current_layout, &ring[idx]);
                    pending_remap = true;
                }
                Some(Action::NavNext) if etype == xlib::KeyPress => {
                    idx = (idx + 1) % ring.len();
                    debug_log!("[🐛DEBUG] next -> layer {}", ring[idx]);
                    publish_layer(&current_layout, &ring[idx]);
                    pending_remap = true;
                }
                // Type on RELEASE: the passive grab becomes an active keyboard grab while the
                // physical key is held, which would swallow our XTest injection. By the time
                // the key is released the active grab has ended, so the synthetic key reaches
                // the focused app. See sdd/ANTI-PATTERNS.md.
                Some(Action::Type(logical)) if etype == xlib::KeyRelease => {
                    let layer = &ring[idx];
                    if pending_remap {
                        // In lazy mode the remap happens here, on first press. Raise the
                        // "Please Wait" flag and give the GTK poll time to actually display it
                        // BEFORE the remap runs — the remap's MappingNotify blocks the GTK main
                        // loop (keymap rebuild), so showing it afterwards never renders.
                        let show_wait = mode == RemapMode::Lazy;
                        if show_wait {
                            debug_log!("[🐛DEBUG] lazy: please_wait=true (remapping {})", layer);
                            please_wait.store(true, Ordering::Relaxed);
                            std::thread::sleep(std::time::Duration::from_millis(PWO_LEAD_MS));
                        }
                        typer.set_layer(cfg.typing.get(layer).unwrap_or(&empty));
                        if show_wait {
                            please_wait.store(false, Ordering::Relaxed);
                        }
                        pending_remap = false;
                    }
                    // Route non-BMP glyphs through the clipboard ONLY when an AWT window is
                    // focused (its keysym path truncates them); everything else — all BMP
                    // glyphs, and non-BMP into non-AWT apps — takes the working keysym path.
                    // Focus is sampled HERE, at delivery, after the remap above has settled.
                    match cfg.glyph_for(layer, logical) {
                        Some(ch) if (ch as u32) > 0xFFFF && awt.focused_is_awt(display) => {
                            debug_log!("[🐛DEBUG] {} -> '{}' (U+{:04X}) non-BMP into AWT: clipboard paste",
                                logical, ch, ch as u32);
                            typer.paste_via_clipboard(&ch.to_string());
                        }
                        Some(ch) => {
                            debug_log!("[🐛DEBUG] {} -> '{}' (layer {})", logical, ch, layer);
                            typer.press(logical);
                        }
                        None => {
                            debug_log!("[🐛DEBUG] {} is unbound in layer {}", logical, layer);
                            typer.press(logical);
                        }
                    }
                }
                _ => {}
            }
        }

        // Wait for more events. In debounce mode with a remap due, only wait `settle_ms` so we
        // can perform it in the idle gap once the knob stops; otherwise block indefinitely
        // (lazy mode remaps on keypress, not on a timer).
        let timeout = if pending_remap && mode == RemapMode::Debounce { settle_ms } else { -1 };
        let mut pfd = libc::pollfd { fd: xfd, events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, timeout) };
        if ready == 0 && pending_remap {
            // Debounce mode settled on a layer with no input -> remap ahead of the next press.
            typer.set_layer(cfg.typing.get(&ring[idx]).unwrap_or(&empty));
            pending_remap = false;
        }
    }
}

/// Resolve a function-key name (e.g. "F13") to its X keycode.
///
/// Preferred path: the keysym is in the X keymap (true for F13–F18 here). Fallback: F13–F24
/// occupy deterministic evdev-derived keycodes even when the keysym is absent from the keymap
/// — the secondary keyboard macro keys emit evdev `KEY_F19..F24`, which land at X keycodes 197–202 that
/// the keymap doesn't label. Since XGrabKey works by keycode, we grab those directly.
fn keycode_for(display: *mut xlib::Display, fkey: &str) -> Option<u8> {
    if let Ok(cstr) = CString::new(fkey) {
        let keysym = unsafe { xlib::XStringToKeysym(cstr.as_ptr()) };
        if keysym != xlib::NoSymbol as xlib::KeySym {
            let kc = unsafe { xlib::XKeysymToKeycode(display, keysym) };
            if kc != 0 {
                return Some(kc);
            }
        }
    }
    evdev_fkey_keycode(fkey)
}

/// Deterministic X keycode for a function key F13–F24, derived from the Linux evdev keycodes
/// (`KEY_F13` = 183 … `KEY_F24` = 194) plus the standard XKB offset of 8 (→ 191 … 202).
fn evdev_fkey_keycode(fkey: &str) -> Option<u8> {
    let digits = fkey.strip_prefix('F').or_else(|| fkey.strip_prefix('f'))?;
    let n: u32 = digits.parse().ok()?;
    if (13..=24).contains(&n) {
        Some((183 + (n - 13) + 8) as u8)
    } else {
        None
    }
}

/// Publish the active layer name for the overlay poll loop, recovering from a poisoned lock.
fn publish_layer(current_layout: &Arc<Mutex<String>>, name: &str) {
    let mut guard = current_layout.lock().unwrap_or_else(|e| e.into_inner());
    *guard = name.to_string();
}
