mod client;
mod config;
mod ipc;
mod overlay;
mod server;
mod settings;
mod socket;
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

// Mode::Daemon carries the validated keyd_config path so run_daemon never needs to unwrap.
enum Mode {
    Daemon(Arc<AppSettings>, String),
    Type(String),
    Show,
}

fn print_help() {
    println!("Usage:");
    println!("  rologlyphex [-c <path>] [-t <ms>] [-s <W>] [-v]   Start overlay daemon");
    println!("  rologlyphex type <char>                             Type a character via the running daemon");
    println!("  rologlyphex show                                    Re-show the overlay in its current state");
    println!();
    println!("Daemon options (can also be set in ~/.config/rologlyphex/config.toml):");
    println!("  -c, --config <path>   Path to keyd config file");
    println!("  -t, --timeout <ms>    Overlay dismiss timeout in milliseconds (default: 3000)");
    println!("  -s, --size <W>        Overlay window width (height calculated, default: 600)");
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

    let mut cli_config: Option<String> = None;
    let mut cli_timeout_ms: Option<u64> = None;
    let mut cli_size: Option<String> = None;
    let mut cli_verbose: Option<bool> = None;

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--verbose" | "-v" => {
                cli_verbose = Some(true);
            }
            "--config" | "-c" => {
                cli_config = Some(iter.next().unwrap_or_else(|| {
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

    settings.merge_cli(cli_config, cli_timeout_ms, cli_size, cli_verbose);

    if settings.verbose.unwrap_or(false) {
        DEBUG_ENABLED.store(true, Ordering::Relaxed);
    }

    let keyd_config = match settings.keyd_config.clone() {
        Some(p) => p,
        None => {
            eprintln!("Error: keyd config path must be specified via --config or in config.toml");
            print_help();
            std::process::exit(1);
        }
    };

    Mode::Daemon(Arc::new(settings), keyd_config)
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
        Mode::Daemon(args, keyd_config) => {
            run_daemon(args, keyd_config);
        }
    }
}

fn run_daemon(settings: Arc<AppSettings>, keyd_config: String) {
    // Parse config at startup
    let layout_map = match config::ConfigParser::parse(&keyd_config) {
        Ok(map) => {
            debug_log!("[🐛DEBUG] Parsed {} layouts from {}", map.len(), keyd_config);
            map
        }
        Err(e) => {
            eprintln!("Error parsing config: {}", e);
            std::process::exit(1);
        }
    };

    let layout_map = Arc::new(RwLock::new(layout_map));
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

    // Shared state accessed by multiple threads
    let shared = ipc::SharedState {
        layout_map: layout_map.clone(),
        current_layout: Arc::new(Mutex::new(String::from("main"))),
        config_path: Arc::new(keyd_config),
        reload_flag: Arc::new(AtomicBool::new(false)),
        config_reload_flag: Arc::new(AtomicBool::new(false)),
    };

    // Flag to signal on-demand overlay re-display
    let show_requested = Arc::new(AtomicBool::new(false));

    // GTK Application setup
    let app = Application::builder()
        .application_id("com.extollit.rologlyphex")
        .build();

    let shared_gtk = shared.clone();
    let show_requested_gtk = show_requested.clone();
    app.connect_activate(move |app| {
        // Hold guard keeps the app alive even when all windows are hidden.
        // Captured by the timer closure below so it lives for the app's lifetime.
        let hold = app.hold();

        let window = overlay::OverlayWindow::new(app, shared_gtk.layout_map.clone(), timeout_ms, window_width);

        // Poll for layout changes and update window
        let current_layout = shared_gtk.current_layout.clone();
        let window_clone = window.clone();
        let config_reload_flag = shared_gtk.config_reload_flag.clone();
        let show_requested = show_requested_gtk.clone();
        let mut last_layout = String::from("main");
        let mut first_change = true;

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            // Hold guard captured here -- prevents app from quitting while timer runs
            let _ = &hold;

            let config_reloaded = config_reload_flag.swap(false, Ordering::Relaxed);
            let show = show_requested.swap(false, Ordering::Relaxed);

            // M-3: recover from poisoned mutex rather than silently freezing the poll loop
            let layout = current_layout.lock().unwrap_or_else(|e| e.into_inner());
            if *layout != last_layout || config_reloaded {
                // Skip the initial /main event from keyd connect
                if first_change && *layout == "main" && !config_reloaded {
                    first_change = false;
                    last_layout = layout.clone();
                    return glib::ControlFlow::Continue;
                }
                first_change = false;
                debug_log!("[🐛DEBUG] Layout changed to: {} (reloaded={})", *layout, config_reloaded);
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

    // Spawn IPC listener thread
    let shared_ipc = shared.clone();
    thread::spawn(move || {
        if let Err(e) = ipc::listen_to_keyd(shared_ipc) {
            eprintln!("IPC listener error: {}", e);
        }
    });

    // Spawn type-command socket server thread
    let reload_flag_server = shared.reload_flag.clone();
    let show_requested_server = show_requested.clone();
    thread::spawn(move || {
        if let Err(e) = server::run_server(reload_flag_server, show_requested_server) {
            eprintln!("Socket server error: {}", e);
        }
    });

    // Run GTK main loop
    app.run_with_args::<String>(&[]);
}
