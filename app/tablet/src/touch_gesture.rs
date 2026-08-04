use crate::filter_touch_sequences::RejectPalmContactSequences;
use crate::input::TouchFrame;
use crate::touch_tap::{TapSurface, TouchTap};
use crate::view_transform::{Point, midpoint, two_finger_scale, two_finger_separation};

pub(crate) enum OneFingerGesture {
    Tap(TapSurface),
    PageSwipe,
}

#[derive(Clone, Copy)]
pub(crate) enum PinchScaleStart {
    Immediate,
    AfterSeparationIncrease(f64),
}

pub(crate) enum TouchGestureEvent {
    Tap {
        surface: TapSurface,
        position: Point,
    },
    PageSwipe {
        start: Point,
        end: Point,
    },
    PinchChanged {
        previous_centroid: Point,
        current_centroid: Point,
        scale_factor: f64,
    },
    PinchFinished,
}

#[derive(Default)]
pub(crate) struct TouchGestureRecognizer {
    contact_filter: RejectPalmContactSequences,
    active: ActiveGesture,
}

#[derive(Default)]
enum ActiveGesture {
    #[default]
    Idle,
    Tap(TouchTap),
    PageSwipe {
        start: Point,
        current: Point,
    },
    Pinch {
        previous: [Point; 2],
        scale_state: PinchScaleState,
    },
}

enum PinchScaleState {
    Active,
    AwaitingSeparation(f64),
}

impl PinchScaleState {
    fn from_start(start: PinchScaleStart, points: [Point; 2]) -> Self {
        match start {
            PinchScaleStart::Immediate => Self::Active,
            PinchScaleStart::AfterSeparationIncrease(distance)
                if distance.is_finite() && distance > 0.0 =>
            {
                Self::AwaitingSeparation(two_finger_separation(points) + distance)
            }
            PinchScaleStart::AfterSeparationIncrease(_) => Self::Active,
        }
    }

    fn factor(&mut self, previous: [Point; 2], current: [Point; 2]) -> f64 {
        match *self {
            Self::Active => two_finger_scale(previous, current).unwrap_or(1.0),
            Self::AwaitingSeparation(activation_separation) => {
                let current_separation = two_finger_separation(current);
                if current_separation <= activation_separation {
                    1.0
                } else {
                    *self = Self::Active;
                    current_separation / activation_separation
                }
            }
        }
    }
}

impl TouchGestureRecognizer {
    pub fn update(
        &mut self,
        frame: &TouchFrame,
        pen_proximity: bool,
        pinch_scale_start: PinchScaleStart,
        classify_one_finger_start: impl FnOnce(Point) -> Option<OneFingerGesture>,
    ) -> Option<TouchGestureEvent> {
        let Some(points) = self
            .contact_filter
            .accept_at_most_two_finger_points(frame, pen_proximity)
        else {
            let was_pinching = matches!(self.active, ActiveGesture::Pinch { .. });
            self.active = ActiveGesture::Idle;
            return was_pinching.then_some(TouchGestureEvent::PinchFinished);
        };

        match points {
            [] => self.finish(),
            [point] => {
                match &mut self.active {
                    ActiveGesture::Idle => {
                        self.active = match classify_one_finger_start(point.position) {
                            Some(OneFingerGesture::Tap(surface)) => {
                                ActiveGesture::Tap(TouchTap::start(surface, point.position))
                            }
                            Some(OneFingerGesture::PageSwipe) => ActiveGesture::PageSwipe {
                                start: point.position,
                                current: point.position,
                            },
                            None => ActiveGesture::Idle,
                        };
                    }
                    ActiveGesture::Tap(tap) => tap.move_to(point.position),
                    ActiveGesture::PageSwipe { current, .. } => *current = point.position,
                    ActiveGesture::Pinch { .. } => {}
                }
                None
            }
            [first, second] => {
                let current = [first.position, second.position];
                match &mut self.active {
                    ActiveGesture::Pinch {
                        previous,
                        scale_state,
                    } => {
                        let previous_points = *previous;
                        *previous = current;
                        Some(TouchGestureEvent::PinchChanged {
                            previous_centroid: midpoint(previous_points),
                            current_centroid: midpoint(current),
                            scale_factor: scale_state.factor(previous_points, current),
                        })
                    }
                    _ => {
                        self.active = ActiveGesture::Pinch {
                            previous: current,
                            scale_state: PinchScaleState::from_start(pinch_scale_start, current),
                        };
                        None
                    }
                }
            }
            _ => unreachable!("touch filter accepts at most two fingers"),
        }
    }

    #[cfg(test)]
    pub fn is_pinching(&self) -> bool {
        matches!(self.active, ActiveGesture::Pinch { .. })
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn finish(&mut self) -> Option<TouchGestureEvent> {
        match std::mem::take(&mut self.active) {
            ActiveGesture::Tap(tap) => tap
                .finish()
                .map(|(surface, position)| TouchGestureEvent::Tap { surface, position }),
            ActiveGesture::PageSwipe { start, current } => Some(TouchGestureEvent::PageSwipe {
                start,
                end: current,
            }),
            ActiveGesture::Pinch { .. } => Some(TouchGestureEvent::PinchFinished),
            ActiveGesture::Idle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TouchPoint;

    fn frame(points: &[(f64, f64)]) -> TouchFrame {
        TouchFrame {
            points: points
                .iter()
                .map(|&(x, y)| TouchPoint {
                    position: Point { x, y },
                    major_diameter: 8.0,
                    palm_classified: false,
                })
                .collect(),
        }
    }

    #[test]
    fn a_stationary_one_finger_sequence_becomes_one_tap() {
        let mut gestures = TouchGestureRecognizer::default();
        assert!(
            gestures
                .update(
                    &frame(&[(10.0, 20.0)]),
                    false,
                    PinchScaleStart::Immediate,
                    |_| Some(OneFingerGesture::Tap(TapSurface::Toolbar)),
                )
                .is_none()
        );
        let event = gestures.update(&frame(&[]), false, PinchScaleStart::Immediate, |_| None);
        let Some(TouchGestureEvent::Tap { surface, position }) = event else {
            panic!("tap was not recognized");
        };
        assert_eq!(surface, TapSurface::Toolbar);
        assert_eq!(position, Point { x: 10.0, y: 20.0 });
    }

    #[test]
    fn pinch_emits_changes_and_one_finish_after_release() {
        let mut gestures = TouchGestureRecognizer::default();
        assert!(
            gestures
                .update(
                    &frame(&[(0.0, 0.0), (10.0, 0.0)]),
                    false,
                    PinchScaleStart::Immediate,
                    |_| None,
                )
                .is_none()
        );
        let event = gestures.update(
            &frame(&[(1.0, 0.0), (12.0, 0.0)]),
            false,
            PinchScaleStart::Immediate,
            |_| None,
        );
        let Some(TouchGestureEvent::PinchChanged {
            previous_centroid,
            current_centroid,
            scale_factor,
        }) = event
        else {
            panic!("pinch change was not recognized");
        };
        assert_eq!(previous_centroid, Point { x: 5.0, y: 0.0 });
        assert_eq!(current_centroid, Point { x: 6.5, y: 0.0 });
        assert_eq!(scale_factor, 1.1);
        assert!(gestures.is_pinching());
        assert!(
            gestures
                .update(
                    &frame(&[(1.0, 0.0)]),
                    false,
                    PinchScaleStart::Immediate,
                    |_| None,
                )
                .is_none()
        );
        assert!(matches!(
            gestures.update(&frame(&[]), false, PinchScaleStart::Immediate, |_| None,),
            Some(TouchGestureEvent::PinchFinished)
        ));
        assert!(!gestures.is_pinching());
        gestures.reset();
    }

    #[test]
    fn palm_rejection_cancels_a_pinch_until_release() {
        let mut gestures = TouchGestureRecognizer::default();
        gestures.update(
            &frame(&[(0.0, 0.0), (10.0, 0.0)]),
            false,
            PinchScaleStart::Immediate,
            |_| None,
        );
        let mut palm = frame(&[(0.0, 0.0)]);
        palm.points[0].palm_classified = true;
        assert!(matches!(
            gestures.update(&palm, false, PinchScaleStart::Immediate, |_| None),
            Some(TouchGestureEvent::PinchFinished)
        ));
        assert!(
            gestures
                .update(
                    &frame(&[(0.0, 0.0)]),
                    false,
                    PinchScaleStart::Immediate,
                    |_| Some(OneFingerGesture::Tap(TapSurface::Toolbar)),
                )
                .is_none()
        );
        assert!(
            gestures
                .update(&frame(&[]), false, PinchScaleStart::Immediate, |_| None,)
                .is_none()
        );
    }

    #[test]
    fn page_swipe_preserves_its_start_and_release_positions() {
        let mut gestures = TouchGestureRecognizer::default();
        gestures.update(
            &frame(&[(2.0, 3.0)]),
            false,
            PinchScaleStart::Immediate,
            |_| Some(OneFingerGesture::PageSwipe),
        );
        gestures.update(
            &frame(&[(20.0, 30.0)]),
            false,
            PinchScaleStart::Immediate,
            |_| None,
        );
        let event = gestures.update(&frame(&[]), false, PinchScaleStart::Immediate, |_| None);
        let Some(TouchGestureEvent::PageSwipe { start, end }) = event else {
            panic!("page swipe was not recognized");
        };
        assert_eq!(start, Point { x: 2.0, y: 3.0 });
        assert_eq!(end, Point { x: 20.0, y: 30.0 });
    }

    #[test]
    fn separation_barrier_preserves_translation_until_zoom_activation() {
        let mut gestures = TouchGestureRecognizer::default();
        gestures.update(
            &frame(&[(0.0, 0.0), (100.0, 0.0)]),
            false,
            PinchScaleStart::AfterSeparationIncrease(20.0),
            |_| None,
        );

        let below = gestures
            .update(
                &frame(&[(10.0, 0.0), (120.0, 0.0)]),
                false,
                PinchScaleStart::Immediate,
                |_| None,
            )
            .unwrap();
        let TouchGestureEvent::PinchChanged {
            previous_centroid,
            current_centroid,
            scale_factor,
        } = below
        else {
            panic!("pinch change was not recognized");
        };
        assert_eq!(previous_centroid, Point { x: 50.0, y: 0.0 });
        assert_eq!(current_centroid, Point { x: 65.0, y: 0.0 });
        assert_eq!(scale_factor, 1.0);

        let crossing = gestures
            .update(
                &frame(&[(0.0, 0.0), (125.0, 0.0)]),
                false,
                PinchScaleStart::Immediate,
                |_| None,
            )
            .unwrap();
        let TouchGestureEvent::PinchChanged { scale_factor, .. } = crossing else {
            panic!("pinch change was not recognized");
        };
        assert_eq!(scale_factor, 125.0 / 120.0);

        let active = gestures
            .update(
                &frame(&[(0.0, 0.0), (150.0, 0.0)]),
                false,
                PinchScaleStart::AfterSeparationIncrease(1000.0),
                |_| None,
            )
            .unwrap();
        let TouchGestureEvent::PinchChanged { scale_factor, .. } = active else {
            panic!("pinch change was not recognized");
        };
        assert_eq!(scale_factor, 150.0 / 125.0);
    }
}
