use crate::stroke::StrokePoint;
use crate::view_transform::Point;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EraserThickness {
    Thin,
    Medium,
    Thick,
}

impl EraserThickness {
    pub const fn pixels(self) -> f64 {
        match self {
            Self::Thin => 30.0,
            Self::Medium => 60.0,
            Self::Thick => 90.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Interval {
    start: f64,
    end: f64,
}

pub fn erase_stroke(
    stroke: &[StrokePoint],
    eraser_centerline: &[Point],
    eraser_width: f64,
) -> Vec<Vec<StrokePoint>> {
    if stroke.is_empty() || eraser_centerline.is_empty() {
        return vec![stroke.to_vec()];
    }
    if stroke.len() == 1 {
        let point = stroke[0];
        let center = Point {
            x: f64::from(point.x),
            y: f64::from(point.y),
        };
        let radius = (f64::from(point.width_quarter_pixels) * 0.25 + eraser_width) * 0.5;
        return if erased_intervals(center, center, eraser_centerline, radius).is_empty() {
            vec![stroke.to_vec()]
        } else {
            Vec::new()
        };
    }

    let mut sections = Vec::new();
    let mut current = Vec::new();
    for source in stroke.windows(2) {
        let start = source[0];
        let end = source[1];
        let source_width =
            f64::from(start.width_quarter_pixels.max(end.width_quarter_pixels)) * 0.25;
        let erased = erased_intervals(
            Point {
                x: f64::from(start.x),
                y: f64::from(start.y),
            },
            Point {
                x: f64::from(end.x),
                y: f64::from(end.y),
            },
            eraser_centerline,
            (source_width + eraser_width) * 0.5,
        );

        if start.x == end.x && start.y == end.y {
            if erased.is_empty() {
                if current.is_empty() {
                    current.push(start);
                }
                current.push(end);
            } else {
                finish_section(&mut sections, &mut current);
            }
            continue;
        }

        let mut cursor = 0.0;
        for interval in erased {
            append_kept_interval(&mut current, start, end, cursor, interval.start);
            if interval.start > cursor {
                finish_section(&mut sections, &mut current);
            }
            cursor = cursor.max(interval.end);
        }
        append_kept_interval(&mut current, start, end, cursor, 1.0);
        if cursor < 1.0 {
            continue;
        }
        finish_section(&mut sections, &mut current);
    }
    finish_section(&mut sections, &mut current);
    sections
}

fn append_kept_interval(
    section: &mut Vec<StrokePoint>,
    start: StrokePoint,
    end: StrokePoint,
    from: f64,
    to: f64,
) {
    if to - from <= f64::EPSILON {
        return;
    }
    let from_point = if from <= 0.0 {
        start
    } else {
        start.interpolate(end, from as f32)
    };
    let to_point = if to >= 1.0 {
        end
    } else {
        start.interpolate(end, to as f32)
    };
    if section.is_empty() {
        section.push(from_point);
    }
    section.push(to_point);
}

fn finish_section(sections: &mut Vec<Vec<StrokePoint>>, current: &mut Vec<StrokePoint>) {
    if current.len() >= 2 {
        sections.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn erased_intervals(
    source_start: Point,
    source_end: Point,
    eraser: &[Point],
    radius: f64,
) -> Vec<Interval> {
    let mut intervals = Vec::new();
    if eraser.len() == 1 {
        if let Some(interval) = line_circle_interval(source_start, source_end, eraser[0], radius) {
            intervals.push(interval);
        }
    } else {
        for segment in eraser.windows(2) {
            intervals.extend(line_capsule_intervals(
                source_start,
                source_end,
                segment[0],
                segment[1],
                radius,
            ));
        }
    }
    merge_intervals(intervals)
}

fn line_capsule_intervals(
    source_start: Point,
    source_end: Point,
    capsule_start: Point,
    capsule_end: Point,
    radius: f64,
) -> Vec<Interval> {
    let mut intervals = Vec::with_capacity(3);
    if let Some(interval) = line_circle_interval(source_start, source_end, capsule_start, radius) {
        intervals.push(interval);
    }
    if let Some(interval) = line_circle_interval(source_start, source_end, capsule_end, radius) {
        intervals.push(interval);
    }

    let source_delta = subtract(source_end, source_start);
    let capsule_delta = subtract(capsule_end, capsule_start);
    let capsule_length_squared = dot(capsule_delta, capsule_delta);
    if capsule_length_squared > f64::EPSILON {
        let relative_start = subtract(source_start, capsule_start);
        let projection = interval_where_linear_between(
            dot(relative_start, capsule_delta),
            dot(source_delta, capsule_delta),
            0.0,
            capsule_length_squared,
        );
        let strip = interval_where_absolute_linear_at_most(
            cross(capsule_delta, relative_start),
            cross(capsule_delta, source_delta),
            radius * capsule_length_squared.sqrt(),
        );
        if let (Some(projection), Some(strip)) = (projection, strip)
            && let Some(overlap) = intersect(projection, strip)
        {
            intervals.push(overlap);
        }
    }
    intervals
}

fn line_circle_interval(
    source_start: Point,
    source_end: Point,
    center: Point,
    radius: f64,
) -> Option<Interval> {
    let delta = subtract(source_end, source_start);
    let offset = subtract(source_start, center);
    let quadratic = dot(delta, delta);
    if quadratic <= f64::EPSILON {
        return (dot(offset, offset) <= radius * radius).then_some(Interval {
            start: 0.0,
            end: 1.0,
        });
    }
    let linear = dot(offset, delta);
    let constant = dot(offset, offset) - radius * radius;
    let discriminant = linear * linear - quadratic * constant;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    clip_unit_interval(Interval {
        start: (-linear - root) / quadratic,
        end: (-linear + root) / quadratic,
    })
}

fn interval_where_linear_between(
    start: f64,
    delta: f64,
    minimum: f64,
    maximum: f64,
) -> Option<Interval> {
    if delta.abs() <= f64::EPSILON {
        return (minimum <= start && start <= maximum).then_some(Interval {
            start: 0.0,
            end: 1.0,
        });
    }
    let first = (minimum - start) / delta;
    let second = (maximum - start) / delta;
    clip_unit_interval(Interval {
        start: first.min(second),
        end: first.max(second),
    })
}

fn interval_where_absolute_linear_at_most(
    start: f64,
    delta: f64,
    maximum: f64,
) -> Option<Interval> {
    interval_where_linear_between(start, delta, -maximum, maximum)
}

fn clip_unit_interval(interval: Interval) -> Option<Interval> {
    intersect(
        interval,
        Interval {
            start: 0.0,
            end: 1.0,
        },
    )
}

fn intersect(left: Interval, right: Interval) -> Option<Interval> {
    let intersection = Interval {
        start: left.start.max(right.start),
        end: left.end.min(right.end),
    };
    (intersection.start <= intersection.end).then_some(intersection)
}

fn merge_intervals(mut intervals: Vec<Interval>) -> Vec<Interval> {
    intervals.sort_by(|left, right| left.start.total_cmp(&right.start));
    let mut merged: Vec<Interval> = Vec::new();
    for interval in intervals {
        if let Some(last) = merged.last_mut()
            && interval.start <= last.end
        {
            last.end = last.end.max(interval.end);
            continue;
        }
        merged.push(interval);
    }
    merged
}

fn subtract(left: Point, right: Point) -> Point {
    Point {
        x: left.x - right.x,
        y: left.y - right.y,
    }
}

fn dot(left: Point, right: Point) -> f64 {
    left.x * right.x + left.y * right.y
}

fn cross(left: Point, right: Point) -> f64 {
    left.x * right.y - left.y * right.x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32) -> StrokePoint {
        StrokePoint {
            x,
            y,
            two_segment_distance_quarters: 40,
            width_quarter_pixels: 8,
            direction: 12,
            pressure: 200,
        }
    }

    #[test]
    fn reproduces_recovered_eraser_width_presets() {
        assert_eq!(EraserThickness::Thin.pixels(), 30.0);
        assert_eq!(EraserThickness::Medium.pixels(), 60.0);
        assert_eq!(EraserThickness::Thick.pixels(), 90.0);
    }

    #[test]
    fn crossing_eraser_splits_one_line_into_two_sections() {
        let stroke = [point(0.0, 0.0), point(100.0, 0.0)];
        let eraser = [Point { x: 50.0, y: -20.0 }, Point { x: 50.0, y: 20.0 }];
        let sections = erase_stroke(&stroke, &eraser, 20.0);
        assert_eq!(sections.len(), 2);
        assert!((sections[0].last().unwrap().x - 39.0).abs() < 1e-4);
        assert!((sections[1].first().unwrap().x - 61.0).abs() < 1e-4);
        assert_eq!(sections[0].last().unwrap().pressure, 200);
    }

    #[test]
    fn disjoint_eraser_keeps_the_original_section() {
        let stroke = [point(0.0, 0.0), point(100.0, 0.0)];
        let eraser = [Point { x: 50.0, y: 50.0 }];
        assert_eq!(erase_stroke(&stroke, &eraser, 20.0), vec![stroke.to_vec()]);
    }

    #[test]
    fn covering_eraser_deletes_the_line() {
        let stroke = [point(0.0, 0.0), point(100.0, 0.0)];
        let eraser = [Point { x: 50.0, y: 0.0 }];
        assert!(erase_stroke(&stroke, &eraser, 200.0).is_empty());
    }

    #[test]
    fn eraser_deletes_an_intersected_single_point_stroke() {
        let stroke = [point(50.0, 50.0)];
        let eraser = [Point { x: 55.0, y: 50.0 }];
        assert!(erase_stroke(&stroke, &eraser, 20.0).is_empty());
    }

    #[test]
    fn eraser_keeps_a_disjoint_single_point_stroke() {
        let stroke = [point(50.0, 50.0)];
        let eraser = [Point { x: 80.0, y: 50.0 }];
        assert_eq!(erase_stroke(&stroke, &eraser, 20.0), vec![stroke.to_vec()]);
    }

    #[test]
    fn disjoint_eraser_preserves_repeated_points_exactly() {
        let a = point(10.0, 20.0);
        let b = point(30.0, 40.0);
        let stroke = [a, a, b, b, b];
        let eraser = [Point {
            x: 1_000.0,
            y: 1_000.0,
        }];
        assert_eq!(erase_stroke(&stroke, &eraser, 20.0), vec![stroke.to_vec()]);
    }

    #[test]
    fn stationary_multi_point_stroke_survives_or_is_erased_as_one_mark() {
        let stroke = [point(50.0, 50.0); 12];
        let distant = [Point { x: 80.0, y: 50.0 }];
        assert_eq!(erase_stroke(&stroke, &distant, 20.0), vec![stroke.to_vec()]);
        let crossing = [Point { x: 55.0, y: 50.0 }];
        assert!(erase_stroke(&stroke, &crossing, 20.0).is_empty());
    }
}
