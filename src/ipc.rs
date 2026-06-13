use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock, Mutex};
use std::thread;
use std::time::Duration;
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

/// Shared mutable state passed to the keyd IPC thread and GTK poll loop.
#[derive(Clone)]
pub struct SharedState {
    pub layout_map: Arc<RwLock<HashMap<String, LayoutInfo>>>,
    pub current_layout: Arc<Mutex<String>>,
    pub config_path: Arc<String>,
    pub reload_flag: Arc<AtomicBool>,
    pub config_reload_flag: Arc<AtomicBool>,
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
            // /main signals that keyd has finished loading its config. Only now is
            // it correct to re-parse — inotify fires before keyd reads the updated
            // file and would produce a blank or stale overlay (see ANTI-PATTERNS #20).
            debug_log!("[🐛DEBUG] Detected reload signal (/main event)");
            reparse_config(&state.layout_map, &state.config_path, state.config_reload_flag.clone());
            state.reload_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
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

