mod config;
mod telegram_api;

use config::TelegramConfig;
use remarque_document::{
    DocumentExchange, DocumentRequest, DocumentRequestKind, DocumentResponse, DocumentResponseKind,
    DocumentSummary, ExportScope, pdf_content_id, read_json, write_json_atomically,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use telegram_api::{
    TelegramApi, TelegramButton, TelegramCallbackQuery, TelegramDocument, TelegramMessage,
    TelegramUpdate,
};

const HELP: &str = "Envoie-moi un PDF pour l’importer. /library choisit le document affiché et /export renvoie la page actuelle ou toutes les pages annotées.";
const DOCUMENTS_PER_MESSAGE: usize = 8;

#[derive(Debug, Default, Deserialize, Serialize)]
struct BotState {
    next_update_id: Option<i64>,
}

struct BotRuntime {
    api: TelegramApi,
    allowed_chat_id: i64,
    data_root: PathBuf,
    state_path: PathBuf,
    exchange: DocumentExchange,
    state: BotState,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("remarque_telegram_bot_failed={error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let config_path = std::env::var_os("REMARQUE_TELEGRAM_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/root/remarque/config/telegram.json"));
    let data_root = std::env::var_os("REMARQUE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/root/remarque/data"));
    let config = TelegramConfig::read_private(&config_path)?;
    let state_path = data_root.join("telegram-state.json");
    let first_start = !state_path.exists();
    let state = if first_start {
        BotState::default()
    } else {
        read_json(&state_path)?
    };
    fs::create_dir_all(data_root.join("incoming"))?;
    fs::create_dir_all(data_root.join("exports"))?;
    let mut runtime = BotRuntime {
        api: TelegramApi::new(config.token),
        allowed_chat_id: config.chat_id,
        exchange: DocumentExchange::new(data_root.join("exchange")),
        data_root,
        state_path,
        state,
    };
    runtime.exchange.prepare()?;
    runtime.api.set_commands()?;
    if first_start {
        runtime.api.send_message(
            runtime.allowed_chat_id,
            "Remarque est en ligne. Envoie-moi un PDF pour l’importer.",
            None,
        )?;
        runtime.save_state()?;
    }
    runtime.poll_forever()
}

impl BotRuntime {
    fn poll_forever(&mut self) -> Result<(), Box<dyn Error>> {
        let mut retry_delay = Duration::from_secs(1);
        loop {
            let updates = match self.api.get_updates(self.state.next_update_id) {
                Ok(updates) => {
                    retry_delay = Duration::from_secs(1);
                    updates
                }
                Err(error) if error.retryable() => {
                    eprintln!("telegram_poll_retry={error}");
                    thread::sleep(retry_delay);
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                    continue;
                }
                Err(error) => return Err(Box::new(error)),
            };
            for update in updates {
                let update_id = update.update_id;
                if let Err(error) = self.apply_update(update) {
                    eprintln!("telegram_update_retry={error}");
                    thread::sleep(Duration::from_secs(2));
                    break;
                }
                self.state.next_update_id = Some(update_id + 1);
                self.save_state()?;
            }
        }
    }

    fn apply_update(&mut self, update: TelegramUpdate) -> Result<(), Box<dyn Error>> {
        if let Some(callback) = update.callback_query {
            return self.apply_callback(update.update_id, callback);
        }
        let Some(message) = update.message else {
            return Ok(());
        };
        if message.chat.id != self.allowed_chat_id {
            return Ok(());
        }
        if let Some(document) = &message.document {
            return self.import_pdf(update.update_id, &message, document);
        }
        match telegram_command(&message.text) {
            "/library" => self.send_library(update.update_id, &message, 0),
            "/export" => self.send_export_choices(&message),
            _ => self.reply(&message, HELP),
        }
    }

    fn apply_callback(
        &mut self,
        update_id: i64,
        callback: TelegramCallbackQuery,
    ) -> Result<(), Box<dyn Error>> {
        let Some(message) = callback.message else {
            return Ok(());
        };
        if message.chat.id != self.allowed_chat_id {
            return Ok(());
        }
        self.api.answer_callback_query(&callback.id)?;
        let data = callback.data.as_deref().unwrap_or("");
        if let Some(document_id) = data.strip_prefix("open:") {
            return self.open_document(update_id, &message, document_id);
        }
        if let Some(page) = data.strip_prefix("library:") {
            let page = page.parse::<usize>().unwrap_or(0);
            return self.send_library(update_id, &message, page);
        }
        match data {
            "export:page" => self.export(update_id, &message, ExportScope::CurrentPage),
            "export:all" => self.export(update_id, &message, ExportScope::AllPages),
            _ => self.reply(&message, HELP),
        }
    }

    fn import_pdf(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
        document: &TelegramDocument,
    ) -> Result<(), Box<dyn Error>> {
        if !metadata_describes_pdf(document) {
            return self.reply(message, "Je n’accepte que les fichiers PDF.");
        }
        let temporary = self
            .data_root
            .join("incoming")
            .join(format!(".download-{update_id}.pdf"));
        if let Err(error) = self.api.download_pdf(document, &temporary) {
            return self.reply(message, &format!("PDF refusé : {error}"));
        }
        if !has_pdf_signature(&temporary)? {
            fs::remove_file(&temporary)?;
            return self.reply(message, "Le fichier reçu n’est pas un PDF valide.");
        }
        let document_id = pdf_content_id(&temporary)?;
        let source_path = self
            .data_root
            .join("incoming")
            .join(format!("{document_id}.pdf"));
        if source_path.exists() {
            fs::remove_file(&temporary)?;
        } else {
            fs::rename(&temporary, &source_path)?;
        }
        ensure_tablet_running()?;
        let response = self.request_document(
            DocumentRequest {
                id: request_id(update_id, 5),
                kind: DocumentRequestKind::ImportPdf {
                    document_id,
                    source_path,
                    title: display_title(document.file_name.as_deref()),
                },
            },
            Duration::from_secs(30),
        )?;
        match response.kind {
            DocumentResponseKind::Opened { document } => self.reply(
                message,
                &format!(
                    "{} ouvert — page {}/{}.",
                    document.title, document.page_number, document.page_count
                ),
            ),
            DocumentResponseKind::Failed { message: failure } => {
                self.reply(message, &format!("Import impossible : {failure}"))
            }
            _ => Err(io::Error::other("unexpected response to PDF import").into()),
        }
    }

    fn send_library(
        &self,
        update_id: i64,
        message: &TelegramMessage,
        requested_page: usize,
    ) -> Result<(), Box<dyn Error>> {
        ensure_tablet_running()?;
        let response = self.request_document(
            DocumentRequest {
                id: request_id(update_id, 1),
                kind: DocumentRequestKind::ListDocuments,
            },
            Duration::from_secs(10),
        )?;
        let documents = match response.kind {
            DocumentResponseKind::Documents { documents } => documents,
            DocumentResponseKind::Failed { message: failure } => {
                return self.reply(message, &format!("Bibliothèque indisponible : {failure}"));
            }
            _ => return Err(io::Error::other("unexpected response to document list").into()),
        };
        let page_count = documents.len().div_ceil(DOCUMENTS_PER_MESSAGE).max(1);
        let page = requested_page.min(page_count - 1);
        let first = page * DOCUMENTS_PER_MESSAGE;
        let mut buttons = documents[first..documents.len().min(first + DOCUMENTS_PER_MESSAGE)]
            .iter()
            .map(|document| {
                vec![TelegramButton {
                    text: document_button_text(document),
                    callback_data: format!("open:{}", document.document_id),
                }]
            })
            .collect::<Vec<_>>();
        let mut navigation = Vec::new();
        if page > 0 {
            navigation.push(TelegramButton {
                text: "‹".to_owned(),
                callback_data: format!("library:{}", page - 1),
            });
        }
        if page + 1 < page_count {
            navigation.push(TelegramButton {
                text: "›".to_owned(),
                callback_data: format!("library:{}", page + 1),
            });
        }
        if !navigation.is_empty() {
            buttons.push(navigation);
        }
        self.api.send_message_with_buttons(
            self.allowed_chat_id,
            &format!("Bibliothèque — {}/{}", page + 1, page_count),
            Some(message.message_id),
            &buttons,
        )?;
        Ok(())
    }

    fn send_export_choices(&self, message: &TelegramMessage) -> Result<(), Box<dyn Error>> {
        self.api.send_message_with_buttons(
            self.allowed_chat_id,
            "Que veux-tu exporter ?",
            Some(message.message_id),
            &[
                vec![TelegramButton {
                    text: "Page actuelle".to_owned(),
                    callback_data: "export:page".to_owned(),
                }],
                vec![TelegramButton {
                    text: "Toutes les pages".to_owned(),
                    callback_data: "export:all".to_owned(),
                }],
            ],
        )?;
        Ok(())
    }

    fn open_document(
        &self,
        update_id: i64,
        message: &TelegramMessage,
        document_id: &str,
    ) -> Result<(), Box<dyn Error>> {
        ensure_tablet_running()?;
        let response = self.request_document(
            DocumentRequest {
                id: request_id(update_id, 2),
                kind: DocumentRequestKind::OpenDocument {
                    document_id: document_id.to_owned(),
                },
            },
            Duration::from_secs(20),
        )?;
        match response.kind {
            DocumentResponseKind::Opened { document } => self.reply(
                message,
                &format!(
                    "{} ouvert — page {}/{}.",
                    document.title, document.page_number, document.page_count
                ),
            ),
            DocumentResponseKind::Failed { message: failure } => {
                self.reply(message, &format!("Ouverture impossible : {failure}"))
            }
            _ => Err(io::Error::other("unexpected response to document open").into()),
        }
    }

    fn export(
        &self,
        update_id: i64,
        message: &TelegramMessage,
        scope: ExportScope,
    ) -> Result<(), Box<dyn Error>> {
        ensure_tablet_running()?;
        let suffix = match scope {
            ExportScope::CurrentPage => 3,
            ExportScope::AllPages => 4,
        };
        let destination = self
            .data_root
            .join("exports")
            .join(format!("export-{update_id}.pdf"));
        let response = self.request_document(
            DocumentRequest {
                id: request_id(update_id, suffix),
                kind: DocumentRequestKind::Export {
                    destination_path: destination,
                    scope: scope.clone(),
                },
            },
            match scope {
                ExportScope::CurrentPage => Duration::from_secs(30),
                ExportScope::AllPages => Duration::from_secs(180),
            },
        )?;
        match response.kind {
            DocumentResponseKind::Exported { path } => {
                let (file_name, caption) = match scope {
                    ExportScope::CurrentPage => {
                        ("remarque-page.pdf", "Page actuelle, aplatie en PDF.")
                    }
                    ExportScope::AllPages => {
                        ("remarque-document.pdf", "Toutes les pages annotées.")
                    }
                };
                self.api.send_document(
                    self.allowed_chat_id,
                    &path,
                    file_name,
                    caption,
                    Some(message.message_id),
                )?;
                let _ = fs::remove_file(path);
                Ok(())
            }
            DocumentResponseKind::Failed { message: failure } => {
                self.reply(message, &format!("Export impossible : {failure}"))
            }
            _ => Err(io::Error::other("unexpected response to export").into()),
        }
    }

    fn request_document(
        &self,
        request: DocumentRequest,
        timeout: Duration,
    ) -> Result<DocumentResponse, Box<dyn Error>> {
        self.exchange.submit(&request)?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(response) = self.exchange.take_response(request.id)? {
                return Ok(response);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "tablet did not answer document request",
                )
                .into());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn reply(&self, message: &TelegramMessage, text: &str) -> Result<(), Box<dyn Error>> {
        self.api
            .send_message(self.allowed_chat_id, text, Some(message.message_id))?;
        Ok(())
    }

    fn save_state(&self) -> io::Result<()> {
        write_json_atomically(&self.state_path, &self.state)
    }
}

fn request_id(update_id: i64, suffix: u64) -> u64 {
    update_id as u64 * 10 + suffix
}

fn telegram_command(text: &str) -> &str {
    text.split_whitespace()
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
}

fn metadata_describes_pdf(document: &TelegramDocument) -> bool {
    document.mime_type.as_deref() == Some("application/pdf")
        || document
            .file_name
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".pdf"))
}

fn has_pdf_signature(path: &Path) -> io::Result<bool> {
    let mut signature = [0; 5];
    let read = fs::File::open(path)?.read(&mut signature)?;
    Ok(read == signature.len() && &signature == b"%PDF-")
}

fn display_title(file_name: Option<&str>) -> String {
    let leaf = file_name
        .and_then(|name| Path::new(name).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("Document PDF");
    let title = leaf
        .chars()
        .filter(|character| !character.is_control())
        .take(120)
        .collect::<String>();
    if title.is_empty() {
        "Document PDF".to_owned()
    } else {
        title
    }
}

fn document_button_text(document: &DocumentSummary) -> String {
    format!(
        "{} · {}/{}",
        document.title, document.page_number, document.page_count
    )
}

fn ensure_tablet_running() -> io::Result<()> {
    let status = Command::new("systemctl")
        .args(["start", "remarque-tablet.service"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            "could not start Remarque tablet application",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_title_cannot_escape_or_hide_control_characters() {
        assert_eq!(
            display_title(Some("../../hello world.pdf")),
            "hello world.pdf"
        );
        assert_eq!(display_title(Some("bad\nname.pdf")), "badname.pdf");
    }

    #[test]
    fn pdf_metadata_accepts_mime_or_extension() {
        let mut document = TelegramDocument {
            file_id: "x".to_owned(),
            file_name: Some("x.PDF".to_owned()),
            mime_type: None,
            file_size: None,
        };
        assert!(metadata_describes_pdf(&document));
        document.file_name = Some("x.bin".to_owned());
        document.mime_type = Some("application/pdf".to_owned());
        assert!(metadata_describes_pdf(&document));
    }

    #[test]
    fn telegram_rejections_are_classified_for_polling() {
        assert!(
            telegram_api::TelegramApiError::Rejected {
                code: 429,
                description: String::new(),
            }
            .retryable()
        );
        assert!(
            !telegram_api::TelegramApiError::Rejected {
                code: 401,
                description: String::new(),
            }
            .retryable()
        );
    }
}
