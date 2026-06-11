use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use notify::{Watcher, RecursiveMode, recommended_watcher};
use crate::config::{ConfigParser, LayoutInfo};
use crate::debug_log;

const KEYD_SOCKET_PATH: &str = "/var/run/keyd.socket";
const MAX_IPC_MESSAGE_SIZE: usize = 4096;

/// Matches keyd's struct ipc_message from keyd.h
/// Layout: { int type, uint32_t timeout, char data[4096], size_t sz }
#[repr(C)]
struct IpcMessage {
    msg_type: i32,         // enum (int-sized on Linux x86_64)
    timeout: u32,
    data: [u8; MAX_IPC_MESSAGE_SIZE],
    sz: usize,             // size_t = 8 bytes on x86_64
}

// Compile-time assertion: IpcMessage must be exactly 4112 bytes to match keyd's C struct
const _: () = assert!(std::mem::size_of::<IpcMessage>() == 4112);

/// IPC message type values from keyd's enum
const IPC_LAYER_LISTEN: i32 = 6; // 0=SUCCESS,1=FAIL,2=BIND,3=INPUT,4=MACRO,5=RELOAD,6=LAYER_LISTEN

/// Shared mutable state passed to both the keyd IPC and inotify threads.
#[derive(Clone)]
pub struct SharedState {
    pub layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
    pub current_layout: Arc<Mutex<String>>,
    pub config_path: Arc<String>,
    pub reload_flag: Arc<AtomicBool>,
    pub config_reload_flag: Arc<AtomicBool>,
    pub last_reparse: Arc<Mutex<Instant>>,
}

pub fn listen_to_keyd(state: SharedState) -> Result<(), String> {
    loop {
        match connect_and_listen(&state) {
            Ok(()) => {
                thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                debug_log!("[🐛DEBUG] keyd connection error: {}", e);
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn connect_and_listen(state: &SharedState) -> Result<(), String> {
    let mut stream = UnixStream::connect(KEYD_SOCKET_PATH)
        .map_err(|e| format!("Failed to connect to keyd socket: {}", e))?;

    // Send IPC_LAYER_LISTEN as a full struct ipc_message
    let msg = IpcMessage {
        msg_type: IPC_LAYER_LISTEN,
        timeout: 0,
        data: [0u8; MAX_IPC_MESSAGE_SIZE],
        sz: 0,
    };

    let msg_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &msg as *const IpcMessage as *const u8,
            std::mem::size_of::<IpcMessage>(),
        )
    };

    stream.write_all(msg_bytes)
        .map_err(|e| format!("Failed to send listen request: {}", e))?;

    debug_log!("[🐛DEBUG] Connected to keyd, listening for layout changes (sent {} byte message)", msg_bytes.len());

    // Buffer raw bytes; decode per complete newline-terminated line to avoid splitting
    // multi-byte characters across reads (keyd names are ASCII, but be correct).
    let mut raw_buf = [0u8; 512];
    let mut pending: Vec<u8> = Vec::new();

    loop {
        match stream.read(&mut raw_buf) {
            Ok(0) => {
                return Err("Connection closed by keyd".to_string());
            }
            Ok(n) => {
                pending.extend_from_slice(&raw_buf[..n]);
                debug_log!("[🐛DEBUG] IPC received {} bytes", n);

                while let Some(newline_idx) = pending.iter().position(|&b| b == b'\n') {
                    let line_bytes = pending[..newline_idx].to_vec();
                    pending.drain(..=newline_idx);

                    if line_bytes.is_empty() {
                        continue;
                    }
                    match String::from_utf8(line_bytes) {
                        Ok(line) => {
                            debug_log!("[🐛DEBUG] IPC line: {:?}", line);
                            handle_layout_event(&line, state);
                        }
                        Err(_) => {
                            eprintln!("Warning: received invalid UTF-8 from keyd IPC");
                        }
                    }
                }
            }
            Err(e) => {
                return Err(format!("Read error: {}", e));
            }
        }
    }
}

fn handle_layout_event(line: &str, state: &SharedState) {
    if let Some(layout_name) = line.strip_prefix('/') {
        let layout_name = layout_name.trim();

        // M-3: recover from poisoned mutex rather than silently skipping the update
        let mut layout = state.current_layout.lock()
            .unwrap_or_else(|e| e.into_inner());
        *layout = layout_name.to_string();
        drop(layout);

        if layout_name == "main" {
            debug_log!("[🐛DEBUG] Detected reload signal (/main event)");
            debounced_reparse(&state.layout_map, &state.config_path, state.config_reload_flag.clone(), &state.last_reparse, "IPC /main");
            state.reload_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

pub fn watch_config_file(
    layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
    config_path: Arc<String>,
    config_reload_flag: Arc<AtomicBool>,
    last_reparse: Arc<Mutex<Instant>>,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = recommended_watcher(tx)
        .map_err(|e| format!("Failed to create watcher: {}", e))?;

    watcher.watch(Path::new(config_path.as_str()), RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch config: {}", e))?;

    debug_log!("[🐛DEBUG] Watching {} for changes", config_path);

    loop {
        match rx.recv() {
            Ok(_event) => {
                debounced_reparse(&layout_map, &config_path, config_reload_flag.clone(), &last_reparse, "inotify");
            }
            Err(e) => {
                eprintln!("Watcher error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Attempt a config reparse if the shared 200ms debounce window allows it.
fn debounced_reparse(
    layout_map: &Arc<RwLock<HashMap<String, LayoutInfo>>>,
    config_path: &str,
    config_reload_flag: Arc<AtomicBool>,
    last_reparse: &Arc<Mutex<Instant>>,
    source: &str,
) {
    if try_claim_reparse(last_reparse) {
        debug_log!("[🐛DEBUG] {} triggered config reparse", source);
        reparse_config(layout_map, config_path, config_reload_flag);
    } else {
        debug_log!("[🐛DEBUG] {} reparse suppressed by debounce", source);
    }
}

/// Returns true and updates the timestamp if enough time has elapsed since the last reparse.
/// Both the IPC and inotify paths call this to share a single debounce window.
fn try_claim_reparse(last_reparse: &Arc<Mutex<Instant>>) -> bool {
    const DEBOUNCE: Duration = Duration::from_millis(200);
    let now = Instant::now();
    // M-3 / ISSUES G: recover from poisoned mutex rather than permanently suppressing reparsing
    let mut last = last_reparse.lock().unwrap_or_else(|e| e.into_inner());
    if now.duration_since(*last) > DEBOUNCE {
        *last = now;
        return true;
    }
    false
}

fn reparse_config(
    layout_map: &Arc<RwLock<HashMap<String, LayoutInfo>>>,
    config_path: &str,
    config_reload_flag: Arc<AtomicBool>,
) {
    match ConfigParser::parse(config_path) {
        Ok(new_map) => {
            match layout_map.write() {
                Ok(mut map) => {
                    let count = new_map.len();
                    *map = new_map;
                    debug_log!("[🐛DEBUG] Config reparsed successfully, {} layouts loaded", count);
                    config_reload_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Error: layout map lock poisoned during reparse: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("Error: failed to reparse config: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_claim_reparse_allows_first_call() {
        let last = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
        assert!(try_claim_reparse(&last));
    }

    #[test]
    fn test_try_claim_reparse_suppresses_second_within_debounce() {
        let last = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
        assert!(try_claim_reparse(&last));
        // Immediate second call is within the 200ms window
        assert!(!try_claim_reparse(&last));
    }

    #[test]
    fn test_try_claim_reparse_allows_after_debounce_expires() {
        let last = Arc::new(Mutex::new(Instant::now() - Duration::from_millis(201)));
        assert!(try_claim_reparse(&last));
    }
}
