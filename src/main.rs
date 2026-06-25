mod client;
mod config;
mod layers;
mod monitor;
mod overlay;
mod server;
mod settings;
mod socket;
mod wmprops;
mod xerror;
mod xgrab;
mod xtype;

use std::sync::{Arc, RwLock, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use gtk4::prelude::*;
use gtk4::Application;
use crate::settings::AppSettings;

pub static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if $crate::DEBUG_ENABLED.load(::std::sync::atomic::Ordering::Relaxed) {
            eprintln!($($arg)*);
        }
    };
}

/// Returns the first char of `s`, or `None` if empty.
/// Prints a warning to stderr if `s` contains more than one character.
pub fn first_char_of(s: &str, context: &str) -> Option<char> {
    let mut chars = s.chars();
    let ch = chars.next();
    if ch.is_some() && chars.next().is_some() {
        eprintln!("Warning: {} '{}' has extra characters, using first only", context, s);
    }
    ch
}

// Mode::Daemon carries the validated layers.toml path so run_daemon never needs to unwrap.
enum Mode {
    Daemon(Arc<AppSettings>, String),
    Type(String),
    Show,
}

fn print_help() {
    println!("Usage:");
    println!("  rologlyphex [-c <path>] [-t <ms>] [-s <W>] [-v]   Start overlay daemon");
    println!("                                                      (-c is the layers.toml path)");
    println!("  rologlyphex type <char>                             Type a character via the running daemon");
    println!("  rologlyphex show                                    Re-show the overlay in its current state");
    println!();
    println!("Daemon options (can also be set in ~/.config/rologlyphex/config.toml):");
    println!("  -c, --config <path>   Path to layers config (default: ~/.config/rologlyphex/layers.toml)");
    println!("  -t, --timeout <ms>    Overlay dismiss timeout in milliseconds (default: 3000)");
    println!("  -s, --size <W>        Overlay window width (height calculated, default: 600)");
    println!("  -m, --monitor <id>    Monitor to show the overlay on (connector name, model, or");
    println!("                        index; default: rightmost monitor)");
    println!("      --corner <pos>    Corner to align to: top-left, top-right, bottom-left,");
    println!("                        bottom-right (default: top-right)");
    println!("  -v, --verbose         Enable debug logging");
    println!("  -h, --help            Show this help");
}

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut iter = args.iter().peekable();

    // Check for client subcommands
    if let Some(first) = iter.peek() {
        if first.as_str() == "show" {
            return Mode::Show;
        }
        if first.as_str() == "type" {
            iter.next(); // consume "type"
            // Consume optional --verbose before the character
            while let Some(arg) = iter.peek() {
                if arg.as_str() == "--verbose" || arg.as_str() == "-v" {
                    DEBUG_ENABLED.store(true, Ordering::Relaxed);
                    iter.next();
                } else {
                    break;
                }
            }
            if let Some(arg) = iter.next() {
                let first_char = first_char_of(arg, "'type' argument").unwrap_or_else(|| {
                    eprintln!("Error: 'type' argument must be exactly one Unicode character");
                    std::process::exit(1);
                });
                return Mode::Type(first_char.to_string());
            } else {
                eprintln!("Error: 'type' requires a character argument");
                eprintln!("Usage: rologlyphex type <char>");
                std::process::exit(1);
            }
        }
    }

    // Daemon mode: load settings from config file first
    let mut settings = AppSettings::load();

    let mut cli_layers: Option<String> = None;
    let mut cli_timeout_ms: Option<u64> = None;
    let mut cli_size: Option<String> = None;
    let mut cli_verbose: Option<bool> = None;
    let mut cli_monitor: Option<String> = None;
    let mut cli_corner: Option<String> = None;

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--verbose" | "-v" => {
                cli_verbose = Some(true);
            }
            "--config" | "-c" => {
                cli_layers = Some(iter.next().unwrap_or_else(|| {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }).clone());
            }
            "--timeout" | "-t" => {
                let val = iter.next().unwrap_or_else(|| {
                    eprintln!("Error: --timeout requires a value in milliseconds");
                    std::process::exit(1);
                });
                cli_timeout_ms = Some(val.parse().unwrap_or_else(|_| {
                    eprintln!("Error: --timeout must be a number (milliseconds)");
                    std::process::exit(1);
                }));
            }
            "--size" | "-s" => {
                cli_size = Some(iter.next().unwrap_or_else(|| {
                    eprintln!("Error: --size requires a width value (e.g. 600)");
                    std::process::exit(1);
                }).clone());
            }
            "--monitor" | "-m" => {
                cli_monitor = Some(iter.next().unwrap_or_else(|| {
                    eprintln!("Error: --monitor requires an identifier (connector name, model, or index)");
                    std::process::exit(1);
                }).clone());
            }
            "--corner" => {
                cli_corner = Some(iter.next().unwrap_or_else(|| {
                    eprintln!("Error: --corner requires a value (top-left, top-right, bottom-left, bottom-right)");
                    std::process::exit(1);
                }).clone());
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {}", other);
                print_help();
                std::process::exit(1);
            }
        }
    }

    settings.merge_cli(cli_layers, cli_timeout_ms, cli_size, cli_verbose, cli_monitor, cli_corner);

    if settings.verbose.unwrap_or(false) {
        DEBUG_ENABLED.store(true, Ordering::Relaxed);
    }

    let layers_path = settings.layers.clone().unwrap_or_else(|| {
        AppSettings::default_layers_path().to_string_lossy().into_owned()
    });

    Mode::Daemon(Arc::new(settings), layers_path)
}

const DEFAULT_WIDTH: i32 = 600;

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        eprintln!("[CRASH] rologlyphex panicked at {}: {}", location, message);
        eprintln!("[CRASH] Set RUST_BACKTRACE=1 for a backtrace.");
    }));
}

fn main() {
    install_panic_hook();
    match parse_args() {
        Mode::Type(character) => {
            client::send_type(&character);
        }
        Mode::Show => {
            client::send_show();
        }
        Mode::Daemon(args, layers_path) => {
            run_daemon(args, layers_path);
        }
    }
}

fn run_daemon(settings: Arc<AppSettings>, layers_path: String) {
    // Load the layers glyph-map / navigation-ring config (rologlyphex owns this; keyd is gone).
    let cfg = match layers::LayersConfig::load(&layers_path) {
        Ok(c) => {
            debug_log!("[🐛DEBUG] Loaded {} layers from {}", c.layer_order.len(), layers_path);
            c
        }
        Err(e) => {
            eprintln!("Error loading layers config '{}': {}", layers_path, e);
            std::process::exit(1);
        }
    };

    let initial_layer = cfg.layer_order[0].clone();
    let layout_map = Arc::new(RwLock::new(cfg.overlay.clone()));
    let cfg = Arc::new(cfg);

    let timeout_ms = settings.timeout.unwrap_or(3000);
    let window_width = if let Some(size_str) = &settings.size {
        let width = size_str.trim().parse::<i32>().unwrap_or(-1);
        if width <= 0 {
            eprintln!("Error: Invalid size '{}', must be a positive integer", size_str);
            std::process::exit(1);
        }
        width
    } else {
        DEFAULT_WIDTH
    };

    // Shared state. The grab thread publishes the active layer name to `current_layout`,
    // which the GTK poll loop watches to show the overlay.
    let current_layout = Arc::new(Mutex::new(initial_layer.clone()));
    let show_requested = Arc::new(AtomicBool::new(false));
    // Set by the grab thread (lazy remap mode) while it rebuilds the keymap; the GTK poll
    // shows/hides the "Please Wait" overlay accordingly.
    let please_wait = Arc::new(AtomicBool::new(false));

    // GTK Application setup
    let app = Application::builder()
        .application_id("com.extollit.rologlyphex")
        .build();

    let layout_map_gtk = layout_map.clone();
    let current_layout_gtk = current_layout.clone();
    let show_requested_gtk = show_requested.clone();
    let please_wait_gtk = please_wait.clone();
    let monitor_pref = settings.monitor.clone();
    let corner = settings.corner.clone();
    let initial_layer_gtk = initial_layer.clone();
    app.connect_activate(move |app| {
        // Hold guard keeps the app alive even when all windows are hidden.
        // Captured by the timer closure below so it lives for the app's lifetime.
        let hold = app.hold();

        // Install our non-fatal X error handlers now that GDK has opened the display (GTK
        // sets its own during init; calling here makes ours win process-wide, covering the
        // overlay, the XTyper display, and the key-grab display). See xerror.rs.
        xerror::install();

        let window = overlay::OverlayWindow::new(
            app,
            layout_map_gtk.clone(),
            timeout_ms,
            window_width,
            monitor_pref.clone(),
            corner.clone(),
        );

        // Poll for layer changes (published by the grab thread) and update the overlay.
        let current_layout = current_layout_gtk.clone();
        let window_clone = window.clone();
        let show_requested = show_requested_gtk.clone();
        let please_wait = please_wait_gtk.clone();
        // Starts equal to the published initial layer, so no overlay shows until the user
        // actually navigates.
        let mut last_layout = initial_layer_gtk.clone();
        let mut wait_shown = false;

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            // Hold guard captured here -- prevents app from quitting while timer runs
            let _ = &hold;

            let show = show_requested.swap(false, Ordering::Relaxed);

            // Show/hide the "Please Wait" overlay (lazy remap mode) on flag edges.
            let wait = please_wait.load(Ordering::Relaxed);
            if wait && !wait_shown {
                window_clone.show_please_wait();
                wait_shown = true;
            } else if !wait && wait_shown {
                window_clone.hide_please_wait();
                wait_shown = false;
            }

            // M-3: recover from poisoned mutex rather than silently freezing the poll loop
            let layout = current_layout.lock().unwrap_or_else(|e| e.into_inner());
            if *layout != last_layout {
                debug_log!("[🐛DEBUG] Layout changed to: {}", *layout);
                window_clone.show_layout(&layout);
                last_layout = layout.clone();
            } else if show {
                debug_log!("[🐛DEBUG] Show overlay requested for layout: {}", last_layout);
                window_clone.show_layout(&last_layout);
            }
            drop(layout);

            glib::ControlFlow::Continue
        });

        // Graceful shutdown on SIGTERM/SIGINT: remove socket and quit
        let app_quit = app.clone();
        glib::unix_signal_add_local(libc::SIGTERM, move || {
            debug_log!("[🐛DEBUG] Received SIGTERM, shutting down");
            let _ = std::fs::remove_file(socket::socket_path());
            app_quit.quit();
            glib::ControlFlow::Break
        });
        let app_quit2 = app.clone();
        glib::unix_signal_add_local(libc::SIGINT, move || {
            debug_log!("[🐛DEBUG] Received SIGINT, shutting down");
            let _ = std::fs::remove_file(socket::socket_path());
            app_quit2.quit();
            glib::ControlFlow::Break
        });
    });

    // Spawn the key-grab input thread (replaces the keyd IPC listener). It grabs F13–F24,
    // owns the layer ring, and types via XTest.
    let cfg_grab = cfg.clone();
    let current_layout_grab = current_layout.clone();
    let nav_settle_ms = settings.nav_settle_ms.unwrap_or(160).min(i32::MAX as u64) as i32;
    let remap_mode = match settings.remap_mode.as_deref() {
        Some("debounce") => xgrab::RemapMode::Debounce,
        Some("lazy") | None => xgrab::RemapMode::Lazy,
        Some(other) => {
            eprintln!("Warning: unknown remap_mode '{}', using 'lazy'", other);
            xgrab::RemapMode::Lazy
        }
    };
    let please_wait_grab = please_wait.clone();
    thread::spawn(move || {
        if let Err(e) = xgrab::run(cfg_grab, current_layout_grab, nav_settle_ms, remap_mode, please_wait_grab) {
            eprintln!("Key-grab thread error: {}", e);
        }
    });

    // Spawn the socket server thread (manual `rologlyphex type` / `show`).
    let show_requested_server = show_requested.clone();
    thread::spawn(move || {
        if let Err(e) = server::run_server(show_requested_server) {
            eprintln!("Socket server error: {}", e);
        }
    });

    // Run GTK main loop
    app.run_with_args::<String>(&[]);
}
