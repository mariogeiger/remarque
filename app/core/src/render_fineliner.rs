use crate::bgra_image::{BgraImage, PixelRectangle};
use crate::color::Color;
use crate::stroke::StrokePoint;

pub fn render_fineliner(image: &mut BgraImage, points: &[StrokePoint], color: Color) {
    let mut rasterizer = FinelinerRasterizer::new(color);
    for &point in points {
        rasterizer.append_point(image, point);
    }
    rasterizer.finish(image);
}

pub fn render_fineliner_raster_points(
    image: &mut BgraImage,
    points: &[FinelinerRasterPoint],
    color: Color,
) {
    let mut rasterizer = FinelinerRasterizer::new(color);
    for &point in points {
        rasterizer.append_point(image, point);
    }
    rasterizer.finish(image);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinelinerRasterPoint {
    pub x: f32,
    pub y: f32,
    pub width: f32,
}

impl From<StrokePoint> for FinelinerRasterPoint {
    fn from(point: StrokePoint) -> Self {
        Self {
            x: point.x,
            y: point.y,
            width: raster_width_from_stored_quarters(point.width_quarter_pixels, 1.0),
        }
    }
}

pub fn raster_width_from_stored_quarters(width_quarters: u16, view_scale: f32) -> f32 {
    0.75 + f32::from(width_quarters) * view_scale * 0.25
}

pub fn nonzero_coverage_rectangle(
    start: FinelinerRasterPoint,
    end: FinelinerRasterPoint,
    image_width: usize,
    image_height: usize,
) -> PixelRectangle {
    let radius = start.width.max(end.width) * 0.5;
    let pixel_bound =
        |edge: f32, limit: usize| (edge - 0.5).ceil().clamp(0.0, limit as f32) as usize;
    let left = pixel_bound(start.x.min(end.x) - radius, image_width);
    let top = pixel_bound(start.y.min(end.y) - radius, image_height);
    let right = pixel_bound(start.x.max(end.x) + radius, image_width);
    let bottom = pixel_bound(start.y.max(end.y) + radius, image_height);
    PixelRectangle {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

pub struct FinelinerRasterizer {
    color: [u8; 4],
    previous_point: Option<FinelinerRasterPoint>,
    previous_edges: Option<SegmentEdges>,
    rendered_start_cap: bool,
    rendered_end_cap: bool,
}

impl FinelinerRasterizer {
    pub fn new(color: Color) -> Self {
        let rgb = color.rgb();
        Self {
            color: [rgb[2], rgb[1], rgb[0], 0xff],
            previous_point: None,
            previous_edges: None,
            rendered_start_cap: false,
            rendered_end_cap: false,
        }
    }

    pub fn append_point(&mut self, image: &mut BgraImage, point: impl Into<FinelinerRasterPoint>) {
        let point = point.into();
        if !point.x.is_finite() || !point.y.is_finite() {
            return;
        }
        let Some(start) = self.previous_point else {
            self.previous_point = Some(point);
            return;
        };
        if start.x == point.x && start.y == point.y {
            return;
        }
        let edges = segment_edges(start, point);
        if let Some(previous) = self.previous_edges {
            render_round_join(image, previous, edges, start, self.color);
        }
        render_coverage_quad(
            image,
            [
                edges.start_negative,
                edges.end_negative,
                edges.start_positive,
                edges.end_positive,
            ],
            [
                start.width * 0.5,
                point.width * 0.5,
                start.width * 0.5,
                point.width * 0.5,
            ],
            false,
            self.color,
        );
        if !self.rendered_start_cap {
            render_round_start_cap(image, edges, start, self.color);
            self.rendered_start_cap = true;
        }
        self.previous_point = Some(point);
        self.previous_edges = Some(edges);
    }

    pub fn finish(&mut self, image: &mut BgraImage) {
        if self.rendered_end_cap {
            return;
        }
        if let Some(end) = self.previous_point {
            if let Some(edges) = self.previous_edges {
                render_round_end_cap(image, edges, end, self.color);
            } else {
                render_antialiased_disc(image, end, self.color);
            }
            self.rendered_end_cap = true;
        }
    }
}

#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct CoverageVertex {
    position: Point,
    half_width: f32,
    signed_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AntialiasedCoverageVertex {
    pub x: f32,
    pub y: f32,
    pub half_width: f32,
    pub signed_distance: f32,
}

#[derive(Clone, Copy)]
struct SegmentEdges {
    start_negative: Point,
    end_negative: Point,
    start_positive: Point,
    end_positive: Point,
}

fn segment_edges(start: FinelinerRasterPoint, end: FinelinerRasterPoint) -> SegmentEdges {
    let delta_x = end.x - start.x;
    let delta_y = end.y - start.y;
    let inverse_length = delta_x.hypot(delta_y).recip();
    let normal_x = -delta_y * inverse_length;
    let normal_y = delta_x * inverse_length;
    let start_radius = start.width * 0.5;
    let end_radius = end.width * 0.5;
    SegmentEdges {
        start_negative: Point {
            x: start.x - normal_x * start_radius,
            y: start.y - normal_y * start_radius,
        },
        end_negative: Point {
            x: end.x - normal_x * end_radius,
            y: end.y - normal_y * end_radius,
        },
        start_positive: Point {
            x: start.x + normal_x * start_radius,
            y: start.y + normal_y * start_radius,
        },
        end_positive: Point {
            x: end.x + normal_x * end_radius,
            y: end.y + normal_y * end_radius,
        },
    }
}

fn render_round_start_cap(
    image: &mut BgraImage,
    edges: SegmentEdges,
    start: FinelinerRasterPoint,
    bgra: [u8; 4],
) {
    let center = Point {
        x: start.x,
        y: start.y,
    };
    let radius = start.width * 0.5;
    if radius <= 0.0 {
        return;
    }
    let divisions = radius.ceil().min(30.0) as usize;
    let iterations = if divisions <= 3 { 1 } else { divisions / 2 };
    let angle_step = std::f32::consts::PI / divisions as f32;
    let mut positive = edges.start_positive;
    let mut negative = edges.start_negative;
    let mut positive_angle = (positive.y - center.y).atan2(positive.x - center.x);
    let mut negative_angle = positive_angle + std::f32::consts::PI;
    for _ in 0..iterations {
        positive_angle += angle_step;
        negative_angle -= angle_step;
        let next_positive = Point {
            x: center.x + positive_angle.cos() * radius,
            y: center.y + positive_angle.sin() * radius,
        };
        let next_negative = Point {
            x: center.x + negative_angle.cos() * radius,
            y: center.y + negative_angle.sin() * radius,
        };
        render_coverage_quad(
            image,
            [positive, next_positive, negative, next_negative],
            [radius; 4],
            false,
            bgra,
        );
        positive = next_positive;
        negative = next_negative;
    }
}

fn render_antialiased_disc(image: &mut BgraImage, center: FinelinerRasterPoint, bgra: [u8; 4]) {
    let radius = center.width * 0.5;
    if !radius.is_finite() || radius <= 0.0 {
        return;
    }
    let rectangle = nonzero_coverage_rectangle(center, center, image.width(), image.height());
    for row in rectangle.y..rectangle.y + rectangle.height {
        for column in rectangle.x..rectangle.x + rectangle.width {
            let distance = (column as f32 + 0.5 - center.x).hypot(row as f32 + 0.5 - center.y);
            image.blend_bgra_coverage(column, row, bgra, antialiased_coverage(distance, radius));
        }
    }
}

fn render_round_end_cap(
    image: &mut BgraImage,
    edges: SegmentEdges,
    end: FinelinerRasterPoint,
    bgra: [u8; 4],
) {
    let center = Point { x: end.x, y: end.y };
    let radius = end.width * 0.5;
    if radius <= 0.0 {
        return;
    }
    let divisions = radius.ceil().min(30.0) as usize;
    let iterations = if divisions <= 3 { 1 } else { divisions / 2 };
    let angle_step = std::f32::consts::PI / divisions as f32;
    let mut positive = edges.end_negative;
    let mut negative = edges.end_positive;
    let mut positive_angle = (positive.y - center.y).atan2(positive.x - center.x);
    let mut negative_angle = positive_angle + std::f32::consts::PI;
    for _ in 0..iterations {
        positive_angle += angle_step;
        negative_angle -= angle_step;
        let next_positive = Point {
            x: center.x + positive_angle.cos() * radius,
            y: center.y + positive_angle.sin() * radius,
        };
        let next_negative = Point {
            x: center.x + negative_angle.cos() * radius,
            y: center.y + negative_angle.sin() * radius,
        };
        render_coverage_quad(
            image,
            [positive, next_positive, negative, next_negative],
            [radius; 4],
            false,
            bgra,
        );
        positive = next_positive;
        negative = next_negative;
    }
}

fn render_round_join(
    image: &mut BgraImage,
    previous: SegmentEdges,
    current: SegmentEdges,
    center_point: FinelinerRasterPoint,
    bgra: [u8; 4],
) {
    let center = Point {
        x: center_point.x,
        y: center_point.y,
    };
    let first_angle =
        (previous.end_negative.y - center.y).atan2(previous.end_negative.x - center.x);
    let next_angle =
        (current.start_negative.y - center.y).atan2(current.start_negative.x - center.x);
    let mut angle = first_angle;
    let mut angle_delta = next_angle - first_angle;
    if angle_delta < -std::f32::consts::PI {
        angle_delta += std::f32::consts::TAU;
    } else if angle_delta > std::f32::consts::PI {
        angle_delta -= std::f32::consts::TAU;
    }
    let radius = center_point.width * 0.5;
    let divisions = (angle_delta.abs() * radius / std::f32::consts::PI)
        .ceil()
        .min(30.0) as usize;
    if angle_delta.abs() >= std::f32::consts::PI * 0.1 && divisions > 0 {
        let mut positive = previous.end_negative;
        let mut negative = previous.end_positive;
        for _ in 0..divisions {
            angle += angle_delta / divisions as f32;
            let next_positive = Point {
                x: center.x + angle.cos() * radius,
                y: center.y + angle.sin() * radius,
            };
            let next_negative = Point {
                x: center.x - angle.cos() * radius,
                y: center.y - angle.sin() * radius,
            };
            render_coverage_quad(
                image,
                [positive, next_positive, negative, next_negative],
                [radius; 4],
                true,
                bgra,
            );
            positive = next_positive;
            negative = next_negative;
        }
    } else {
        render_coverage_quad(
            image,
            [
                current.start_negative,
                previous.end_negative,
                current.start_positive,
                previous.end_positive,
            ],
            [radius; 4],
            true,
            bgra,
        );
    }
}

fn render_coverage_quad(
    image: &mut BgraImage,
    mut positions: [Point; 4],
    mut half_widths: [f32; 4],
    align_opposite_side: bool,
    bgra: [u8; 4],
) {
    if align_opposite_side {
        let first_side = Point {
            x: positions[1].x - positions[0].x,
            y: positions[1].y - positions[0].y,
        };
        let opposite_side = Point {
            x: positions[3].x - positions[2].x,
            y: positions[3].y - positions[2].y,
        };
        if first_side.x * opposite_side.x + first_side.y * opposite_side.y < 0.0 {
            positions.swap(2, 3);
            half_widths.swap(2, 3);
        }
    }
    let vertex = |position, half_width, sign: f32| CoverageVertex {
        position,
        half_width,
        signed_distance: sign * half_width,
    };
    rasterize_coverage_triangle(
        image,
        [
            vertex(positions[0], half_widths[0], 1.0),
            vertex(positions[1], half_widths[1], 1.0),
            vertex(positions[2], half_widths[2], -1.0),
        ],
        bgra,
    );
    rasterize_coverage_triangle(
        image,
        [
            vertex(positions[2], half_widths[2], -1.0),
            vertex(positions[1], half_widths[1], 1.0),
            vertex(positions[3], half_widths[3], -1.0),
        ],
        bgra,
    );
}

fn rasterize_coverage_triangle(
    image: &mut BgraImage,
    vertices: [CoverageVertex; 3],
    bgra: [u8; 4],
) {
    let minimum_y = vertices
        .iter()
        .map(|vertex| vertex.position.y)
        .fold(f32::INFINITY, f32::min);
    let maximum_y = vertices
        .iter()
        .map(|vertex| vertex.position.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let first_row = (minimum_y - 0.5).floor() as isize + 1;
    let last_row = (maximum_y - 0.5).floor() as isize;
    for row in first_row.max(0)..=last_row.min(image.height() as isize - 1) {
        let y = row as f32 + 0.5;
        let mut intersections = Vec::with_capacity(2);
        for edge in [(0, 1), (1, 2), (2, 0)] {
            let (mut lower, mut upper) = (vertices[edge.0], vertices[edge.1]);
            if lower.position.y > upper.position.y {
                std::mem::swap(&mut lower, &mut upper);
            }
            if lower.position.y == upper.position.y || y <= lower.position.y || y > upper.position.y
            {
                continue;
            }
            let amount = (y - lower.position.y) / (upper.position.y - lower.position.y);
            intersections.push(CoverageVertex {
                position: Point {
                    x: lower.position.x + (upper.position.x - lower.position.x) * amount,
                    y,
                },
                half_width: lower.half_width + (upper.half_width - lower.half_width) * amount,
                signed_distance: lower.signed_distance
                    + (upper.signed_distance - lower.signed_distance) * amount,
            });
        }
        if intersections.len() < 2 {
            continue;
        }
        intersections.sort_by(|left, right| left.position.x.total_cmp(&right.position.x));
        let left = intersections[0];
        let right = *intersections.last().unwrap();
        let first_column = (left.position.x + 0.5).floor() as isize;
        let last_column = (right.position.x - 0.5) as isize;
        let span = right.position.x - left.position.x;
        for column in first_column.max(0)..=last_column.min(image.width() as isize - 1) {
            let amount = if span == 0.0 {
                0.0
            } else {
                (column as f32 + 0.5 - left.position.x) / span
            };
            let half_width = left.half_width + (right.half_width - left.half_width) * amount;
            let signed_distance =
                left.signed_distance + (right.signed_distance - left.signed_distance) * amount;
            let coverage = antialiased_coverage(signed_distance.abs(), half_width);
            image.blend_bgra_coverage(column as usize, row as usize, bgra, coverage);
        }
    }
}

fn antialiased_coverage(distance: f32, half_width: f32) -> u8 {
    if half_width <= 0.0 {
        return 0;
    }
    let inner = half_width - 0.75;
    if distance < inner {
        255
    } else if distance <= half_width {
        ((1.0 - (distance - inner) / (half_width - inner)) * 255.0) as u8
    } else {
        0
    }
}

pub fn render_antialiased_triangle(
    image: &mut BgraImage,
    vertices: [AntialiasedCoverageVertex; 3],
    color: Color,
) {
    let rgb = color.rgb();
    rasterize_coverage_triangle(
        image,
        vertices.map(|vertex| CoverageVertex {
            position: Point {
                x: vertex.x,
                y: vertex.y,
            },
            half_width: vertex.half_width,
            signed_distance: vertex.signed_distance,
        }),
        [rgb[2], rgb[1], rgb[0], 0xff],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32, pressure: u8) -> StrokePoint {
        StrokePoint {
            x,
            y,
            two_segment_distance_quarters: 0,
            width_quarter_pixels: 8,
            direction: 0,
            pressure,
        }
    }

    #[test]
    fn opaque_black_segment_paints_its_center_black() {
        let mut image = BgraImage::filled(20, 20, [255, 255, 255]);
        render_fineliner(
            &mut image,
            &[point(2.0, 10.0, 255), point(18.0, 10.0, 255)],
            Color::Black,
        );
        assert_eq!(image.pixel(10, 9), [0, 0, 0, 255]);
    }

    #[test]
    fn pressure_does_not_change_fineliner_opacity() {
        let mut image = BgraImage::filled(20, 20, [255, 255, 255]);
        render_fineliner(
            &mut image,
            &[point(2.0, 10.0, 128), point(18.0, 10.0, 128)],
            Color::Black,
        );
        assert_eq!(image.pixel(10, 9), [0, 0, 0, 255]);
    }

    #[test]
    fn one_point_renders_an_antialiased_disc() {
        let mut image = BgraImage::filled(20, 20, [255, 255, 255]);
        render_fineliner(&mut image, &[point(10.5, 10.5, 255)], Color::Black);
        assert_eq!(image.pixel(10, 10), [0, 0, 0, 255]);
        assert_ne!(image.pixel(11, 10), [255, 255, 255, 255]);
        assert_eq!(image.pixel(12, 10), [255, 255, 255, 255]);
    }

    #[test]
    fn preserves_fractional_view_scale_in_raster_width() {
        assert_eq!(raster_width_from_stored_quarters(24, 3.1425083), 19.60505);
    }

    #[test]
    fn reproduces_captured_native_dirty_rectangle() {
        let width = raster_width_from_stored_quarters(24, 3.1425083);
        let rectangle = nonzero_coverage_rectangle(
            FinelinerRasterPoint {
                x: 722.9141,
                y: 361.59583,
                width,
            },
            FinelinerRasterPoint {
                x: 723.3006,
                y: 361.46204,
                width,
            },
            1620,
            2160,
        );
        assert_eq!(
            rectangle,
            PixelRectangle {
                x: 713,
                y: 352,
                width: 20,
                height: 19,
            }
        );
    }

    #[test]
    fn finishing_a_line_rounds_its_end() {
        let mut image = BgraImage::filled(24, 20, [255, 255, 255]);
        let mut rasterizer = FinelinerRasterizer::new(Color::Black);
        rasterizer.append_point(&mut image, point(2.0, 10.0, 255));
        rasterizer.append_point(&mut image, point(18.0, 10.0, 255));
        assert_eq!(image.pixel(18, 9), [255, 255, 255, 255]);
        rasterizer.finish(&mut image);
        assert_ne!(image.pixel(18, 9), [255, 255, 255, 255]);
    }
}
