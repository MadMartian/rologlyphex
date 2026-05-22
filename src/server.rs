use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;

use crate::socket::socket_path;
use crate::xtype::XTyper;
use crate::debug_log;

use crate::settings::AppSettings;
use std::sync::Arc;

/// Maximum bytes to read from a client connection.
/// A single UTF-8 character is at most 4 bytes; 16 bytes is generous.
const MAX_MESSAGE_SIZE: usize = 16;

pub fn run_server(_settings: Arc<AppSettings>) -> Result<(), String> {
    let path = socket_path();

    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Failed to remove stale socket {:?}: {}", path, e))?;
    }

    let listener = UnixListener::bind(&path)
        .map_err(|e| format!("Failed to bind socket {:?}: {}", path, e))?;

    // Make socket accessible only to owner (Root/keyd has CAP_DAC_OVERRIDE)
    if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        eprintln!("Warning: Failed to set socket permissions to 0o600: {}", e);
    }

    let mut typer = XTyper::open()?;

    debug_log!("[🐛DEBUG] Listening on {:?}", path);

    for stream in listener.incoming() {
        match stream {
            Ok(mut conn) => {
                let mut buf = [0u8; MAX_MESSAGE_SIZE];
                let mut total = 0;
                loop {
                    match conn.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => total += n,
                        Err(_) => break,
                    }
                    if total >= buf.len() { break; }
                }
                if total == 0 {
                    continue;
                }
                match std::str::from_utf8(&buf[..total]) {
                    Ok(s) => {
                        let text = s.trim();
                        if text.is_empty() {
                            continue;
                        }
                        let mut chars = text.chars();
                        let ch = chars.next().unwrap();
                        if chars.next().is_some() {
                            eprintln!("Warning: received multiple characters '{}', typing only first '{}'", text, ch);
                        }
                        debug_log!("[🐛DEBUG] Typing: {:?}", ch);
                        typer.type_char(ch);
                    }
                    Err(e) => {
                        eprintln!("Warning: received invalid UTF-8 ({} bytes): {}", total, e);
                    }
                }
            }
            Err(e) => {
                debug_log!("[🐛DEBUG] Accept error: {}", e);
            }
        }
    }

    Ok(())
}
