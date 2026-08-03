mod config;
mod telegram_api;

use config::TelegramConfig;
use remarque_document::{
    CurrentDocument, DocumentExchange, DocumentRequest, DocumentRequestKind, DocumentResponse,
    DocumentResponseKind, read_json, write_json_atomically,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use telegram_api::{TelegramApi, TelegramDocument, TelegramMessage, TelegramUpdate};

const HELP: &str = "Envoie-moi un PDF : je l’ouvre dans Remarque. /page renvoie la page actuelle, /document l’original, /next et /previous changent de page, /close revient à la page blanche, /open rouvre le dernier PDF.";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ReceivedPdf {
    path: PathBuf,
    display_name: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct BotState {
    next_update_id: Option<i64>,
    last_pdf: Option<ReceivedPdf>,
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
            "Remarque est en ligne. Envoie-moi un PDF pour l’ouvrir sur la tablette.",
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
        let Some(message) = update.message else {
            return Ok(());
        };
        if message.chat.id != self.allowed_chat_id {
            return Ok(());
        }
        if let Some(document) = &message.document {
            return self.receive_pdf(update.update_id, &message, document);
        }
        let command = message
            .text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .split('@')
            .next()
            .unwrap_or("");
        match command {
            "/open" => self.reopen_last_pdf(update.update_id, &message),
            "/page" => self.send_current_page(update.update_id, &message),
            "/document" => self.send_current_document(update.update_id, &message),
            "/next" => self.change_page(update.update_id, &message, 1),
            "/previous" => self.change_page(update.update_id, &message, -1),
            "/close" => self.close_document(update.update_id, &message),
            "/status" => self.send_status(update.update_id, &message),
            _ => self.reply(&message, HELP),
        }
    }

    fn receive_pdf(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
        document: &TelegramDocument,
    ) -> Result<(), Box<dyn Error>> {
        if !metadata_describes_pdf(document) {
            return self.reply(message, "Je n’accepte que les fichiers PDF.");
        }
        let display_name = safe_pdf_name(document.file_name.as_deref().unwrap_or("document.pdf"));
        let destination = self
            .data_root
            .join("incoming")
            .join(format!("{update_id}-{display_name}"));
        if let Err(error) = self.api.download_pdf(document, &destination) {
            return self.reply(message, &format!("PDF refusé : {error}"));
        }
        if !has_pdf_signature(&destination)? {
            fs::remove_file(&destination)?;
            return self.reply(message, "Le fichier reçu n’est pas un PDF valide.");
        }
        let received = ReceivedPdf {
            path: destination.clone(),
            display_name,
        };
        if self.open_pdf(update_id as u64 * 10 + 1, &received, message)? {
            if let Some(previous) = self.state.last_pdf.replace(received) {
                let _ = fs::remove_file(previous.path);
            }
        } else {
            let _ = fs::remove_file(&destination);
        }
        Ok(())
    }

    fn reopen_last_pdf(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
    ) -> Result<(), Box<dyn Error>> {
        let Some(received) = self.state.last_pdf.clone() else {
            return self.reply(message, "Aucun PDF reçu pour le moment.");
        };
        self.open_pdf(update_id as u64 * 10 + 1, &received, message)?;
        Ok(())
    }

    fn open_pdf(
        &mut self,
        request_id: u64,
        received: &ReceivedPdf,
        message: &TelegramMessage,
    ) -> Result<bool, Box<dyn Error>> {
        ensure_tablet_running()?;
        let response = self.request_document(
            DocumentRequest {
                id: request_id,
                kind: DocumentRequestKind::OpenPdf {
                    source_path: received.path.clone(),
                    display_name: received.display_name.clone(),
                },
            },
            Duration::from_secs(30),
        )?;
        match response.kind {
            DocumentResponseKind::Opened { document } => {
                self.reply(
                    message,
                    &format!(
                        "Ouvert sur la tablette : {} ({} page{}).",
                        document.display_name,
                        document.page_count,
                        if document.page_count == 1 { "" } else { "s" }
                    ),
                )?;
                Ok(true)
            }
            DocumentResponseKind::Failed { message: failure } => {
                self.reply(message, &format!("Impossible d’ouvrir le PDF : {failure}"))?;
                Ok(false)
            }
            _ => Err(io::Error::other("unexpected response to open PDF").into()),
        }
    }

    fn send_current_page(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
    ) -> Result<(), Box<dyn Error>> {
        ensure_tablet_running()?;
        let destination = self
            .data_root
            .join("exports")
            .join(format!("page-{update_id}.pdf"));
        let response = self.request_document(
            DocumentRequest {
                id: update_id as u64 * 10 + 2,
                kind: DocumentRequestKind::ExportCurrentPage {
                    destination_path: destination,
                },
            },
            Duration::from_secs(30),
        )?;
        match response.kind {
            DocumentResponseKind::Exported { path } => {
                self.api.send_document(
                    self.allowed_chat_id,
                    &path,
                    "remarque-page.pdf",
                    "Page actuelle, aplatie en PDF.",
                    Some(message.message_id),
                )?;
                fs::remove_file(path)?;
            }
            DocumentResponseKind::Failed { message: failure } => {
                self.reply(message, &format!("Export impossible : {failure}"))?
            }
            _ => return Err(io::Error::other("unexpected response to page export").into()),
        }
        Ok(())
    }

    fn send_current_document(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
    ) -> Result<(), Box<dyn Error>> {
        ensure_tablet_running()?;
        let response = self.current_document(update_id as u64 * 10 + 3)?;
        match response {
            Some(document) => self.api.send_document(
                self.allowed_chat_id,
                &document.source_path,
                &safe_pdf_name(&document.display_name),
                "PDF original, sans modification.",
                Some(message.message_id),
            )?,
            None => self.reply(message, "Aucun PDF n’est ouvert.")?,
        }
        Ok(())
    }

    fn send_status(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
    ) -> Result<(), Box<dyn Error>> {
        if !tablet_is_running()? {
            return self.reply(
                message,
                "Remarque est arrêté ; l’application native est affichée.",
            );
        }
        match self.current_document(update_id as u64 * 10 + 4)? {
            Some(document) => self.reply(
                message,
                &format!(
                    "Remarque est actif. {} — page {}/{}.",
                    document.display_name, document.page_number, document.page_count
                ),
            ),
            None => self.reply(message, "Remarque est actif sur une page blanche."),
        }
    }

    fn change_page(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
        delta: i32,
    ) -> Result<(), Box<dyn Error>> {
        ensure_tablet_running()?;
        let response = self.request_document(
            DocumentRequest {
                id: update_id as u64 * 10 + if delta > 0 { 5 } else { 6 },
                kind: DocumentRequestKind::ChangePage { delta },
            },
            Duration::from_secs(20),
        )?;
        match response.kind {
            DocumentResponseKind::PageChanged { document } => self.reply(
                message,
                &format!(
                    "Page {}/{} affichée.",
                    document.page_number, document.page_count
                ),
            ),
            DocumentResponseKind::Failed { message: failure } => self.reply(
                message,
                &format!("Changement de page impossible : {failure}"),
            ),
            _ => Err(io::Error::other("unexpected page-change response").into()),
        }
    }

    fn current_document(&self, request_id: u64) -> Result<Option<CurrentDocument>, Box<dyn Error>> {
        let response = self.request_document(
            DocumentRequest {
                id: request_id,
                kind: DocumentRequestKind::GetCurrentDocument,
            },
            Duration::from_secs(10),
        )?;
        match response.kind {
            DocumentResponseKind::CurrentDocument { document } => Ok(Some(document)),
            DocumentResponseKind::NoDocument => Ok(None),
            DocumentResponseKind::Failed { message } => Err(io::Error::other(message).into()),
            _ => Err(io::Error::other("unexpected current-document response").into()),
        }
    }

    fn close_document(
        &mut self,
        update_id: i64,
        message: &TelegramMessage,
    ) -> Result<(), Box<dyn Error>> {
        ensure_tablet_running()?;
        let response = self.request_document(
            DocumentRequest {
                id: update_id as u64 * 10 + 7,
                kind: DocumentRequestKind::CloseDocument,
            },
            Duration::from_secs(10),
        )?;
        match response.kind {
            DocumentResponseKind::Closed => {
                self.reply(message, "PDF fermé. La page blanche est affichée.")
            }
            DocumentResponseKind::NoDocument => self.reply(message, "Aucun PDF n’est ouvert."),
            DocumentResponseKind::Failed { message: failure } => {
                self.reply(message, &format!("Fermeture impossible : {failure}"))
            }
            _ => Err(io::Error::other("unexpected close-document response").into()),
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

fn safe_pdf_name(name: &str) -> String {
    let leaf = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document.pdf");
    let mut safe = leaf
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect::<String>();
    if !safe.to_ascii_lowercase().ends_with(".pdf") {
        safe.push_str(".pdf");
    }
    if safe == ".pdf" {
        "document.pdf".to_owned()
    } else {
        safe
    }
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

fn tablet_is_running() -> io::Result<bool> {
    Ok(Command::new("systemctl")
        .args(["is-active", "--quiet", "remarque-tablet.service"])
        .status()?
        .success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_cannot_escape_the_incoming_directory() {
        assert_eq!(safe_pdf_name("../../hello world.pdf"), "hello_world.pdf");
        assert_eq!(safe_pdf_name("notes"), "notes.pdf");
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
