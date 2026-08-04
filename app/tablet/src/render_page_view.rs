use crate::bgra_image::{BgraImage, PixelRectangle};
use crate::render_fineliner::{
    FinelinerRasterPoint, nonzero_coverage_rectangle, raster_width_from_stored_quarters,
};
use crate::stroke::StrokePoint;
#[cfg(test)]
use crate::view_transform::midpoint;
use crate::view_transform::{Point, Size, ViewTransform};

pub(crate) const OUTSIDE_PAGE_RGB: [u8; 3] = [0xe5, 0xe4, 0xe1];
const OUTSIDE_PAGE_BGRA: [u8; 4] = [
    OUTSIDE_PAGE_RGB[2],
    OUTSIDE_PAGE_RGB[1],
    OUTSIDE_PAGE_RGB[0],
    0xff,
];

pub(crate) fn identity_transform(width: usize, height: usize) -> ViewTransform {
    ViewTransform {
        focal_point: Point {
            x: width as f64 * 0.5,
            y: height as f64 * 0.5,
        },
        scale: 1.0,
    }
}

pub(crate) fn transform_background_nearest_neighbor(
    background: &BgraImage,
    transform: ViewTransform,
    viewport: Size,
    output_width: usize,
    output_height: usize,
    content_top: usize,
) -> BgraImage {
    if background.width() == output_width
        && background.height() == output_height
        && transform == identity_transform(output_width, output_height)
    {
        return background.clone();
    }

    let mut pixels = Vec::with_capacity(output_width * output_height * 4);
    for _ in 0..output_width * output_height {
        pixels.extend_from_slice(&OUTSIDE_PAGE_BGRA);
    }
    let source_columns = (0..output_width)
        .map(|x| {
            let source_x = transform
                .view_to_scene(
                    Point {
                        x: x as f64,
                        y: 0.0,
                    },
                    viewport,
                )
                .x
                .floor() as isize;
            usize::try_from(source_x)
                .ok()
                .filter(|&source_x| source_x < background.width())
        })
        .collect::<Vec<_>>();

    for y in content_top.min(output_height)..output_height {
        let source_y = transform
            .view_to_scene(
                Point {
                    x: 0.0,
                    y: y as f64,
                },
                viewport,
            )
            .y
            .floor() as isize;
        let Some(source_y) = usize::try_from(source_y)
            .ok()
            .filter(|&source_y| source_y < background.height())
        else {
            continue;
        };
        for (x, source_x) in source_columns.iter().enumerate() {
            let Some(source_x) = source_x else {
                continue;
            };
            let source = (source_y * background.width() + source_x) * 4;
            let destination = (y * output_width + x) * 4;
            pixels[destination..destination + 4]
                .copy_from_slice(&background.pixels()[source..source + 4]);
        }
    }
    BgraImage::try_from_bgra(output_width, output_height, pixels)
        .expect("constructed BGRA image matches its dimensions")
}

pub(crate) fn transform_stroke_point(
    point: StrokePoint,
    transform: ViewTransform,
    viewport: Size,
) -> FinelinerRasterPoint {
    let position = transform.scene_to_view(
        Point {
            x: f64::from(point.x),
            y: f64::from(point.y),
        },
        viewport,
    );
    FinelinerRasterPoint {
        x: position.x as f32,
        y: position.y as f32,
        width: raster_width_from_stored_quarters(
            point.width_quarter_pixels,
            transform.scale as f32,
        ),
    }
}

pub(crate) fn eraser_preview_point(
    position: Point,
    width: f64,
    transform: ViewTransform,
    viewport: Size,
) -> FinelinerRasterPoint {
    let position = transform.scene_to_view(position, viewport);
    FinelinerRasterPoint {
        x: position.x as f32,
        y: position.y as f32,
        width: 0.75 + width as f32,
    }
}

pub(crate) fn fineliner_segment_rectangle(
    start: FinelinerRasterPoint,
    end: FinelinerRasterPoint,
    image_width: usize,
    image_height: usize,
) -> PixelRectangle {
    let bounds = nonzero_coverage_rectangle(start, end, image_width, image_height);
    PixelRectangle {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_transform_preserves_the_background() {
        let background = BgraImage::filled(4, 4, [12, 34, 56]);
        let transformed = transform_background_nearest_neighbor(
            &background,
            identity_transform(4, 4),
            Size {
                width: 4.0,
                height: 4.0,
            },
            4,
            4,
            0,
        );
        assert_eq!(transformed, background);
    }

    #[test]
    fn zoom_reuses_each_nearest_source_pixel() {
        let mut background = BgraImage::filled(4, 4, [255, 255, 255]);
        background.fill_rectangle(1, 1, 1, 1, [255, 0, 0]);
        background.fill_rectangle(2, 1, 1, 1, [0, 255, 0]);
        let transformed = transform_background_nearest_neighbor(
            &background,
            ViewTransform {
                focal_point: Point { x: 2.0, y: 2.0 },
                scale: 2.0,
            },
            Size {
                width: 4.0,
                height: 4.0,
            },
            4,
            4,
            0,
        );
        assert_eq!(transformed.pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(transformed.pixel(1, 0), [0, 0, 255, 255]);
        assert_eq!(transformed.pixel(2, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn positions_outside_the_scene_use_the_page_surround() {
        let background = BgraImage::filled(4, 4, [255, 255, 255]);
        let transformed = transform_background_nearest_neighbor(
            &background,
            ViewTransform {
                focal_point: Point { x: 0.0, y: 2.0 },
                scale: 1.0,
            },
            Size {
                width: 4.0,
                height: 4.0,
            },
            4,
            4,
            0,
        );
        assert_eq!(transformed.pixel(0, 0), OUTSIDE_PAGE_BGRA);
        assert_eq!(transformed.pixel(2, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn coordinate_helpers_share_the_same_view_transform() {
        let viewport = Size {
            width: 100.0,
            height: 80.0,
        };
        let transform = ViewTransform {
            focal_point: Point { x: 50.0, y: 40.0 },
            scale: 2.0,
        };
        assert_eq!(
            midpoint([Point { x: 2.0, y: 4.0 }, Point { x: 8.0, y: 10.0 }]),
            Point { x: 5.0, y: 7.0 }
        );
        let point = transform_stroke_point(
            StrokePoint {
                x: 50.0,
                y: 40.0,
                two_segment_distance_quarters: 0,
                width_quarter_pixels: 8,
                direction: 0,
                pressure: 0,
            },
            transform,
            viewport,
        );
        assert_eq!((point.x, point.y, point.width), (50.0, 40.0, 4.75));
        let preview = eraser_preview_point(Point { x: 50.0, y: 40.0 }, 30.0, transform, viewport);
        assert_eq!((preview.x, preview.y, preview.width), (50.0, 40.0, 30.75));
        let rectangle = fineliner_segment_rectangle(point, point, 100, 80);
        assert!(rectangle.width > 0 && rectangle.height > 0);
    }
}
