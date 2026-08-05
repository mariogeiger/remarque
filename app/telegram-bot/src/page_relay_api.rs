use crate::config::RelayConfig;
use remarque_page_log::PageSnapshot;
use serde::Deserialize;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;
use ureq::Agent;

const MAXIMUM_ASSET_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct CreatedPageShare {
    pub share_id: String,
    pub guest_url: String,
    pub owner_token: String,
    pub expires_at_unix_seconds: u64,
}

#[derive(Debug)]
pub struct PageRelayError(String);

impl fmt::Display for PageRelayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PageRelayError {}

pub struct PageRelayApi {
    origin: String,
    owner_token: String,
    agent: Agent,
}

impl PageRelayApi {
    pub fn new(config: RelayConfig) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            origin: config.origin,
            owner_token: config.owner_token,
            agent,
        }
    }

    pub fn upload_background(&self, digest: &[u8; 32], path: &Path) -> Result<(), PageRelayError> {
        let bytes = fs::read(path).map_err(page_relay_error)?;
        if bytes.len() > MAXIMUM_ASSET_BYTES {
            return Err(PageRelayError(
                "page background exceeds the relay limit".to_owned(),
            ));
        }
        let url = format!("{}/api/assets/{}", self.origin, encode_hex(digest));
        let response = self
            .agent
            .put(&url)
            .header("Authorization", &format!("Bearer {}", self.owner_token))
            .header("Content-Type", "application/x-remarque-bgra")
            .send(bytes.as_slice())
            .map_err(page_relay_error)?;
        require_success(response.status(), "background upload")
    }

    pub fn create_share(
        &self,
        snapshot: &PageSnapshot,
    ) -> Result<CreatedPageShare, PageRelayError> {
        let url = format!("{}/api/shares", self.origin);
        let mut response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.owner_token))
            .send_json(serde_json::json!({ "snapshot": snapshot }))
            .map_err(page_relay_error)?;
        require_success(response.status(), "share creation")?;
        response
            .body_mut()
            .read_json()
            .map_err(|error| PageRelayError(format!("relay returned invalid share JSON: {error}")))
    }

    pub fn revoke_share(&self, share_id: &str) -> Result<(), PageRelayError> {
        let url = format!("{}/api/shares/{share_id}", self.origin);
        let response = self
            .agent
            .delete(&url)
            .header("Authorization", &format!("Bearer {}", self.owner_token))
            .call()
            .map_err(page_relay_error)?;
        require_success(response.status(), "share revocation")
    }

    pub fn websocket_url(&self, share_id: &str) -> String {
        format!(
            "wss://{}/api/shares/{share_id}/ws",
            self.origin.trim_start_matches("https://")
        )
    }
}

fn require_success(status: ureq::http::StatusCode, operation: &str) -> Result<(), PageRelayError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(PageRelayError(format!(
            "relay rejected {operation} with HTTP {status}"
        )))
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(text, "{byte:02x}").expect("writing to a string cannot fail");
    }
    text
}

fn page_relay_error(error: impl fmt::Display) -> PageRelayError {
    PageRelayError(error.to_string())
}

impl From<io::Error> for PageRelayError {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}
