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
