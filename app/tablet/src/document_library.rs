use crate::page_coordinates::migrate_fit_page_strokes_to_fit_width;
#[cfg(feature = "takeover")]
use crate::pdfium::read_pdf_page_sizes;
use crate::stroke::Stroke;
use remarque_document::{DocumentSummary, pdf_content_id, write_json_atomically};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 3;
const FIRST_NOTEBOOK_ID: &str = "notebook-1";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PdfPageSource {
    pub source_path: PathBuf,
    pub page_index: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct DocumentPage {
    pub background: Option<PdfPageSource>,
    pub strokes: Vec<Stroke>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LibraryDocument {
    document_id: String,
    title: String,
    current_page: usize,
    pages: Vec<DocumentPage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DocumentLibrary {
    format_version: u32,
    active_document_id: String,
    next_notebook_number: u32,
    documents: Vec<LibraryDocument>,
}

impl DocumentLibrary {
    pub fn with_default_notebook() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            active_document_id: FIRST_NOTEBOOK_ID.to_owned(),
            next_notebook_number: 2,
            documents: vec![LibraryDocument {
                document_id: FIRST_NOTEBOOK_ID.to_owned(),
                title: "Carnet 1".to_owned(),
                current_page: 0,
                pages: vec![DocumentPage::default()],
            }],
        }
    }

    pub fn active_page(&self) -> &DocumentPage {
        let document = self.active_document();
        &document.pages[document.current_page]
    }

    pub fn store_active_strokes(&mut self, strokes: Vec<Stroke>) {
        let document = self.active_document_mut();
        document.pages[document.current_page].strokes = strokes;
    }

    pub fn change_page(&mut self, delta: i32) -> bool {
        let document = self.active_document_mut();
        let current = document.current_page;
        document.current_page =
            (current as i64 + i64::from(delta)).clamp(0, document.pages.len() as i64 - 1) as usize;
        document.current_page != current
    }

    pub fn insert_blank_page(&mut self) -> DocumentSummary {
        let document = self.active_document_mut();
        let insertion = document.current_page + 1;
        document.pages.insert(insertion, DocumentPage::default());
        document.current_page = insertion;
        summary(document)
    }

    pub fn create_notebook(&mut self) -> DocumentSummary {
        let number = self.next_notebook_number;
        self.next_notebook_number += 1;
        let document = LibraryDocument {
            document_id: format!("notebook-{number}"),
            title: format!("Carnet {number}"),
            current_page: 0,
            pages: vec![DocumentPage::default()],
        };
        self.active_document_id = document.document_id.clone();
        self.documents.push(document);
        summary(self.documents.last().unwrap())
    }

    pub fn import_pdf(
        &mut self,
        document_id: String,
        source_path: PathBuf,
        title: String,
        page_count: u32,
    ) -> io::Result<DocumentSummary> {
        validate_document_id(&document_id)?;
        if page_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PDF has no pages",
            ));
        }
        if let Some(document) = self
            .documents
            .iter_mut()
            .find(|document| document.document_id == document_id)
        {
            for page in &mut document.pages {
                if let Some(background) = &mut page.background {
                    background.source_path.clone_from(&source_path);
                }
            }
            document.title = title;
            self.active_document_id = document_id;
            return Ok(summary(document));
        }
        let pages = (0..page_count)
            .map(|page_index| DocumentPage {
                background: Some(PdfPageSource {
                    source_path: source_path.clone(),
                    page_index,
                }),
                strokes: Vec::new(),
            })
            .collect();
        let document = LibraryDocument {
            document_id: document_id.clone(),
            title,
            current_page: 0,
            pages,
        };
        self.active_document_id = document_id;
        self.documents.push(document);
        Ok(summary(self.documents.last().unwrap()))
    }

    pub fn open_document(&mut self, document_id: &str) -> io::Result<DocumentSummary> {
        let document = self
            .documents
            .iter()
            .find(|document| document.document_id == document_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "document was not found"))?;
        let result = summary(document);
        self.active_document_id = document_id.to_owned();
        Ok(result)
    }

    pub fn active_summary(&self) -> DocumentSummary {
        summary(self.active_document())
    }

    pub fn summaries(&self) -> Vec<DocumentSummary> {
        self.documents.iter().map(summary).collect()
    }

    pub fn active_document_id(&self) -> &str {
        &self.active_document_id
    }

    pub fn active_pages(&self) -> &[DocumentPage] {
        &self.active_document().pages
    }

    fn active_document(&self) -> &LibraryDocument {
        self.documents
            .iter()
            .find(|document| document.document_id == self.active_document_id)
            .expect("validated library has an active document")
    }

    fn active_document_mut(&mut self) -> &mut LibraryDocument {
        self.documents
            .iter_mut()
            .find(|document| document.document_id == self.active_document_id)
            .expect("validated library has an active document")
    }

    fn validate(&self) -> io::Result<()> {
        if self.format_version != FORMAT_VERSION || self.documents.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "document library format is unsupported",
            ));
        }
        let mut ids = HashSet::new();
        for document in &self.documents {
            validate_document_id(&document.document_id)?;
            if !ids.insert(&document.document_id)
                || document.pages.is_empty()
                || document.current_page >= document.pages.len()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "document library contains an invalid document",
                ));
            }
        }
        if !ids.contains(&self.active_document_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "document library has no active document",
            ));
        }
        Ok(())
    }
}

pub(crate) fn restore_document_library(
    state_path: &Path,
    canvas_width: usize,
    canvas_height: usize,
    content_top: usize,
) -> io::Result<DocumentLibrary> {
    let legacy_path = state_path.with_file_name("current-notebook.json");
    let Some(source_path) = most_recent_state_path(state_path, &legacy_path)? else {
        return Ok(DocumentLibrary::with_default_notebook());
    };
    let bytes = fs::read(&source_path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let format_version = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;
    let library = if format_version == FORMAT_VERSION {
        serde_json::from_value(value).map_err(io::Error::other)?
    } else if format_version < FORMAT_VERSION {
        migrate_legacy_notebook(value, canvas_width, canvas_height, content_top)?
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "document library format is unsupported",
        ));
    };
    library.validate()?;
    Ok(library)
}

fn most_recent_state_path(library_path: &Path, legacy_path: &Path) -> io::Result<Option<PathBuf>> {
    let library_modified = modified_if_present(library_path)?;
    let legacy_modified = modified_if_present(legacy_path)?;
    Ok(match (library_modified, legacy_modified) {
        (Some(library_modified), Some(legacy_modified)) => {
            Some(if library_modified >= legacy_modified {
                library_path.to_owned()
            } else {
                legacy_path.to_owned()
            })
        }
        (Some(_), None) => Some(library_path.to_owned()),
        (None, Some(_)) => Some(legacy_path.to_owned()),
        (None, None) => None,
    })
}

fn modified_if_present(path: &Path) -> io::Result<Option<std::time::SystemTime>> {
    match fs::metadata(path) {
        Ok(metadata) => metadata.modified().map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn save_document_library(
    state_path: &Path,
    library: &DocumentLibrary,
) -> io::Result<()> {
    library.validate()?;
    write_json_atomically(state_path, library)
}

fn summary(document: &LibraryDocument) -> DocumentSummary {
    DocumentSummary {
        document_id: document.document_id.clone(),
        title: document.title.clone(),
        page_number: document.current_page as u32 + 1,
        page_count: document.pages.len() as u32,
    }
}

fn validate_document_id(document_id: &str) -> io::Result<()> {
    if document_id.is_empty()
        || document_id.len() > 48
        || !document_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "document ID is invalid",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct LegacyOpenDocument {
    source_path: PathBuf,
    display_name: String,
    page_number: u32,
    page_count: u32,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyActivePage {
    Blank,
    Pdf,
}

#[derive(Deserialize)]
struct LegacyNotebook {
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    active_page: Option<LegacyActivePage>,
    #[serde(default)]
    document: Option<LegacyOpenDocument>,
    #[serde(default)]
    pages: Vec<Vec<Stroke>>,
    #[serde(default)]
    blank_page: Vec<Stroke>,
    #[serde(default)]
    strokes: Vec<Stroke>,
}

fn migrate_legacy_notebook(
    value: serde_json::Value,
    canvas_width: usize,
    canvas_height: usize,
    content_top: usize,
) -> io::Result<DocumentLibrary> {
    let legacy: LegacyNotebook = serde_json::from_value(value).map_err(io::Error::other)?;
    let pdf_is_active = matches!(
        legacy.active_page.unwrap_or(LegacyActivePage::Pdf),
        LegacyActivePage::Pdf
    );
    let mut library = DocumentLibrary::with_default_notebook();
    library.documents[0].pages[0].strokes = legacy.blank_page;
    let Some(document) = legacy.document else {
        return Ok(library);
    };
    if document.page_count == 0
        || document.page_number == 0
        || document.page_number > document.page_count
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "persisted PDF page number is invalid",
        ));
    }
    let current_page = document.page_number as usize - 1;
    let mut strokes = legacy.pages;
    if strokes.len() != document.page_count as usize {
        strokes = vec![Vec::new(); document.page_count as usize];
        strokes[current_page] = legacy.strokes;
    }
    if legacy.format_version == 0 {
        let page_sizes = read_legacy_page_sizes(&document.source_path)?;
        migrate_fit_page_strokes_to_fit_width(
            &mut strokes,
            &page_sizes,
            canvas_width,
            canvas_height,
            content_top,
        );
    }
    let document_id =
        pdf_content_id(&document.source_path).unwrap_or_else(|_| "legacy-pdf".to_owned());
    let pages = strokes
        .into_iter()
        .enumerate()
        .map(|(page_index, strokes)| DocumentPage {
            background: Some(PdfPageSource {
                source_path: document.source_path.clone(),
                page_index: page_index as u32,
            }),
            strokes,
        })
        .collect();
    library.documents.push(LibraryDocument {
        document_id: document_id.clone(),
        title: document.display_name,
        current_page,
        pages,
    });
    if pdf_is_active {
        library.active_document_id = document_id;
    }
    Ok(library)
}

#[cfg(feature = "takeover")]
fn read_legacy_page_sizes(path: &Path) -> io::Result<Vec<[f64; 2]>> {
    read_pdf_page_sizes(path)
}

#[cfg(not(feature = "takeover"))]
fn read_legacy_page_sizes(_path: &Path) -> io::Result<Vec<[f64; 2]>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "legacy format 0 migration requires PDFium",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remarque-library-{name}-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn every_document_uses_the_same_page_operations() {
        let mut library = DocumentLibrary::with_default_notebook();
        assert_eq!(library.insert_blank_page().page_count, 2);
        assert!(library.change_page(-1));
        assert_eq!(library.active_summary().page_number, 1);
        let imported = library
            .import_pdf(
                "pdf-0123456789abcdef0123456789abcdef".to_owned(),
                PathBuf::from("source.pdf"),
                "Source".to_owned(),
                2,
            )
            .unwrap();
        assert_eq!(imported.page_count, 2);
        assert_eq!(library.insert_blank_page().page_count, 3);
        assert!(library.active_page().background.is_none());
        library.store_active_strokes(Vec::new());
        assert_eq!(library.create_notebook().title, "Carnet 2");
        assert_eq!(
            library.open_document(FIRST_NOTEBOOK_ID).unwrap().page_count,
            2
        );
        assert_eq!(library.summaries().len(), 3);
        assert_eq!(library.active_document_id(), FIRST_NOTEBOOK_ID);
        assert_eq!(library.active_pages().len(), 2);
        library.validate().unwrap();
    }

    #[test]
    fn version_two_state_becomes_a_two_document_library() {
        let directory = state_path("migration").with_extension("");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let legacy_path = directory.join("current-notebook.json");
        let path = directory.join("document-library.json");
        let legacy = serde_json::json!({
            "format_version": 2,
            "active_page": "blank",
            "document": {
                "source_path": "/missing/source.pdf",
                "display_name": "Imported.pdf",
                "page_number": 2,
                "page_count": 2
            },
            "pages": [[], []],
            "blank_page": []
        });
        fs::write(&legacy_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let library = restore_document_library(&path, 1620, 2160, 112).unwrap();
        assert_eq!(library.summaries().len(), 2);
        assert_eq!(library.active_document_id(), FIRST_NOTEBOOK_ID);
        assert_eq!(library.summaries()[1].page_number, 2);
        save_document_library(&path, &library).unwrap();
        let restored = restore_document_library(&path, 1620, 2160, 112).unwrap();
        assert_eq!(restored.format_version, FORMAT_VERSION);
        assert_eq!(restored.summaries().len(), 2);
        let _ = fs::remove_dir_all(directory);
    }
}
