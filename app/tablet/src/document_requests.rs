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
        DocumentRequestKind::ImportPdf {
            document_id,
            source_path,
            title,
        } => notebook
            .import_pdf(document_id.clone(), source_path, title.clone())
            .map(|document| DocumentResponseKind::Opened { document }),
        DocumentRequestKind::OpenDocument { document_id } => notebook
            .open_document(document_id)
            .map(|document| DocumentResponseKind::Opened { document }),
        DocumentRequestKind::ListDocuments => Ok(DocumentResponseKind::Documents {
            documents: notebook.documents(),
        }),
        DocumentRequestKind::Export {
            destination_path,
            scope,
        } => notebook.export(destination_path, scope.clone()).map(|()| {
            DocumentResponseKind::Exported {
                path: destination_path.clone(),
            }
        }),
    };
    let response = result.unwrap_or_else(|error| DocumentResponseKind::Failed {
        message: error.to_string(),
    });
    exchange.complete(pending, response)
}
