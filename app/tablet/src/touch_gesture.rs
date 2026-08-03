use crate::filter_touch_sequences::RejectPalmContactSequences;
use crate::input::TouchFrame;
use crate::touch_tap::{TapSurface, TouchTap};
use crate::view_transform::Point;

pub(crate) enum OneFingerGesture {
    Tap(TapSurface),
    PageSwipe,
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
        previous: [Point; 2],
        current: [Point; 2],
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
    },
}

impl TouchGestureRecognizer {
    pub fn update(
        &mut self,
        frame: &TouchFrame,
        pen_proximity: bool,
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
                let previous = match std::mem::replace(
                    &mut self.active,
                    ActiveGesture::Pinch { previous: current },
                ) {
                    ActiveGesture::Pinch { previous } => Some(previous),
                    _ => None,
                };
                previous.map(|previous| TouchGestureEvent::PinchChanged { previous, current })
            }
            _ => unreachable!("touch filter accepts at most two fingers"),
        }
    }

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
                .update(&frame(&[(10.0, 20.0)]), false, |_| {
                    Some(OneFingerGesture::Tap(TapSurface::Toolbar))
                })
                .is_none()
        );
        let event = gestures.update(&frame(&[]), false, |_| None);
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
                .update(&frame(&[(0.0, 0.0), (10.0, 0.0)]), false, |_| None)
                .is_none()
        );
        let event = gestures.update(&frame(&[(1.0, 0.0), (12.0, 0.0)]), false, |_| None);
        let Some(TouchGestureEvent::PinchChanged { previous, current }) = event else {
            panic!("pinch change was not recognized");
        };
        assert_eq!(
            previous,
            [Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }]
        );
        assert_eq!(
            current,
            [Point { x: 1.0, y: 0.0 }, Point { x: 12.0, y: 0.0 }]
        );
        assert!(gestures.is_pinching());
        assert!(
            gestures
                .update(&frame(&[(1.0, 0.0)]), false, |_| None)
                .is_none()
        );
        assert!(matches!(
            gestures.update(&frame(&[]), false, |_| None),
            Some(TouchGestureEvent::PinchFinished)
        ));
        assert!(!gestures.is_pinching());
        gestures.reset();
    }

    #[test]
    fn palm_rejection_cancels_a_pinch_until_release() {
        let mut gestures = TouchGestureRecognizer::default();
        gestures.update(&frame(&[(0.0, 0.0), (10.0, 0.0)]), false, |_| None);
        let mut palm = frame(&[(0.0, 0.0)]);
        palm.points[0].palm_classified = true;
        assert!(matches!(
            gestures.update(&palm, false, |_| None),
            Some(TouchGestureEvent::PinchFinished)
        ));
        assert!(
            gestures
                .update(&frame(&[(0.0, 0.0)]), false, |_| {
                    Some(OneFingerGesture::Tap(TapSurface::Toolbar))
                })
                .is_none()
        );
        assert!(gestures.update(&frame(&[]), false, |_| None).is_none());
    }

    #[test]
    fn page_swipe_preserves_its_start_and_release_positions() {
        let mut gestures = TouchGestureRecognizer::default();
        gestures.update(&frame(&[(2.0, 3.0)]), false, |_| {
            Some(OneFingerGesture::PageSwipe)
        });
        gestures.update(&frame(&[(20.0, 30.0)]), false, |_| None);
        let event = gestures.update(&frame(&[]), false, |_| None);
        let Some(TouchGestureEvent::PageSwipe { start, end }) = event else {
            panic!("page swipe was not recognized");
        };
        assert_eq!(start, Point { x: 2.0, y: 3.0 });
        assert_eq!(end, Point { x: 20.0, y: 30.0 });
    }
}
