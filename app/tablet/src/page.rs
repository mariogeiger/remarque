use crate::bgra_image::{BgraImage, PixelRectangle};
#[cfg(feature = "takeover")]
use crate::pdfium::RenderedPdfPage;
use crate::stroke::Stroke;

const WHITE: [u8; 3] = [0xff, 0xff, 0xff];

pub(crate) struct Page {
    pub background: Option<BgraImage>,
    pub rectangle: PixelRectangle,
    pub size_points: [f64; 2],
    pub strokes: Vec<Stroke>,
}

impl Page {
    pub fn blank(
        canvas_width: usize,
        canvas_height: usize,
        content_top: usize,
        strokes: Vec<Stroke>,
    ) -> Self {
        let content_height = canvas_height - content_top;
        Self {
            background: None,
            rectangle: PixelRectangle {
                x: 0,
                y: content_top,
                width: canvas_width,
                height: content_height,
            },
            size_points: [canvas_width as f64, content_height as f64],
            strokes,
        }
    }

    #[cfg(feature = "takeover")]
    pub fn from_rendered_pdf(rendered: RenderedPdfPage, strokes: Vec<Stroke>) -> Self {
        Self {
            background: Some(rendered.background),
            rectangle: rendered.page_rectangle,
            size_points: rendered.page_size_points,
            strokes,
        }
    }

    pub fn raster_background(&self, canvas_width: usize, canvas_height: usize) -> BgraImage {
        self.background
            .clone()
            .unwrap_or_else(|| BgraImage::filled(canvas_width, canvas_height, WHITE))
    }

    pub fn scene_width(&self, canvas_width: usize) -> usize {
        self.background
            .as_ref()
            .map_or(canvas_width, BgraImage::width)
    }

    pub fn scene_height(&self, canvas_height: usize) -> usize {
        self.background
            .as_ref()
            .map_or(canvas_height, BgraImage::height)
    }

    pub fn has_pdf_background(&self) -> bool {
        self.background.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_page_uses_the_same_raster_and_stroke_container() {
        let page = Page::blank(120, 200, 20, Vec::new());
        assert!(!page.has_pdf_background());
        assert_eq!(page.rectangle.width, 120);
        assert_eq!(page.rectangle.height, 180);
        assert_eq!(page.size_points, [120.0, 180.0]);
        assert!(page.strokes.is_empty());
        assert_eq!(page.scene_width(120), 120);
        assert_eq!(page.scene_height(200), 200);
        assert_eq!(
            page.raster_background(120, 200).pixel(0, 199),
            [0xff, 0xff, 0xff, 0xff]
        );
    }
}
