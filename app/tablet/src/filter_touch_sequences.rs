use crate::input::TouchFrame;
use crate::view_transform::Point;

#[derive(Default)]
pub struct RejectPalmContactSequences {
    rejected_until_release: bool,
}

impl RejectPalmContactSequences {
    pub fn accept_two_finger_positions(
        &mut self,
        frame: &TouchFrame,
        pen_proximity: bool,
    ) -> Option<[Point; 2]> {
        if frame.points.is_empty() {
            self.rejected_until_release = false;
            return None;
        }
        if pen_proximity
            || frame.points.len() > 2
            || frame.points.iter().any(|point| point.is_palm())
        {
            self.rejected_until_release = true;
        }
        if self.rejected_until_release || frame.points.len() != 2 {
            return None;
        }
        Some([frame.points[0].position, frame.points[1].position])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TouchPoint;

    fn frame(major_diameters: &[f64]) -> TouchFrame {
        TouchFrame {
            points: major_diameters
                .iter()
                .enumerate()
                .map(|(index, &major_diameter)| TouchPoint {
                    position: Point {
                        x: index as f64,
                        y: index as f64,
                    },
                    major_diameter,
                    palm_classified: false,
                })
                .collect(),
        }
    }

    #[test]
    fn accepts_exactly_two_finger_contacts() {
        let mut rejection = RejectPalmContactSequences::default();
        assert!(
            rejection
                .accept_two_finger_positions(&frame(&[8.0, 11.0]), false)
                .is_some()
        );
    }

    #[test]
    fn rejects_a_sequence_until_every_contact_is_released() {
        let mut rejection = RejectPalmContactSequences::default();
        assert!(
            rejection
                .accept_two_finger_positions(&frame(&[8.0, 48.0]), false)
                .is_none()
        );
        assert!(
            rejection
                .accept_two_finger_positions(&frame(&[8.0, 9.0]), false)
                .is_none()
        );
        rejection.accept_two_finger_positions(&frame(&[]), false);
        assert!(
            rejection
                .accept_two_finger_positions(&frame(&[8.0, 9.0]), false)
                .is_some()
        );
    }

    #[test]
    fn rejects_pen_proximity_and_more_than_two_contacts() {
        let mut pen_rejection = RejectPalmContactSequences::default();
        assert!(
            pen_rejection
                .accept_two_finger_positions(&frame(&[8.0, 9.0]), true)
                .is_none()
        );

        let mut contact_count_rejection = RejectPalmContactSequences::default();
        assert!(
            contact_count_rejection
                .accept_two_finger_positions(&frame(&[8.0, 9.0, 10.0]), false)
                .is_none()
        );
    }
}
