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

pub fn listen_to_keyd(
    layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
    current_layout: Arc<Mutex<String>>,
    config_path: Arc<String>,
    reload_flag: Arc<AtomicBool>,
    config_reload_flag: Arc<AtomicBool>,
    last_reparse: Arc<Mutex<Instant>>,
) -> Result<(), String> {
    loop {
        match connect_and_listen(&layout_map, &current_layout, &config_path, &reload_flag, &config_reload_flag, &last_reparse) {
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

fn connect_and_listen(
    layout_map: &Arc<RwLock<HashMap<String, LayoutInfo>>>,
    current_layout: &Arc<Mutex<String>>,
    config_path: &str,
    reload_flag: &Arc<AtomicBool>,
    config_reload_flag: &Arc<AtomicBool>,
    last_reparse: &Arc<Mutex<Instant>>,
) -> Result<(), String> {
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

    // Read and parse incoming events with proper stream framing.
    // Unix stream sockets do not preserve message boundaries, so we buffer
    // incoming data and only process complete newline-terminated lines.
    let mut buffer = [0u8; 512];
    let mut pending = String::new();

    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                return Err("Connection closed by keyd".to_string());
            }
            Ok(n) => {
                let data = String::from_utf8_lossy(&buffer[..n]);
                debug_log!("[🐛DEBUG] IPC received {} bytes: {:?}", n, data);

                pending.push_str(&data);

                // Process only complete newline-terminated lines
                while let Some(newline_idx) = pending.find('\n') {
                    let line = pending[..newline_idx].to_string();
                    pending.drain(..=newline_idx);

                    if !line.is_empty() {
                        debug_log!("[🐛DEBUG] IPC line: {:?}", line);
                        handle_layout_event(&line, layout_map, current_layout, config_path, reload_flag, config_reload_flag, last_reparse);
                    }
                }
            }
            Err(e) => {
                return Err(format!("Read error: {}", e));
            }
        }
    }
}

fn handle_layout_event(
    line: &str,
    layout_map: &Arc<RwLock<HashMap<String, LayoutInfo>>>,
    current_layout: &Arc<Mutex<String>>,
    config_path: &str,
    reload_flag: &Arc<AtomicBool>,
    config_reload_flag: &Arc<AtomicBool>,
    last_reparse: &Arc<Mutex<Instant>>,
) {
    if let Some(layout_name) = line.strip_prefix('/') {
        // Layout change event
        let layout_name = layout_name.trim();

        // Update current layout for GTK thread to see
        if let Ok(mut layout) = current_layout.lock() {
            *layout = layout_name.to_string();
        }

        // If it's /main (after reload), schedule config re-parse and XTyper rescan
        if layout_name == "main" {
            debug_log!("[🐛DEBUG] Detected reload signal (/main event)");
            if try_claim_reparse(last_reparse) {
                reparse_config(layout_map, config_path, config_reload_flag.clone());
            } else {
                debug_log!("[🐛DEBUG] IPC /main reparse suppressed by debounce");
            }
            reload_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

pub fn watch_config_file(
    layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
    config_path: &str,
    config_reload_flag: Arc<AtomicBool>,
    last_reparse: Arc<Mutex<Instant>>,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = recommended_watcher(tx)
        .map_err(|e| format!("Failed to create watcher: {}", e))?;

    watcher.watch(Path::new(config_path), RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch config: {}", e))?;

    debug_log!("[🐛DEBUG] Watching {} for changes", config_path);

    loop {
        match rx.recv() {
            Ok(_event) => {
                if try_claim_reparse(&last_reparse) {
                    debug_log!("[🐛DEBUG] Config file changed, reparsing...");
                    reparse_config(&layout_map, config_path, config_reload_flag.clone());
                } else {
                    debug_log!("[🐛DEBUG] inotify reparse suppressed by debounce");
                }
            }
            Err(e) => {
                eprintln!("Watcher error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Returns true and updates the timestamp if enough time has elapsed since the last reparse.
/// Both the IPC and inotify paths call this to share a single debounce window.
fn try_claim_reparse(last_reparse: &Arc<Mutex<Instant>>) -> bool {
    const DEBOUNCE: Duration = Duration::from_millis(200);
    let now = Instant::now();
    if let Ok(mut last) = last_reparse.lock() {
        if now.duration_since(*last) > DEBOUNCE {
            *last = now;
            return true;
        }
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
