use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

use crate::socket::client_socket_path;

pub fn send_type(character: &str) {
    let path = match client_socket_path() {
        Some(p) => p,
        None => {
            eprintln!("Error: rologlyphex daemon not running (no socket found)");
            std::process::exit(1);
        }
    };

    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: cannot connect to rologlyphex daemon ({:?}): {}", path, e);
            std::process::exit(1);
        }
    };

    if let Err(e) = stream.write_all(character.as_bytes()) {
        eprintln!("Error: failed to send character: {}", e);
        std::process::exit(1);
    }

    // Signal end-of-message so server doesn't block waiting for more data
    let _ = stream.shutdown(Shutdown::Write);
}
