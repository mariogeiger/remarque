use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:7433";
const DEFAULT_PUBLIC_ORIGIN: &str = "https://remarque.geiger.ink";
const DEFAULT_DATA_DIRECTORY: &str = "/var/lib/remarque-page-relay";
const DEFAULT_VIEWER_DIRECTORY: &str = "/usr/lib/remarque-page-relay/wasm";

#[derive(Clone, Debug)]
pub struct RelaySettings {
    pub listen_address: SocketAddr,
    pub public_origin: String,
    pub data_directory: PathBuf,
    pub viewer_directory: PathBuf,
    pub owner_token: String,
}

impl RelaySettings {
    pub fn from_environment() -> io::Result<Self> {
        let listen_address = env::var("REMARQUE_RELAY_LISTEN")
            .unwrap_or_else(|_| DEFAULT_LISTEN_ADDRESS.to_owned())
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid relay address"))?;
        let public_origin = env::var("REMARQUE_RELAY_PUBLIC_ORIGIN")
            .unwrap_or_else(|_| DEFAULT_PUBLIC_ORIGIN.to_owned());
        if !public_origin.starts_with("https://") || public_origin.ends_with('/') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "relay public origin must be an HTTPS origin without a trailing slash",
            ));
        }
        let owner_token = match (
            env::var("REMARQUE_RELAY_OWNER_TOKEN").ok(),
            env::var_os("REMARQUE_RELAY_OWNER_TOKEN_FILE"),
        ) {
            (Some(token), None) => token,
            (None, Some(path)) => std::fs::read_to_string(path)?.trim_end().to_owned(),
            (None, None) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "a relay owner token or token file is required",
                ));
            }
            (Some(_), Some(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "configure either a relay owner token or token file, not both",
                ));
            }
        };
        if owner_token.len() < 32 || owner_token.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "relay owner token must contain at least 32 non-whitespace bytes",
            ));
        }
        Ok(Self {
            listen_address,
            public_origin,
            data_directory: env::var_os("REMARQUE_RELAY_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIRECTORY)),
            viewer_directory: env::var_os("REMARQUE_RELAY_VIEWER_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_VIEWER_DIRECTORY)),
            owner_token,
        })
    }
}
