use crate::notebook::Notebook;
use remarque_document::{DocumentExchange, DocumentRequestKind, DocumentResponseKind};
use std::io;

pub fn apply_oldest_document_request(
    notebook: &mut Notebook,
    exchange: &DocumentExchange,
) -> io::Result<()> {
    let Some(pending) = exchange.oldest_pending()? else {
        return Ok(());
    };
    let result = match &pending.request.kind {
        DocumentRequestKind::OpenPdf {
            source_path,
            display_name,
        } => notebook
            .open_pdf(source_path, display_name.clone())
            .map(|document| DocumentResponseKind::Opened { document }),
        DocumentRequestKind::ExportCurrentPage { destination_path } => notebook
            .export_current_page(destination_path)
            .map(|()| DocumentResponseKind::Exported {
                path: destination_path.clone(),
            }),
        DocumentRequestKind::ChangePage { delta } => notebook
            .change_page(*delta)
            .map(|document| DocumentResponseKind::PageChanged { document }),
        DocumentRequestKind::GetCurrentDocument => Ok(notebook
            .current_document()
            .map_or(DocumentResponseKind::NoDocument, |document| {
                DocumentResponseKind::CurrentDocument { document }
            })),
    };
    let response = result.unwrap_or_else(|error| DocumentResponseKind::Failed {
        message: error.to_string(),
    });
    exchange.complete(pending, response)
}
