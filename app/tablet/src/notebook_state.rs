use crate::page::Page;
use crate::page_coordinates::migrate_fit_page_strokes_to_fit_width;
use crate::pdfium::{read_pdf_page_sizes, render_pdf_page};
use crate::stroke::Stroke;
use remarque_document::{read_json, write_json_atomically};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct OpenDocument {
    pub source_path: PathBuf,
    pub display_name: String,
    pub page_number: u32,
    pub page_count: u32,
}

pub(crate) struct PdfNotebook {
    pub document: OpenDocument,
    pub pages: Vec<Vec<Stroke>>,
}

pub(crate) struct RestoredNotebook {
    pub page: Page,
    pub pdf: Option<PdfNotebook>,
    pub blank_strokes: Vec<Stroke>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredActivePage {
    Blank,
    Pdf,
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredNotebook {
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    active_page: Option<StoredActivePage>,
    #[serde(default)]
    document: Option<OpenDocument>,
    #[serde(default)]
    pages: Vec<Vec<Stroke>>,
    #[serde(default)]
    blank_page: Vec<Stroke>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    strokes: Vec<Stroke>,
}

pub(crate) fn restore_notebook(
    state_path: &Path,
    canvas_width: usize,
    canvas_height: usize,
    content_top: usize,
) -> io::Result<Option<RestoredNotebook>> {
    if !state_path.exists() {
        return Ok(None);
    }
    let StoredNotebook {
        format_version,
        active_page,
        document,
        mut pages,
        blank_page,
        strokes,
    } = read_json(state_path)?;
    if format_version > FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "notebook format version is unsupported",
        ));
    }
    let Some(document) = document else {
        if matches!(active_page, Some(StoredActivePage::Pdf)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "active PDF state has no document",
            ));
        }
        return Ok(Some(RestoredNotebook {
            page: Page::blank(canvas_width, canvas_height, content_top, blank_page),
            pdf: None,
            blank_strokes: Vec::new(),
        }));
    };
    validate_document_page(&document)?;
    let current_index = document.page_number.saturating_sub(1) as usize;
    if pages.len() != document.page_count as usize {
        pages = vec![Vec::new(); document.page_count as usize];
        pages[current_index] = strokes;
    }
    if format_version == 0 {
        let page_sizes = read_pdf_page_sizes(&document.source_path)?;
        migrate_fit_page_strokes_to_fit_width(
            &mut pages,
            &page_sizes,
            canvas_width,
            canvas_height,
            content_top,
        );
    }
    let pdf_is_active = matches!(
        active_page.unwrap_or(StoredActivePage::Pdf),
        StoredActivePage::Pdf
    );
    let (page, blank_strokes) = if pdf_is_active {
        let rendered = render_pdf_page(
            &document.source_path,
            current_index as u32,
            canvas_width,
            canvas_height,
            content_top,
        )?;
        if rendered.page_count != document.page_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "persisted PDF page count changed",
            ));
        }
        (
            Page::from_rendered_pdf(rendered, pages[current_index].clone()),
            blank_page,
        )
    } else {
        (
            Page::blank(canvas_width, canvas_height, content_top, blank_page),
            Vec::new(),
        )
    };
    Ok(Some(RestoredNotebook {
        page,
        pdf: Some(PdfNotebook { document, pages }),
        blank_strokes,
    }))
}

pub(crate) fn save_notebook(
    state_path: &Path,
    page: &Page,
    pdf: Option<&PdfNotebook>,
    blank_strokes: &[Stroke],
) -> io::Result<()> {
    let active_pdf = page.has_pdf_background();
    let (document, mut pages) = pdf.map_or((None, Vec::new()), |pdf| {
        (Some(pdf.document.clone()), pdf.pages.clone())
    });
    if active_pdf {
        let document = document.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "PDF page has no document")
        })?;
        pages[document.page_number.saturating_sub(1) as usize] = page.strokes.clone();
    }
    write_json_atomically(
        state_path,
        &StoredNotebook {
            format_version: FORMAT_VERSION,
            active_page: Some(if active_pdf {
                StoredActivePage::Pdf
            } else {
                StoredActivePage::Blank
            }),
            document,
            pages,
            blank_page: if active_pdf {
                blank_strokes.to_vec()
            } else {
                page.strokes.clone()
            },
            strokes: Vec::new(),
        },
    )
}

fn validate_document_page(document: &OpenDocument) -> io::Result<()> {
    if document.page_count == 0
        || document.page_number == 0
        || document.page_number > document.page_count
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "persisted PDF page number is invalid",
        ));
    }
    Ok(())
}
