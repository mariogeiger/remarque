use crate::notebook::Notebook;
use remarque_document::{DocumentExchange, DocumentRequestKind, DocumentResponseKind};
use std::io;

pub fn apply_all_pending_document_requests(
    notebook: &mut Notebook,
    exchange: &DocumentExchange,
) -> io::Result<()> {
    while apply_oldest_document_request(notebook, exchange)? {}
    Ok(())
}

fn apply_oldest_document_request(
    notebook: &mut Notebook,
    exchange: &DocumentExchange,
) -> io::Result<bool> {
    let Some(pending) = exchange.oldest_pending()? else {
        return Ok(false);
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
        DocumentRequestKind::PreparePageShare {
            destination_directory,
        } => notebook.prepare_page_share(destination_directory).map(
            |(snapshot_path, background_path)| DocumentResponseKind::PageSharePrepared {
                snapshot_path,
                background_path,
            },
        ),
        DocumentRequestKind::ConnectPageShare {
            share_id,
            websocket_url,
            owner_token,
        } => notebook
            .connect_page_share(share_id, websocket_url, owner_token)
            .map(|()| DocumentResponseKind::PageShareConnected),
    };
    let response = result.unwrap_or_else(|error| DocumentResponseKind::Failed {
        message: error.to_string(),
    });
    exchange.complete(pending, response)?;
    Ok(true)
}
