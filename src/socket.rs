use std::path::PathBuf;

const SOCKET_NAME: &str = "rologlyphex.sock";

/// Typed socket commands, shared by client (encode) and server (decode).
pub enum Command {
    Type(char),
    Show,
}

impl Command {
    pub fn encode(&self) -> String {
        match self {
            Command::Type(ch) => format!("type {}\n", ch),
            Command::Show => "show\n".to_string(),
        }
    }

    pub fn decode(s: &str) -> Option<Command> {
        let s = s.trim();
        if s == "show" {
            Some(Command::Show)
        } else if let Some(rest) = s.strip_prefix("type ") {
            rest.chars().next().map(Command::Type)
        } else {
            None
        }
    }
}

pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join(SOCKET_NAME);
    }
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

    // 2. Fall back to scanning /run/user/*/rologlyphex.sock (keep as last resort)
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
