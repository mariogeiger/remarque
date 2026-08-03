use crate::document_library::DocumentPage;
use crate::page::Page;
use remarque_document::{OwnedRasterPdfPage, write_generated_bgra_pages_as_pdf};
use std::io;
use std::path::Path;

pub(crate) fn export_document_pages(
    destination: &Path,
    pages: &[DocumentPage],
    canvas_width: usize,
    canvas_height: usize,
    content_top: usize,
) -> io::Result<()> {
    write_generated_bgra_pages_as_pdf(destination, pages.len(), |index| {
        let page =
            Page::from_document_page(&pages[index], canvas_width, canvas_height, content_top)?;
        let size_points = page.size_points;
        let flattened = page.flatten(canvas_width, canvas_height)?;
        Ok(OwnedRasterPdfPage {
            pixel_width: flattened.width(),
            pixel_height: flattened.height(),
            size_points,
            bgra: flattened.into_pixels(),
        })
    })
}
