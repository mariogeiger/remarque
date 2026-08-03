use serde::Deserialize;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Deserialize)]
pub(crate) struct TelegramConfig {
    pub token: String,
    pub chat_id: i64,
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
        if !valid_token || config.chat_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Telegram configuration is invalid",
            ));
        }
        Ok(config)
    }
}
