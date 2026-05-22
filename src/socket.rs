use std::path::PathBuf;
use crate::debug_log;
use std::time::Duration;
use dbus::blocking::Connection;

const SOCKET_NAME: &str = "rologlyphex.sock";

fn active_seat0_uid() -> Option<u32> {
    let conn = Connection::new_system().ok()?;
    let proxy = conn.with_proxy("org.freedesktop.login1", "/org/freedesktop/login1", Duration::from_millis(2000));

    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    let (sessions,): (Vec<(String, u32, String, String, dbus::Path<'static>)>,) =
        proxy.method_call("org.freedesktop.login1.Manager", "ListSessions", ()).ok()?;

    for (_session_id, uid, _user, seat_id, object_path) in sessions {
        if seat_id == "seat0" {
            let session_proxy = conn.with_proxy("org.freedesktop.login1", object_path, Duration::from_millis(1000));

            let active: bool = match session_proxy.get("org.freedesktop.login1.Session", "Active") {
                Ok(v) => v,
                Err(_) => continue,
            };
            let session_type: String = match session_proxy.get("org.freedesktop.login1.Session", "Type") {
                Ok(v) => v,
                Err(_) => continue,
            };

            if active && session_type == "x11" {
                debug_log!("[🐛DEBUG] Found active seat0 X11 session for UID {}", uid);
                return Some(uid);
            }
        }
    }

    None
}

pub fn socket_path() -> PathBuf {
    // Use a fixed path in XDG_RUNTIME_DIR so both daemon (user session) and
    // CLI client (possibly run as root by keyd) can agree on the location.
    // The daemon creates the socket; the client just needs to know the path.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/{}", uid, SOCKET_NAME))
}

/// Socket path for the client side — tries XDG_RUNTIME_DIR first,
/// then scans /run/user/*/rologlyphex.sock for the first match.
pub fn client_socket_path() -> Option<PathBuf> {
    // 1. If we're the same user as the daemon, the normal path works
    let path = socket_path();
    if path.exists() {
        return Some(path);
    }

    // 2. Running as root (e.g. from keyd command()) — try to find the active seat0 user via D-Bus
    if let Some(uid) = active_seat0_uid() {
        let path = PathBuf::from(format!("/run/user/{}/{}", uid, SOCKET_NAME));
        if path.exists() {
            return Some(path);
        }
    }

    // 3. Fall back to scanning /run/user/*/rologlyphex.sock (keep as last resort)
    if let Ok(entries) = std::fs::read_dir("/run/user") {
        for entry in entries.flatten() {
            let candidate = entry.path().join(SOCKET_NAME);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}
