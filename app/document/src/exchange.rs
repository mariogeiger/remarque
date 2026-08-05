use crate::atomic_file::write_bytes_atomically;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentRequest {
    pub id: u64,
    pub kind: DocumentRequestKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum DocumentRequestKind {
    ImportPdf {
        document_id: String,
        source_path: PathBuf,
        title: String,
    },
    OpenDocument {
        document_id: String,
    },
    ListDocuments,
    Export {
        destination_path: PathBuf,
        scope: ExportScope,
    },
    PreparePageShare {
        destination_directory: PathBuf,
    },
    ConnectPageShare {
        share_id: String,
        websocket_url: String,
        owner_token: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportScope {
    CurrentPage,
    AllPages,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentSummary {
    pub document_id: String,
    pub title: String,
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
    Opened {
        document: DocumentSummary,
    },
    Exported {
        path: PathBuf,
    },
    PageSharePrepared {
        snapshot_path: PathBuf,
        background_path: Option<PathBuf>,
    },
    PageShareConnected,
    Documents {
        documents: Vec<DocumentSummary>,
    },
    Failed {
        message: String,
    },
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
        if path.exists() {
            let existing: DocumentRequest = read_json(&path)?;
            if existing != *request {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "request ID is already assigned to a different operation",
                ));
            }
            return Ok(());
        }
        let response_path = self.response_path(request.id);
        if response_path.exists() {
            let response: DocumentResponse = read_json(&response_path)?;
            if response.request_id != request.id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "response file contains the wrong request ID",
                ));
            }
            return Ok(());
        }
        write_json_atomically(&path, request)
    }

    pub fn oldest_pending(&self) -> io::Result<Option<PendingDocumentRequest>> {
        self.prepare()?;
        let mut paths = Vec::new();
        for entry in fs::read_dir(self.request_directory())? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                paths.push(path);
            }
        }
        paths.sort();
        let Some(path) = paths.into_iter().next() else {
            return Ok(None);
        };
        let request: DocumentRequest = read_json(&path)?;
        if path != self.request_path(request.id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request file name does not match its request ID",
            ));
        }
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
        let response: DocumentResponse = read_json(&path)?;
        if response.request_id != request_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response file contains the wrong request ID",
            ));
        }
        fs::remove_file(path)?;
        Ok(Some(response))
    }

    pub fn library_state_path(&self) -> PathBuf {
        self.root.join("document-library.json")
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
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_bytes_atomically(path, &bytes)
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
            kind: DocumentRequestKind::ListDocuments,
        };
        exchange.submit(&request).unwrap();
        let pending = exchange.oldest_pending().unwrap().unwrap();
        assert_eq!(pending.request, request);
        exchange
            .complete(
                pending,
                DocumentResponseKind::Documents {
                    documents: Vec::new(),
                },
            )
            .unwrap();
        assert!(exchange.oldest_pending().unwrap().is_none());
        assert_eq!(
            exchange.take_response(42).unwrap().unwrap().kind,
            DocumentResponseKind::Documents {
                documents: Vec::new(),
            }
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_request_is_idempotent() {
        let root = temporary_directory("duplicate");
        let exchange = DocumentExchange::new(&root);
        let request = DocumentRequest {
            id: 9,
            kind: DocumentRequestKind::ListDocuments,
        };
        exchange.submit(&request).unwrap();
        exchange.submit(&request).unwrap();
        assert_eq!(exchange.oldest_pending().unwrap().unwrap().request, request);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_request_id_cannot_change_the_operation() {
        let root = temporary_directory("conflicting-duplicate");
        let exchange = DocumentExchange::new(&root);
        exchange
            .submit(&DocumentRequest {
                id: 9,
                kind: DocumentRequestKind::ListDocuments,
            })
            .unwrap();
        let error = exchange
            .submit(&DocumentRequest {
                id: 9,
                kind: DocumentRequestKind::OpenDocument {
                    document_id: "notebook-1".to_owned(),
                },
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        let _ = fs::remove_dir_all(root);
    }
}
