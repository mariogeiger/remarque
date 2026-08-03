use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::render_fineliner::{FinelinerRasterPoint, render_fineliner_raster_points};
use crate::view_transform::{Point, Size, ViewTransform, viewport_indicator};

const MARGIN: f64 = 32.0;
const EDGE_INSET: f32 = 24.0;
const WIDTH_QUARTER_PIXELS: u16 = 6 * 4;

pub(crate) fn draw_viewport_indicators(
    image: &mut BgraImage,
    transform: ViewTransform,
    viewport: Size,
) {
    let horizontal = viewport_indicator(
        transform.focal_point.x,
        transform.scale,
        viewport.width,
        0.0,
        viewport.width,
    );
    let vertical = viewport_indicator(
        transform.focal_point.y,
        transform.scale,
        viewport.height,
        0.0,
        viewport.height,
    );
    let track_width = viewport.width - MARGIN * 2.0;
    let track_height = viewport.height - MARGIN * 2.0;
    if let Some(indicator) = horizontal {
        draw_position_indicator(
            image,
            Point {
                x: MARGIN + track_width * indicator.start,
                y: viewport.height - f64::from(EDGE_INSET),
            },
            Point {
                x: MARGIN + track_width * (indicator.start + indicator.length),
                y: viewport.height - f64::from(EDGE_INSET),
            },
        );
    }
    if let Some(indicator) = vertical {
        draw_position_indicator(
            image,
            Point {
                x: viewport.width - f64::from(EDGE_INSET),
                y: MARGIN + track_height * indicator.start,
            },
            Point {
                x: viewport.width - f64::from(EDGE_INSET),
                y: MARGIN + track_height * (indicator.start + indicator.length),
            },
        );
    }
}

fn draw_position_indicator(image: &mut BgraImage, start: Point, end: Point) {
    let point = |position: Point| FinelinerRasterPoint {
        x: position.x as f32,
        y: position.y as f32,
        width: 0.75 + f32::from(WIDTH_QUARTER_PIXELS) * 0.25,
    };
    render_fineliner_raster_points(image, &[point(start), point(end)], Color::Gray);
}
