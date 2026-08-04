use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::render_fineliner::{FinelinerRasterPoint, render_fineliner_raster_points};
use crate::view_transform::{Bounds, Point, Size, ViewTransform, viewport_indicator};

const MARGIN: f64 = 32.0;
const EDGE_INSET: f32 = 24.0;
const WIDTH_QUARTER_PIXELS: u16 = 6 * 4;

pub(crate) fn draw_viewport_indicators(
    image: &mut BgraImage,
    transform: ViewTransform,
    view_size: Size,
    visible_view: Bounds,
    page: Bounds,
) {
    let [horizontal, vertical] = viewport_indicators(transform, view_size, visible_view, page);
    let track_width = visible_view.size.width - MARGIN * 2.0;
    let track_height = visible_view.size.height - MARGIN * 2.0;
    if let Some(indicator) = horizontal {
        draw_position_indicator(
            image,
            Point {
                x: visible_view.origin.x + MARGIN + track_width * indicator.start,
                y: visible_view.origin.y + visible_view.size.height - f64::from(EDGE_INSET),
            },
            Point {
                x: visible_view.origin.x
                    + MARGIN
                    + track_width * (indicator.start + indicator.length),
                y: visible_view.origin.y + visible_view.size.height - f64::from(EDGE_INSET),
            },
        );
    }
    if let Some(indicator) = vertical {
        draw_position_indicator(
            image,
            Point {
                x: visible_view.origin.x + visible_view.size.width - f64::from(EDGE_INSET),
                y: visible_view.origin.y + MARGIN + track_height * indicator.start,
            },
            Point {
                x: visible_view.origin.x + visible_view.size.width - f64::from(EDGE_INSET),
                y: visible_view.origin.y
                    + MARGIN
                    + track_height * (indicator.start + indicator.length),
            },
        );
    }
}

fn viewport_indicators(
    transform: ViewTransform,
    view_size: Size,
    visible_view: Bounds,
    page: Bounds,
) -> [Option<crate::view_transform::FractionalInterval>; 2] {
    let visible_page = transform.view_bounds_to_scene(visible_view, view_size);
    [
        viewport_indicator(
            visible_page.origin.x,
            visible_page.size.width,
            page.origin.x,
            page.size.width,
        ),
        viewport_indicator(
            visible_page.origin.y,
            visible_page.size.height,
            page.origin.y,
            page.size.height,
        ),
    ]
}

fn draw_position_indicator(image: &mut BgraImage, start: Point, end: Point) {
    let point = |position: Point| FinelinerRasterPoint {
        x: position.x as f32,
        y: position.y as f32,
        width: 0.75 + f32::from(WIDTH_QUARTER_PIXELS) * 0.25,
    };
    render_fineliner_raster_points(image, &[point(start), point(end)], Color::Gray);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicators_compare_the_viewport_against_the_scene() {
        let mut image = BgraImage::filled(1000, 800, [255, 255, 255]);
        let transform = ViewTransform {
            focal_point: Point { x: 500.0, y: 400.0 },
            scale: 1.0,
        };
        let viewport = Size {
            width: 1000.0,
            height: 800.0,
        };
        let visible_view = Bounds {
            origin: Point { x: 0.0, y: 80.0 },
            size: Size {
                width: 1000.0,
                height: 720.0,
            },
        };
        let page = Bounds {
            origin: Point { x: 0.0, y: 80.0 },
            size: Size {
                width: 1000.0,
                height: 1920.0,
            },
        };
        draw_viewport_indicators(&mut image, transform, viewport, visible_view, page);
        let [horizontal, vertical] = viewport_indicators(transform, viewport, visible_view, page);
        assert!(horizontal.is_none());
        let vertical = vertical.unwrap();
        assert_eq!(vertical.length, 0.375);
        assert_eq!(vertical.start, 0.0);
        assert_ne!(image.pixel(976, 112), [255, 255, 255, 255]);
    }

    #[test]
    fn vertical_indicator_is_visible_at_minimum_scale_for_a_tall_page() {
        let viewport = Size {
            width: 1000.0,
            height: 800.0,
        };
        let visible_view = Bounds {
            origin: Point { x: 0.0, y: 80.0 },
            size: Size {
                width: 1000.0,
                height: 720.0,
            },
        };
        let page = Bounds {
            origin: Point { x: 0.0, y: 80.0 },
            size: Size {
                width: 1000.0,
                height: 1000.0,
            },
        };
        let [horizontal, vertical] = viewport_indicators(
            ViewTransform {
                focal_point: Point { x: 500.0, y: 400.0 },
                scale: 1.0,
            },
            viewport,
            visible_view,
            page,
        );
        assert!(horizontal.is_none());
        assert_eq!(vertical.unwrap().length, 0.72);
    }
}
