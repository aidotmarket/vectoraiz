use super::GateError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const APP_DISCLOSURE: [&str; 7] = [
    "Connect {App}",
    "{App} will be able to search your vectorAIz library and read whole documents from it. {App}'s provider handles what {App} reads.",
    "Claude Desktop will be able to search your vectorAIz library and read whole documents from it. Anthropic handles what Claude reads.",
    "You can remove the connection at any time in AI apps. What vectorAIz already sent to {App} can't be taken back.",
    "Connect {App}", "Cancel",
    "Claude Desktop then asks you to install vectorAIz's extension, and warns that an extension installed from a file can access your computer. The extension sends Claude's search and read requests to vectorAIz on this computer and returns the results to Claude.",
];
const fn hash() -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < APP_DISCLOSURE.len() {
        let bytes = APP_DISCLOSURE[i].as_bytes();
        let mut j = 0;
        while j < bytes.len() {
            h ^= bytes[j] as u64;
            h = h.wrapping_mul(0x100000001b3);
            j += 1;
        }
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}
pub const DISCLOSURE_VERSION_APP: u64 = hash();
pub fn app_disclosure_version() -> String {
    format!("{DISCLOSURE_VERSION_APP:016x}")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConsent {
    pub disclosure_version: String,
    pub confirmed_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppClient {
    pub client_id: String,
    pub app: String,
    pub name: String,
    pub consent: AppConsent,
    pub grants: Vec<String>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}
#[derive(Clone, Default)]
pub struct AppPathState {
    pub enabled: bool,
    pub clients: Vec<AppClient>,
    pub revoked_in_memory: HashSet<String>,
    pub off_in_memory: bool,
}
pub fn check_mcp(
    s: &AppPathState,
    client_id: &str,
    _tool: &str,
    target: &str,
) -> Result<(), GateError> {
    if !s.enabled || s.off_in_memory {
        return Err(GateError::PathOff);
    }
    let c = s
        .clients
        .iter()
        .find(|c| c.client_id == client_id)
        .ok_or(GateError::Revoked)?;
    if c.revoked_at.is_some()
        || s.revoked_in_memory.contains(client_id)
        || !c.grants.iter().any(|g| g == target)
    {
        return Err(GateError::Revoked);
    }
    if c.consent.disclosure_version != app_disclosure_version() {
        return Err(GateError::ConsentRequired);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gate_table_every_tool() {
        let client = AppClient {
            client_id: "a".repeat(32),
            app: "other".into(),
            name: "Test".into(),
            consent: AppConsent {
                disclosure_version: app_disclosure_version(),
                confirmed_at: "today".into(),
            },
            grants: vec!["library".into()],
            created_at: "today".into(),
            revoked_at: None,
        };
        let allowed = AppPathState {
            enabled: true,
            clients: vec![client.clone()],
            ..Default::default()
        };
        for tool in ["list_collections", "search", "get_document", "get_chunk"] {
            assert!(check_mcp(&allowed, &client.client_id, tool, "library").is_ok());
            for case in 0..7 {
                let mut s = allowed.clone();
                match case {
                    0 => s.enabled = false,
                    1 => s.off_in_memory = true,
                    2 => s.clients[0].revoked_at = Some("today".into()),
                    3 => {
                        s.revoked_in_memory.insert(client.client_id.clone());
                    }
                    4 => s.clients[0].consent.disclosure_version = "old".into(),
                    5 => s.clients[0].grants.clear(),
                    _ => s.clients.clear(),
                }
                assert!(check_mcp(&s, &client.client_id, tool, "library").is_err());
            }
        }
    }
}
