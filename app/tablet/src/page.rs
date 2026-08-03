use crate::bgra_image::{BgraImage, PixelRectangle};
#[cfg(feature = "takeover")]
use crate::document_library::DocumentPage;
#[cfg(feature = "takeover")]
use crate::pdfium::RenderedPdfPage;
#[cfg(feature = "takeover")]
use crate::pdfium::render_pdf_page;
#[cfg(feature = "takeover")]
use crate::render_fineliner::{
    FinelinerRasterPoint, raster_width_from_stored_quarters, render_fineliner_raster_points,
};
use crate::stroke::Stroke;
#[cfg(feature = "takeover")]
use std::io;

const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const PAPER_PRO_PIXELS_PER_INCH: f64 = 229.0;
const POINTS_PER_INCH: f64 = 72.0;

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
            size_points: [
                canvas_width as f64 * POINTS_PER_INCH / PAPER_PRO_PIXELS_PER_INCH,
                content_height as f64 * POINTS_PER_INCH / PAPER_PRO_PIXELS_PER_INCH,
            ],
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

    #[cfg(feature = "takeover")]
    pub fn from_document_page(
        page: &DocumentPage,
        canvas_width: usize,
        canvas_height: usize,
        content_top: usize,
    ) -> io::Result<Self> {
        let Some(source) = &page.background else {
            return Ok(Self::blank(
                canvas_width,
                canvas_height,
                content_top,
                page.strokes.clone(),
            ));
        };
        let rendered = render_pdf_page(
            &source.source_path,
            source.page_index,
            canvas_width,
            canvas_height,
            content_top,
        )?;
        Ok(Self::from_rendered_pdf(rendered, page.strokes.clone()))
    }

    #[cfg(feature = "takeover")]
    pub fn flatten(&self, canvas_width: usize, canvas_height: usize) -> io::Result<BgraImage> {
        let background = self.raster_background(canvas_width, canvas_height);
        let mut flattened = BgraImage::try_from_bgra(
            self.rectangle.width,
            self.rectangle.height,
            background.copy_rectangle(
                self.rectangle.x,
                self.rectangle.y,
                self.rectangle.width,
                self.rectangle.height,
            ),
        )
        .map_err(io::Error::other)?;
        for stroke in &self.strokes {
            let points = stroke
                .points
                .iter()
                .map(|point| FinelinerRasterPoint {
                    x: point.x,
                    y: point.y,
                    width: raster_width_from_stored_quarters(point.width_quarter_pixels, 1.0),
                })
                .collect::<Vec<_>>();
            render_fineliner_raster_points(&mut flattened, &points, stroke.color);
        }
        Ok(flattened)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_page_uses_the_same_raster_and_stroke_container() {
        let page = Page::blank(120, 200, 20, Vec::new());
        assert!(page.background.is_none());
        assert_eq!(page.rectangle.width, 120);
        assert_eq!(page.rectangle.height, 180);
        assert_eq!(
            page.size_points,
            [
                120.0 * POINTS_PER_INCH / PAPER_PRO_PIXELS_PER_INCH,
                180.0 * POINTS_PER_INCH / PAPER_PRO_PIXELS_PER_INCH,
            ]
        );
        assert!(page.strokes.is_empty());
        assert_eq!(page.scene_width(120), 120);
        assert_eq!(page.scene_height(200), 200);
        assert_eq!(
            page.raster_background(120, 200).pixel(0, 199),
            [0xff, 0xff, 0xff, 0xff]
        );
    }
}
