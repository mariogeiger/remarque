use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentRequest {
    pub id: u64,
    pub kind: DocumentRequestKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum DocumentRequestKind {
    OpenPdf {
        source_path: PathBuf,
        display_name: String,
    },
    ExportCurrentPage {
        destination_path: PathBuf,
    },
    ChangePage {
        delta: i32,
    },
    CloseDocument,
    GetCurrentDocument,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CurrentDocument {
    pub source_path: PathBuf,
    pub display_name: String,
    pub page_number: u32,
    pub page_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentResponse {
    pub request_id: u64,
    pub kind: DocumentResponseKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum DocumentResponseKind {
    Opened { document: CurrentDocument },
    Exported { path: PathBuf },
    PageChanged { document: CurrentDocument },
    Closed,
    CurrentDocument { document: CurrentDocument },
    NoDocument,
    Failed { message: String },
}

pub struct PendingDocumentRequest {
    path: PathBuf,
    pub request: DocumentRequest,
}

#[derive(Clone, Debug)]
pub struct DocumentExchange {
    root: PathBuf,
}

impl DocumentExchange {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn prepare(&self) -> io::Result<()> {
        fs::create_dir_all(self.request_directory())?;
        fs::create_dir_all(self.response_directory())?;
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(self.request_directory(), fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(self.response_directory(), fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    pub fn submit(&self, request: &DocumentRequest) -> io::Result<()> {
        self.prepare()?;
        let path = self.request_path(request.id);
        if path.exists() || self.response_path(request.id).exists() {
            return Ok(());
        }
        write_json_atomically(&path, request)
    }

    pub fn oldest_pending(&self) -> io::Result<Option<PendingDocumentRequest>> {
        self.prepare()?;
        let mut paths = fs::read_dir(self.request_directory())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        let Some(path) = paths.into_iter().next() else {
            return Ok(None);
        };
        let request = read_json(&path)?;
        Ok(Some(PendingDocumentRequest { path, request }))
    }

    pub fn complete(
        &self,
        pending: PendingDocumentRequest,
        kind: DocumentResponseKind,
    ) -> io::Result<()> {
        let response = DocumentResponse {
            request_id: pending.request.id,
            kind,
        };
        write_json_atomically(&self.response_path(response.request_id), &response)?;
        fs::remove_file(pending.path)
    }

    pub fn take_response(&self, request_id: u64) -> io::Result<Option<DocumentResponse>> {
        let path = self.response_path(request_id);
        if !path.exists() {
            return Ok(None);
        }
        let response = read_json(&path)?;
        fs::remove_file(path)?;
        Ok(Some(response))
    }

    pub fn state_path(&self) -> PathBuf {
        self.root.join("current-notebook.json")
    }

    fn request_directory(&self) -> PathBuf {
        self.root.join("requests")
    }

    fn response_directory(&self) -> PathBuf {
        self.root.join("responses")
    }

    fn request_path(&self, request_id: u64) -> PathBuf {
        self.request_directory()
            .join(format!("{request_id:020}.json"))
    }

    fn response_path(&self, request_id: u64) -> PathBuf {
        self.response_directory()
            .join(format!("{request_id:020}.json"))
    }
}

pub fn write_json_atomically(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("atomic file has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "remarque-document-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn response_becomes_visible_before_request_is_removed() {
        let root = temporary_directory("exchange");
        let exchange = DocumentExchange::new(&root);
        let request = DocumentRequest {
            id: 42,
            kind: DocumentRequestKind::GetCurrentDocument,
        };
        exchange.submit(&request).unwrap();
        let pending = exchange.oldest_pending().unwrap().unwrap();
        assert_eq!(pending.request, request);
        exchange
            .complete(pending, DocumentResponseKind::NoDocument)
            .unwrap();
        assert!(exchange.oldest_pending().unwrap().is_none());
        assert_eq!(
            exchange.take_response(42).unwrap().unwrap().kind,
            DocumentResponseKind::NoDocument
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_request_is_idempotent() {
        let root = temporary_directory("duplicate");
        let exchange = DocumentExchange::new(&root);
        let request = DocumentRequest {
            id: 9,
            kind: DocumentRequestKind::GetCurrentDocument,
        };
        exchange.submit(&request).unwrap();
        exchange.submit(&request).unwrap();
        assert_eq!(exchange.oldest_pending().unwrap().unwrap().request, request);
        let _ = fs::remove_dir_all(root);
    }
}
