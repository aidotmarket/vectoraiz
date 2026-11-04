//! Local-only framing and platform identity checks shared by the app and bridge.
pub mod channel;
pub mod protocol;
pub const REQUEST_CAP: usize = 1024 * 1024;
pub const RESPONSE_CAP: usize = 2 * 1024 * 1024;
pub const NO_ACCESS: &str = "vectorAIz has not given this app access.";
pub const NOT_RUNNING: &str = "Open vectorAIz to search your library.";

pub fn valid_client(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read only the bounded client credential, afresh for each tool call.
pub fn read_client_file(path: &std::path::Path) -> String {
    use std::io::Read;
    let mut bytes = Vec::new();
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    if !file.metadata().is_ok_and(|m| m.len() <= 256) {
        return String::new();
    }
    if file.take(256).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return String::new();
    };
    let id = text.trim();
    if valid_client(id) {
        id.to_owned()
    } else {
        String::new()
    }
}
