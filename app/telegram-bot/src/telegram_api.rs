use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use ureq::Agent;
use ureq::unversioned::multipart::{Form, Part};

const MAX_PDF_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum TelegramApiError {
    Transport,
    Rejected { code: i64, description: String },
    InvalidResponse,
    Io(io::Error),
}

impl TelegramApiError {
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Transport
                | Self::Rejected {
                    code: 429 | 500..=599,
                    ..
                }
        )
    }
}

impl fmt::Display for TelegramApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport => write!(formatter, "Telegram transport failed"),
            Self::Rejected { code, description } => {
                write!(formatter, "Telegram API {code}: {description}")
            }
            Self::InvalidResponse => write!(formatter, "Telegram returned an invalid response"),
            Self::Io(error) => write!(formatter, "local Telegram file operation failed: {error}"),
        }
    }
}

impl std::error::Error for TelegramApiError {}

impl From<io::Error> for TelegramApiError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TelegramMessage {
    pub message_id: i64,
    pub chat: TelegramChat,
    #[serde(default)]
    pub text: String,
    pub document: Option<TelegramDocument>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TelegramChat {
    pub id: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TelegramDocument {
    pub file_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Deserialize)]
struct TelegramFile {
    file_path: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    error_code: Option<i64>,
    description: Option<String>,
}

pub(crate) struct TelegramApi {
    token: String,
    agent: Agent,
}

impl TelegramApi {
    pub fn new(token: String) -> Self {
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(65)))
            .build();
        Self {
            token,
            agent: config.into(),
        }
    }

    pub fn get_updates(
        &self,
        offset: Option<i64>,
    ) -> Result<Vec<TelegramUpdate>, TelegramApiError> {
        #[derive(Serialize)]
        struct Parameters {
            #[serde(skip_serializing_if = "Option::is_none")]
            offset: Option<i64>,
            timeout: u32,
            allowed_updates: [&'static str; 1],
        }
        self.call_json(
            "getUpdates",
            &Parameters {
                offset,
                timeout: 50,
                allowed_updates: ["message"],
            },
        )
    }

    pub fn set_commands(&self) -> Result<(), TelegramApiError> {
        #[derive(Serialize)]
        struct Command<'a> {
            command: &'a str,
            description: &'a str,
        }
        #[derive(Serialize)]
        struct Parameters<'a> {
            commands: [Command<'a>; 6],
        }
        let _: bool = self.call_json(
            "setMyCommands",
            &Parameters {
                commands: [
                    Command {
                        command: "open",
                        description: "Ouvrir le dernier PDF reçu",
                    },
                    Command {
                        command: "page",
                        description: "Envoyer la page annotée",
                    },
                    Command {
                        command: "document",
                        description: "Envoyer le PDF original",
                    },
                    Command {
                        command: "next",
                        description: "Afficher la page suivante",
                    },
                    Command {
                        command: "previous",
                        description: "Afficher la page précédente",
                    },
                    Command {
                        command: "status",
                        description: "État de Remarque",
                    },
                ],
            },
        )?;
        Ok(())
    }

    pub fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i64>,
    ) -> Result<(), TelegramApiError> {
        #[derive(Serialize)]
        struct Parameters<'a> {
            chat_id: i64,
            text: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            reply_parameters: Option<ReplyParameters>,
        }
        #[derive(Serialize)]
        struct ReplyParameters {
            message_id: i64,
        }
        let _: serde_json::Value = self.call_json(
            "sendMessage",
            &Parameters {
                chat_id,
                text,
                reply_parameters: reply_to.map(|message_id| ReplyParameters { message_id }),
            },
        )?;
        Ok(())
    }

    pub fn download_pdf(
        &self,
        document: &TelegramDocument,
        destination: &Path,
    ) -> Result<(), TelegramApiError> {
        if document
            .file_size
            .is_some_and(|size| size > MAX_PDF_BYTES as u64)
        {
            return Err(TelegramApiError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "PDF exceeds the 50 MiB limit",
            )));
        }
        #[derive(Serialize)]
        struct Parameters<'a> {
            file_id: &'a str,
        }
        let file: TelegramFile = self.call_json(
            "getFile",
            &Parameters {
                file_id: &document.file_id,
            },
        )?;
        let remote_path = file.file_path.ok_or(TelegramApiError::InvalidResponse)?;
        let url = format!(
            "https://api.telegram.org/file/bot{}/{remote_path}",
            self.token
        );
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|_| TelegramApiError::Transport)?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit((MAX_PDF_BYTES + 1) as u64)
            .read_to_vec()
            .map_err(|_| TelegramApiError::Transport)?;
        if bytes.len() > MAX_PDF_BYTES {
            return Err(TelegramApiError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "PDF exceeds the 50 MiB limit",
            )));
        }
        write_atomically(destination, &bytes)?;
        Ok(())
    }

    pub fn send_document(
        &self,
        chat_id: i64,
        path: &Path,
        file_name: &str,
        caption: &str,
        reply_to: Option<i64>,
    ) -> Result<(), TelegramApiError> {
        let chat_id = chat_id.to_string();
        let reply_parameters =
            reply_to.map(|message_id| serde_json::json!({ "message_id": message_id }).to_string());
        let mut form = Form::new()
            .text("chat_id", &chat_id)
            .text("caption", caption)
            .part(
                "document",
                Part::file(path)?
                    .file_name(file_name)
                    .mime_str("application/pdf")
                    .map_err(|_| TelegramApiError::InvalidResponse)?,
            );
        if let Some(reply_parameters) = &reply_parameters {
            form = form.text("reply_parameters", reply_parameters);
        }
        let url = self.method_url("sendDocument");
        let mut response = self
            .agent
            .post(&url)
            .send(form)
            .map_err(|_| TelegramApiError::Transport)?;
        let response: ApiResponse<serde_json::Value> = response
            .body_mut()
            .read_json()
            .map_err(|_| TelegramApiError::InvalidResponse)?;
        response.result().map(|_| ())
    }

    fn call_json<T: DeserializeOwned>(
        &self,
        method: &str,
        parameters: &impl Serialize,
    ) -> Result<T, TelegramApiError> {
        let url = self.method_url(method);
        let mut response = self
            .agent
            .post(&url)
            .send_json(parameters)
            .map_err(|_| TelegramApiError::Transport)?;
        let response: ApiResponse<T> = response
            .body_mut()
            .read_json()
            .map_err(|_| TelegramApiError::InvalidResponse)?;
        response.result()
    }

    fn method_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.token)
    }
}

impl<T> ApiResponse<T> {
    fn result(self) -> Result<T, TelegramApiError> {
        if self.ok {
            self.result.ok_or(TelegramApiError::InvalidResponse)
        } else {
            Err(TelegramApiError::Rejected {
                code: self.error_code.unwrap_or(0),
                description: self
                    .description
                    .unwrap_or_else(|| "request rejected".to_owned()),
            })
        }
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("download path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary: PathBuf = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}
