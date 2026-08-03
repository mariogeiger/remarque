use crate::bgra_image::BgraImage;
use crate::draw_text::draw_text;
use crate::view_transform::Point;
use remarque_document::DocumentSummary;

const HEADER_HEIGHT: usize = 128;
const ROW_X: usize = 64;
const ROW_WIDTH: usize = 1492;
const ROW_START_Y: usize = 160;
const ROW_HEIGHT: usize = 126;
const ROW_STEP: usize = 140;
pub(crate) const DOCUMENTS_PER_SCREEN: usize = 13;

pub(crate) enum DocumentLibraryAction {
    Close,
    CreateNotebook,
    OpenDocument(String),
    PreviousScreen,
    NextScreen,
    None,
}

pub(crate) fn draw_document_library(
    image: &mut BgraImage,
    documents: &[DocumentSummary],
    active_document_id: &str,
    screen_index: usize,
) {
    image.fill_rectangle(0, 0, image.width(), image.height(), [0xe8, 0xe7, 0xe2]);
    image.fill_rectangle(0, 0, image.width(), HEADER_HEIGHT, [0xfa, 0xf9, 0xf6]);
    draw_back_button(image);
    draw_text(image, 144, 82, "Bibliothèque", 38, 700, [0x25, 0x25, 0x24]);
    draw_new_notebook_button(image);

    let first = screen_index * DOCUMENTS_PER_SCREEN;
    for (slot, document) in documents
        .iter()
        .skip(first)
        .take(DOCUMENTS_PER_SCREEN)
        .enumerate()
    {
        draw_document_row(
            image,
            ROW_START_Y + slot * ROW_STEP,
            document,
            document.document_id == active_document_id,
        );
    }
    draw_screen_navigation(image, documents.len(), screen_index);
}

pub(crate) fn document_library_action_at(
    position: Point,
    documents: &[DocumentSummary],
    screen_index: usize,
) -> DocumentLibraryAction {
    let x = position.x.max(0.0) as usize;
    let y = position.y.max(0.0) as usize;
    if x < 120 && y < HEADER_HEIGHT {
        return DocumentLibraryAction::Close;
    }
    if x >= 1180 && y < HEADER_HEIGHT {
        return DocumentLibraryAction::CreateNotebook;
    }
    if x >= ROW_X && x < ROW_X + ROW_WIDTH && y >= ROW_START_Y {
        let slot = (y - ROW_START_Y) / ROW_STEP;
        let within_row = (y - ROW_START_Y) % ROW_STEP < ROW_HEIGHT;
        let index = screen_index * DOCUMENTS_PER_SCREEN + slot;
        if within_row && slot < DOCUMENTS_PER_SCREEN && index < documents.len() {
            return DocumentLibraryAction::OpenDocument(documents[index].document_id.clone());
        }
    }
    if y >= 2020 {
        if x < 810 && screen_index > 0 {
            return DocumentLibraryAction::PreviousScreen;
        }
        if x >= 810 && (screen_index + 1) * DOCUMENTS_PER_SCREEN < documents.len() {
            return DocumentLibraryAction::NextScreen;
        }
    }
    DocumentLibraryAction::None
}

fn draw_back_button(image: &mut BgraImage) {
    image.fill_rounded_rectangle(24, 20, 88, 88, 22.0, [0xef, 0xee, 0xea]);
    image.fill_rounded_rectangle(55, 61, 36, 6, 3.0, [0x35, 0x35, 0x34]);
    image.fill_rounded_rectangle(50, 48, 6, 20, 3.0, [0x35, 0x35, 0x34]);
    image.fill_rounded_rectangle(50, 66, 6, 20, 3.0, [0x35, 0x35, 0x34]);
}

fn draw_new_notebook_button(image: &mut BgraImage) {
    image.fill_rounded_rectangle(1180, 20, 376, 88, 22.0, [0xd9, 0xe8, 0xf4]);
    image.fill_rounded_rectangle(1214, 61, 28, 6, 3.0, [0x25, 0x25, 0x24]);
    image.fill_rounded_rectangle(1225, 50, 6, 28, 3.0, [0x25, 0x25, 0x24]);
    draw_text(
        image,
        1264,
        77,
        "Nouveau carnet",
        28,
        270,
        [0x25, 0x25, 0x24],
    );
}

fn draw_document_row(image: &mut BgraImage, y: usize, document: &DocumentSummary, active: bool) {
    image.fill_rounded_rectangle(
        ROW_X,
        y,
        ROW_WIDTH,
        ROW_HEIGHT,
        20.0,
        if active {
            [0xd9, 0xe8, 0xf4]
        } else {
            [0xff, 0xff, 0xff]
        },
    );
    image.fill_rounded_rectangle(96, y + 24, 64, 78, 7.0, [0x4a, 0x4a, 0x47]);
    image.fill_rounded_rectangle(102, y + 30, 52, 66, 4.0, [0xfa, 0xf9, 0xf6]);
    draw_text(
        image,
        196,
        y + 56,
        &document.title,
        31,
        1120,
        [0x25, 0x25, 0x24],
    );
    let pages = if document.page_count == 1 {
        "1 page".to_owned()
    } else {
        format!("{} pages", document.page_count)
    };
    draw_text(
        image,
        196,
        y + 96,
        &format!("{pages}  ·  page {}", document.page_number),
        23,
        900,
        [0x6a, 0x6a, 0x66],
    );
    if active {
        image.fill_rounded_rectangle(1472, y + 49, 28, 28, 14.0, [0x3b, 0x76, 0x9f]);
    }
}

fn draw_screen_navigation(image: &mut BgraImage, document_count: usize, screen_index: usize) {
    let screen_count = document_count.div_ceil(DOCUMENTS_PER_SCREEN).max(1);
    draw_text(
        image,
        742,
        2090,
        &format!("{} / {screen_count}", screen_index + 1),
        25,
        140,
        [0x5a, 0x5a, 0x57],
    );
    if screen_index > 0 {
        draw_text(image, 650, 2090, "‹", 42, 50, [0x25, 0x25, 0x24]);
    }
    if screen_index + 1 < screen_count {
        draw_text(image, 920, 2090, "›", 42, 50, [0x25, 0x25, 0x24]);
    }
}
