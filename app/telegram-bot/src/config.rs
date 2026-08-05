use serde::Deserialize;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Deserialize)]
pub(crate) struct TelegramConfig {
    pub token: String,
    pub chat_id: i64,
    pub relay: Option<RelayConfig>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct RelayConfig {
    pub origin: String,
    pub owner_token: String,
}

impl TelegramConfig {
    pub fn read_private(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Telegram configuration must have mode 0600",
            ));
        }
        let config: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        let valid_token = config.token.split_once(':').is_some_and(|(id, secret)| {
            !id.is_empty()
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && secret.len() >= 20
                && secret
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        });
        let valid_relay = config.relay.as_ref().is_none_or(|relay| {
            relay.origin.starts_with("https://")
                && !relay.origin.ends_with('/')
                && !relay.origin.bytes().any(|byte| byte.is_ascii_whitespace())
                && relay.owner_token.len() >= 32
                && !relay
                    .owner_token
                    .bytes()
                    .any(|byte| byte.is_ascii_whitespace())
        });
        if !valid_token || config.chat_id == 0 || !valid_relay {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Telegram configuration is invalid",
            ));
        }
        Ok(config)
    }
}
