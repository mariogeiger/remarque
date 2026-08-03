//! Pure scene/view coordinate transformations recovered from native behavior.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub origin: Point,
    pub size: Size,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewTransform {
    pub focal_point: Point,
    pub scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FractionalInterval {
    pub start: f64,
    pub length: f64,
}

impl ViewTransform {
    pub fn view_to_scene(self, view_point: Point, viewport: Size) -> Point {
        Point {
            x: self.focal_point.x + (view_point.x - viewport.width * 0.5) / self.scale,
            y: self.focal_point.y + (view_point.y - viewport.height * 0.5) / self.scale,
        }
    }

    pub fn scene_to_view(self, scene_point: Point, viewport: Size) -> Point {
        Point {
            x: (scene_point.x - self.focal_point.x) * self.scale + viewport.width * 0.5,
            y: (scene_point.y - self.focal_point.y) * self.scale + viewport.height * 0.5,
        }
    }

    pub fn zoom_on_point(
        self,
        view_point: Point,
        factor: f64,
        viewport: Size,
        scene: Bounds,
    ) -> Option<Self> {
        self.scale_and_translate(view_point, view_point, factor, viewport, scene)
    }

    pub fn scale_and_translate(
        self,
        previous_view_point: Point,
        current_view_point: Point,
        factor: f64,
        viewport: Size,
        scene: Bounds,
    ) -> Option<Self> {
        let scale = self.scale * factor;
        if !scale.is_finite() || scale < 1e-6 {
            return None;
        }
        let anchored_scene_point = self.view_to_scene(previous_view_point, viewport);
        let focal_point = Point {
            x: anchored_scene_point.x + (viewport.width * 0.5 - current_view_point.x) / scale,
            y: anchored_scene_point.y + (viewport.height * 0.5 - current_view_point.y) / scale,
        };
        let snapped = Point {
            x: (focal_point.x * scale).trunc() / scale,
            y: (focal_point.y * scale).trunc() / scale,
        };
        Some(Self {
            focal_point: clamp_focal_point(snapped, scale, viewport, scene),
            scale,
        })
    }
}

pub fn two_finger_scale(initial: [Point; 2], current: [Point; 2]) -> Option<f64> {
    let initial_separation = distance(initial[0], initial[1]);
    let current_separation = distance(current[0], current[1]);
    (initial_separation > 0.0).then_some(current_separation / initial_separation)
}

pub fn centroid(points: &[Point]) -> Option<Point> {
    if points.is_empty() {
        return None;
    }
    let sum = points
        .iter()
        .fold(Point { x: 0.0, y: 0.0 }, |sum, point| Point {
            x: sum.x + point.x,
            y: sum.y + point.y,
        });
    let count = points.len() as f64;
    Some(Point {
        x: sum.x / count,
        y: sum.y / count,
    })
}

pub fn viewport_indicator(
    focal_position: f64,
    scale: f64,
    viewport_length: f64,
    scene_origin: f64,
    scene_length: f64,
) -> Option<FractionalInterval> {
    if !scale.is_finite() || scale <= 0.0 || viewport_length <= 0.0 || scene_length <= 0.0 {
        return None;
    }
    let visible_length = (viewport_length / scale).min(scene_length);
    if visible_length >= scene_length {
        return None;
    }
    Some(FractionalInterval {
        start: ((focal_position - visible_length * 0.5 - scene_origin) / scene_length)
            .clamp(0.0, 1.0 - visible_length / scene_length),
        length: visible_length / scene_length,
    })
}

fn clamp_focal_point(focal: Point, scale: f64, viewport: Size, scene: Bounds) -> Point {
    Point {
        x: clamp_axis(
            focal.x,
            viewport.width / scale,
            scene.origin.x,
            scene.size.width,
        ),
        y: clamp_axis(
            focal.y,
            viewport.height / scale,
            scene.origin.y,
            scene.size.height,
        ),
    }
}

fn clamp_axis(focal: f64, visible_length: f64, origin: f64, scene_length: f64) -> f64 {
    if visible_length >= scene_length {
        origin + scene_length * 0.5
    } else {
        let half_visible = visible_length * 0.5;
        focal.clamp(origin + half_visible, origin + scene_length - half_visible)
    }
}

fn distance(left: Point, right: Point) -> f64 {
    (right.x - left.x).hypot(right.y - left.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Size = Size {
        width: 1000.0,
        height: 800.0,
    };
    const SCENE: Bounds = Bounds {
        origin: Point { x: 0.0, y: 0.0 },
        size: Size {
            width: 2000.0,
            height: 1600.0,
        },
    };

    #[test]
    fn separation_ratio_is_the_two_finger_scale() {
        let scale = two_finger_scale(
            [Point { x: 0.0, y: 0.0 }, Point { x: 40.0, y: 0.0 }],
            [Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 0.0 }],
        );
        assert_eq!(scale, Some(2.5));
    }

    #[test]
    fn zoom_keeps_the_anchor_under_the_same_view_pixel() {
        let transform = ViewTransform {
            focal_point: Point {
                x: 1000.0,
                y: 800.0,
            },
            scale: 1.0,
        };
        let anchor = Point { x: 250.0, y: 300.0 };
        let scene_point = transform.view_to_scene(anchor, VIEWPORT);
        let zoomed = transform
            .zoom_on_point(anchor, 2.0, VIEWPORT, SCENE)
            .unwrap();
        let after = zoomed.view_to_scene(anchor, VIEWPORT);
        assert!((scene_point.x - after.x).abs() <= 0.5 / zoomed.scale);
        assert!((scene_point.y - after.y).abs() <= 0.5 / zoomed.scale);
    }

    #[test]
    fn two_finger_translation_maps_the_previous_centroid_to_the_current_centroid() {
        let transform = ViewTransform {
            focal_point: Point {
                x: 1000.0,
                y: 800.0,
            },
            scale: 2.0,
        };
        let previous = Point { x: 400.0, y: 300.0 };
        let current = Point { x: 475.0, y: 350.0 };
        let scene_point = transform.view_to_scene(previous, VIEWPORT);
        let translated = transform
            .scale_and_translate(previous, current, 1.0, VIEWPORT, SCENE)
            .unwrap();
        let mapped = translated.scene_to_view(scene_point, VIEWPORT);
        assert!((mapped.x - current.x).abs() <= 1.0);
        assert!((mapped.y - current.y).abs() <= 1.0);
    }

    #[test]
    fn viewport_indicator_tracks_the_visible_scene_interval() {
        let indicator = viewport_indicator(750.0, 2.0, 1000.0, 0.0, 1000.0).unwrap();
        assert_eq!(indicator.start, 0.5);
        assert_eq!(indicator.length, 0.5);
        assert!(viewport_indicator(500.0, 1.0, 1000.0, 0.0, 1000.0).is_none());
    }

    #[test]
    fn scene_and_view_maps_are_inverse() {
        for scale in [0.25, 0.5, 1.0, 2.0, 8.0] {
            let transform = ViewTransform {
                focal_point: Point {
                    x: 713.25,
                    y: -81.75,
                },
                scale,
            };
            for view_point in [
                Point { x: 0.0, y: 0.0 },
                Point { x: 500.0, y: 400.0 },
                Point {
                    x: VIEWPORT.width,
                    y: VIEWPORT.height,
                },
            ] {
                let round_trip = transform
                    .scene_to_view(transform.view_to_scene(view_point, VIEWPORT), VIEWPORT);
                assert!((round_trip.x - view_point.x).abs() < 1e-9);
                assert!((round_trip.y - view_point.y).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn captured_native_pinch_separations_produce_native_scales() {
        let outward = two_finger_scale(
            [Point { x: 0.0, y: 0.0 }, Point { x: 448.135, y: 0.0 }],
            [
                Point { x: 0.0, y: 0.0 },
                Point {
                    x: 1040.771,
                    y: 0.0,
                },
            ],
        )
        .unwrap();
        let inward = two_finger_scale(
            [
                Point { x: 0.0, y: 0.0 },
                Point {
                    x: 1050.521,
                    y: 0.0,
                },
            ],
            [Point { x: 0.0, y: 0.0 }, Point { x: 229.706, y: 0.0 }],
        )
        .unwrap();
        assert!((outward - 2.32245).abs() < 1e-5);
        assert!((inward - 0.218659).abs() < 1e-6);
    }
}
