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
    scene: Size,
) {
    let [horizontal, vertical] = viewport_indicators(transform, viewport, scene);
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

fn viewport_indicators(
    transform: ViewTransform,
    viewport: Size,
    scene: Size,
) -> [Option<crate::view_transform::FractionalInterval>; 2] {
    [
        viewport_indicator(
            transform.focal_point.x,
            transform.scale,
            viewport.width,
            0.0,
            scene.width,
        ),
        viewport_indicator(
            transform.focal_point.y,
            transform.scale,
            viewport.height,
            0.0,
            scene.height,
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
            focal_point: Point { x: 500.0, y: 500.0 },
            scale: 1.0,
        };
        let viewport = Size {
            width: 1000.0,
            height: 800.0,
        };
        let scene = Size {
            width: 1000.0,
            height: 2000.0,
        };
        draw_viewport_indicators(&mut image, transform, viewport, scene);
        let [horizontal, vertical] = viewport_indicators(transform, viewport, scene);
        assert!(horizontal.is_none());
        let vertical = vertical.unwrap();
        assert_eq!(vertical.length, 0.4);
        assert_eq!(vertical.start, 0.05);
        assert_ne!(image.pixel(976, 72), [255, 255, 255, 255]);
    }
}
