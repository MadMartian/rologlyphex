// Persistent X11 CLIPBOARD owner thread for the non-BMP paste path (see sdd/TECH.md and the
// retired sdd/PLAN.non-BMP.md history in git).
//
// The grab thread can't serve the X11 selection protocol: it must return to its input loop
// immediately, and a synchronous paste that releases ownership leaves CLIPBOARD unowned
// (empty) — it does not restore the user's prior clipboard. So all clipboard ownership lives
// here, in a thread that owns its own Xlib display + a hidden window for the daemon's lifetime.
//
// On a Paste command this thread:
//   1. saves the user's current CLIPBOARD (fetched from the existing owner) — once, while we
//      don't already own it;
//   2. claims ownership, sets the payload to the emoji, and synthesizes Ctrl+V;
//   3. serves SelectionRequests until the consumer's *data* request is answered, then a short
//      grace window — event-driven release, the ANTI-PATTERNS #24 fix (a fixed timeout dropped
//      ownership before a slow JVM consumer requested the data);
//   4. swaps the payload to the saved content and keeps serving it, so the prior clipboard is
//      actually restored, until the user copies something else (SelectionClear).

use crate::debug_log;
use std::ptr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use x11::{keysym, xlib, xtest};

/// Grace held after the paste's data request is answered before swapping to the restored
/// content — covers a slow consumer that re-polls TARGETS/UTF8_STRING a few more times.
const GRACE: Duration = Duration::from_millis(250);
/// Backstop if the consumer never requests the data at all (e.g. Ctrl+V went nowhere).
const HARD_CAP: Duration = Duration::from_millis(1500);
/// Bound on the synchronous fetch of the prior clipboard from its current owner.
const FETCH_TIMEOUT: Duration = Duration::from_millis(300);
/// Poll cadence while we own the selection and must answer requests promptly.
const ACTIVE_TICK_MS: i32 = 10;

enum Cmd {
    Paste(Vec<u8>),
}

/// Handle to the clipboard-owner thread. Cheap to clone-free share; sending is non-blocking.
pub struct ClipboardServer {
    tx: Sender<Cmd>,
}

impl ClipboardServer {
    /// Spawn the owner thread and wait for it to confirm its X connection opened.
    pub fn spawn() -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || match Owner::open() {
            Ok(mut owner) => {
                let _ = ready_tx.send(Ok(()));
                owner.run(rx);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(ClipboardServer { tx }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("clipboard owner thread exited during startup".to_string()),
        }
    }

    /// Queue `text` to be delivered via the clipboard. Non-blocking; the owner thread does the
    /// claim/Ctrl+V/serve/restore.
    pub fn paste(&self, text: &str) {
        let _ = self.tx.send(Cmd::Paste(text.as_bytes().to_vec()));
    }
}

struct Owner {
    dpy: *mut xlib::Display,
    win: xlib::Window,
    clipboard: xlib::Atom,
    utf8: xlib::Atom,
    targets: xlib::Atom,
    prop: xlib::Atom, // scratch property used to fetch the external clipboard
    /// Bytes we currently serve to requestors (None = we serve nothing / don't own).
    payload: Option<Vec<u8>>,
    /// The user's prior clipboard, restored once the emoji has been consumed.
    saved: Option<Vec<u8>>,
    owns: bool,
    /// Set while a paste is being delivered (between claim and restore).
    deliver_start: Option<Instant>,
    data_served_at: Option<Instant>,
}

impl Owner {
    fn open() -> Result<Self, String> {
        // SAFETY: opening a fresh Xlib display; null name = $DISPLAY. Returns null on failure.
        let dpy = unsafe { xlib::XOpenDisplay(ptr::null()) };
        if dpy.is_null() {
            return Err("clipboard: cannot open X display".to_string());
        }
        // SAFETY: dpy is a live connection just opened above; the names are static NUL literals.
        let (win, clipboard, utf8, targets, prop) = unsafe {
            let root = xlib::XDefaultRootWindow(dpy);
            let win = xlib::XCreateSimpleWindow(dpy, root, 0, 0, 1, 1, 0, 0, 0);
            (
                win,
                intern(dpy, b"CLIPBOARD\0"),
                intern(dpy, b"UTF8_STRING\0"),
                intern(dpy, b"TARGETS\0"),
                intern(dpy, b"ROLOGLYPHEX_CLIP\0"),
            )
        };
        Ok(Owner {
            dpy, win, clipboard, utf8, targets, prop,
            payload: None, saved: None, owns: false,
            deliver_start: None, data_served_at: None,
        })
    }

    fn run(&mut self, rx: Receiver<Cmd>) {
        let xfd = unsafe { xlib::XConnectionNumber(self.dpy) };
        loop {
            // Idle (not owning, nothing in flight): block on the channel — no busy-wait, and a
            // queued paste is handled with zero latency. No SelectionRequests can arrive while
            // we don't own the selection, so there is nothing to serve here.
            if !self.owns && self.deliver_start.is_none() {
                match rx.recv() {
                    Ok(Cmd::Paste(bytes)) => self.begin_paste(bytes),
                    Err(_) => return, // sender dropped: daemon shutting down
                }
                continue;
            }

            // Active: drain any queued pastes, serve X requests, advance the restore timer,
            // then wait briefly for more X activity.
            while let Ok(Cmd::Paste(bytes)) = rx.try_recv() {
                self.begin_paste(bytes);
            }
            self.pump_events();
            self.maybe_restore();

            let mut pfd = libc::pollfd { fd: xfd, events: libc::POLLIN, revents: 0 };
            // SAFETY: single valid pollfd; bounded timeout. poll has no aliasing requirements.
            unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, ACTIVE_TICK_MS) };
        }
    }

    fn begin_paste(&mut self, emoji: Vec<u8>) {
        // Save the external clipboard once — only when we don't already own it, so consecutive
        // emoji keep restoring to the *original* prior content.
        if !self.owns {
            self.saved = self.fetch_external();
        }
        self.payload = Some(emoji);
        if !self.claim() {
            eprintln!("[🐛DEBUG] clipboard: failed to claim CLIPBOARD ownership");
            self.payload = None;
            return;
        }
        self.owns = true;
        self.deliver_start = Some(Instant::now());
        self.data_served_at = None;
        self.send_ctrl_v();
    }

    /// Once the paste has been consumed (data request answered + grace) or the backstop fires,
    /// swap to the saved content and keep serving it — the actual restore.
    fn maybe_restore(&mut self) {
        let Some(start) = self.deliver_start else {
            return;
        };
        let done = match self.data_served_at {
            Some(t) => t.elapsed() >= GRACE,
            None => start.elapsed() >= HARD_CAP,
        };
        if !done {
            return;
        }
        if self.data_served_at.is_none() {
            eprintln!("[🐛DEBUG] clipboard: paste not consumed before backstop");
        }
        self.deliver_start = None;
        self.data_served_at = None;
        // Restore: serve the saved content henceforth (keep ownership). `saved` is preserved
        // until SelectionClear so further emoji restore to the same prior content.
        self.payload = self.saved.clone();
        match &self.payload {
            Some(p) => debug_log!("[🐛DEBUG] clipboard: restored prior selection ({} bytes)", p.len()),
            None => {
                // Nothing to restore -> release ownership (clipboard becomes empty).
                self.release();
                self.owns = false;
            }
        }
    }

    fn pump_events(&mut self) {
        while let Some(ev) = self.poll_event() {
            // SAFETY: `type_` is the common initial member of the XEvent union, valid for any
            // variant.
            match unsafe { ev.type_ } {
                xlib::SelectionRequest => {
                    // SAFETY: the tag confirms the selection_request member is active.
                    let req = unsafe { ev.selection_request };
                    let is_data = req.target != self.targets;
                    self.serve_request(&req);
                    if self.deliver_start.is_some() && is_data {
                        self.data_served_at = Some(Instant::now());
                    }
                }
                xlib::SelectionClear => {
                    // The user (or another app) took the clipboard: stop owning and serving,
                    // and drop the saved content — it's no longer "prior".
                    self.owns = false;
                    self.payload = None;
                    self.saved = None;
                    self.deliver_start = None;
                    self.data_served_at = None;
                }
                _ => {}
            }
        }
    }

    /// Answer one SelectionRequest from the current `payload`.
    fn serve_request(&self, req: &xlib::XSelectionRequestEvent) {
        let payload = match &self.payload {
            Some(p) => p.as_slice(),
            None => {
                self.send_notify(req, 0); // nothing to serve -> refuse
                return;
            }
        };
        // Obsolete-client convention: a None property means "store under the target atom".
        let property = if req.property == 0 { req.target } else { req.property };

        let answered = if req.target == self.targets {
            let supported = [self.utf8, xlib::XA_STRING, self.targets];
            // SAFETY: display/requestor/property valid; `supported` is a live [Atom; 3]
            // borrowed only for this synchronous call.
            unsafe {
                xlib::XChangeProperty(
                    self.dpy, req.requestor, property,
                    xlib::XA_ATOM, 32, xlib::PropModeReplace,
                    supported.as_ptr() as *const u8, supported.len() as i32,
                );
            }
            true
        } else if req.target == self.utf8 || req.target == xlib::XA_STRING {
            // SAFETY: as above; `payload` outlives this synchronous call.
            unsafe {
                xlib::XChangeProperty(
                    self.dpy, req.requestor, property,
                    req.target, 8, xlib::PropModeReplace,
                    payload.as_ptr(), payload.len() as i32,
                );
            }
            true
        } else {
            false
        };
        self.send_notify(req, if answered { property } else { 0 });
    }

    /// Synchronously fetch the current CLIPBOARD content (UTF8_STRING) from its owner, or None
    /// if we own it, there is no owner, or the owner refuses / times out.
    fn fetch_external(&self) -> Option<Vec<u8>> {
        // SAFETY: live display; reading the current selection owner is a simple query.
        let owner = unsafe { xlib::XGetSelectionOwner(self.dpy, self.clipboard) };
        if owner == 0 || owner == self.win {
            return None;
        }
        // SAFETY: ask the owner to convert CLIPBOARD as UTF8_STRING into our scratch property.
        unsafe {
            xlib::XDeleteProperty(self.dpy, self.win, self.prop);
            xlib::XConvertSelection(self.dpy, self.clipboard, self.utf8, self.prop, self.win, xlib::CurrentTime);
            xlib::XFlush(self.dpy);
        }
        let deadline = Instant::now() + FETCH_TIMEOUT;
        loop {
            if Instant::now() > deadline {
                return None;
            }
            match self.poll_event() {
                // SAFETY: union type tag read; see pump_events.
                Some(ev) if unsafe { ev.type_ } == xlib::SelectionNotify => {
                    // SAFETY: tag confirms the `selection` member is active.
                    let sn = unsafe { ev.selection };
                    if sn.property == 0 {
                        return None; // owner refused the conversion
                    }
                    return self.read_prop();
                }
                Some(_) => {} // unrelated event while fetching; we don't own yet, so ignore
                None => thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    /// Read our scratch property as raw bytes, then delete it.
    fn read_prop(&self) -> Option<Vec<u8>> {
        let mut actual_type: xlib::Atom = 0;
        let mut actual_format: i32 = 0;
        let mut nitems: u64 = 0;
        let mut bytes_after: u64 = 0;
        let mut data: *mut u8 = ptr::null_mut();
        // SAFETY: all out-params are valid locals; long_length is in 32-bit units (16M = 64MB
        // ceiling, far above any clipboard text). On success `data` is a server buffer we free.
        let status = unsafe {
            xlib::XGetWindowProperty(
                self.dpy, self.win, self.prop,
                0, 1 << 24, xlib::False, 0, /* AnyPropertyType */
                &mut actual_type, &mut actual_format, &mut nitems, &mut bytes_after, &mut data,
            )
        };
        if status != xlib::Success as i32 || data.is_null() {
            return None;
        }
        // SAFETY: format 8 => `nitems` bytes at `data`; copy out, then free + delete the prop.
        let bytes = unsafe { std::slice::from_raw_parts(data, nitems as usize).to_vec() };
        unsafe {
            xlib::XFree(data as *mut _);
            xlib::XDeleteProperty(self.dpy, self.win, self.prop);
        }
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
    }

    // --- Small, documented FFI helpers ---

    fn claim(&self) -> bool {
        // SAFETY: clipboard atom interned and win created on this display in open().
        unsafe {
            xlib::XSetSelectionOwner(self.dpy, self.clipboard, self.win, xlib::CurrentTime);
            xlib::XFlush(self.dpy);
            xlib::XGetSelectionOwner(self.dpy, self.clipboard) == self.win
        }
    }

    fn release(&self) {
        // SAFETY: releasing ownership to None is always valid.
        unsafe {
            xlib::XSetSelectionOwner(self.dpy, self.clipboard, 0, xlib::CurrentTime);
            xlib::XFlush(self.dpy);
        }
    }

    fn send_ctrl_v(&self) {
        // SAFETY: live display; XTest synthesizes events into the focused window with no
        // ownership of the keycodes.
        unsafe {
            let ctrl = xlib::XKeysymToKeycode(self.dpy, keysym::XK_Control_L as u64);
            let v = xlib::XKeysymToKeycode(self.dpy, keysym::XK_v as u64);
            xtest::XTestFakeKeyEvent(self.dpy, ctrl as u32, xlib::True, 0);
            xtest::XTestFakeKeyEvent(self.dpy, v as u32, xlib::True, 0);
            xtest::XTestFakeKeyEvent(self.dpy, v as u32, xlib::False, 0);
            xtest::XTestFakeKeyEvent(self.dpy, ctrl as u32, xlib::False, 0);
            xlib::XFlush(self.dpy);
        }
    }

    /// Reply to a requestor with SelectionNotify; `property` is the granted atom, or 0 (None)
    /// to refuse.
    fn send_notify(&self, req: &xlib::XSelectionRequestEvent, property: xlib::Atom) {
        let mut notify = xlib::XSelectionEvent {
            type_: xlib::SelectionNotify,
            serial: 0,
            send_event: xlib::True,
            display: self.dpy,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property,
            time: req.time,
        };
        // SAFETY: `notify` is fully initialized above; XSendEvent copies it and keeps no pointer.
        unsafe {
            xlib::XSendEvent(self.dpy, req.requestor, xlib::False, 0,
                &mut notify as *mut _ as *mut xlib::XEvent);
            xlib::XFlush(self.dpy);
        }
    }

    /// Pop the next queued X event, or None if the queue is currently empty.
    fn poll_event(&self) -> Option<xlib::XEvent> {
        // SAFETY: when XPending reports > 0, XNextEvent fully initializes the event it writes.
        unsafe {
            if xlib::XPending(self.dpy) > 0 {
                let mut ev: xlib::XEvent = std::mem::zeroed();
                xlib::XNextEvent(self.dpy, &mut ev);
                Some(ev)
            } else {
                None
            }
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: dpy was opened in open() and is not used after this.
        unsafe { xlib::XCloseDisplay(self.dpy) };
    }
}

/// Intern an atom from a static NUL-terminated name.
///
/// # Safety
/// `dpy` must be a live Xlib display and `name` a NUL-terminated byte string.
unsafe fn intern(dpy: *mut xlib::Display, name: &[u8]) -> xlib::Atom {
    xlib::XInternAtom(dpy, name.as_ptr() as *const _, xlib::False)
}
