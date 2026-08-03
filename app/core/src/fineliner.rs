use crate::stroke::{PenSample, StrokePoint};
use std::f32::consts::TAU;

const TWO_SEGMENT_DISTANCE_SCALE: f32 = 2.5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinelinerThickness {
    Thin,
    Medium,
    Thick,
}

impl FinelinerThickness {
    pub const fn pixels(self) -> f32 {
        match self {
            Self::Thin => 2.0,
            Self::Medium => 4.0,
            Self::Thick => 6.0,
        }
    }

    pub const fn quarter_pixels(self) -> u16 {
        (self.pixels() * 4.0) as u16
    }
}

#[derive(Debug)]
pub struct FinelinerStrokeBuilder {
    thickness: FinelinerThickness,
    samples: Vec<PenSample>,
    points: Vec<StrokePoint>,
}

impl FinelinerStrokeBuilder {
    pub fn new(thickness: FinelinerThickness) -> Self {
        Self {
            thickness,
            samples: Vec::new(),
            points: Vec::new(),
        }
    }

    pub fn append_sample(&mut self, sample: PenSample, view_scale: f32) -> StrokePoint {
        let previous = self.samples.last().copied().unwrap_or(sample);
        let preceding = self
            .samples
            .get(self.samples.len().saturating_sub(2))
            .copied()
            .unwrap_or(previous);
        let current_heading = heading(previous, sample);
        let previous_heading = heading(preceding, previous);
        let mean_pressure = ((previous.pressure + sample.pressure) * 0.5).clamp(0.0, 1.0);
        let current_distance = distance(previous, sample);
        let previous_distance = distance(preceding, previous);
        let point = StrokePoint {
            x: sample.x,
            y: sample.y,
            two_segment_distance_quarters: ((current_distance + previous_distance)
                * view_scale
                * (2.0 * TWO_SEGMENT_DISTANCE_SCALE))
                .round() as u16,
            width_quarter_pixels: self.thickness.quarter_pixels(),
            direction: (((previous_heading + current_heading) * 0.5 * 255.0) / TAU).round() as u8,
            pressure: (mean_pressure * 255.0).round() as u8,
        };
        self.samples.push(sample);
        self.points.push(point);
        point
    }

    pub fn points(&self) -> &[StrokePoint] {
        &self.points
    }

    pub fn finish(self) -> Vec<StrokePoint> {
        self.points
    }
}

fn distance(from: PenSample, to: PenSample) -> f32 {
    (to.x - from.x).hypot(to.y - from.y)
}

fn heading(from: PenSample, to: PenSample) -> f32 {
    let heading = (to.y - from.y).atan2(to.x - from.x);
    if heading < 0.0 {
        heading + TAU
    } else {
        heading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproduces_recovered_width_presets() {
        assert_eq!(FinelinerThickness::Thin.quarter_pixels(), 8);
        assert_eq!(FinelinerThickness::Medium.quarter_pixels(), 16);
        assert_eq!(FinelinerThickness::Thick.quarter_pixels(), 24);
    }

    #[test]
    fn pressure_is_stored_without_changing_width() {
        let mut stroke = FinelinerStrokeBuilder::new(FinelinerThickness::Medium);
        let first = stroke.append_sample(
            PenSample {
                x: 0.0,
                y: 0.0,
                pressure: 0.2,
            },
            1.0,
        );
        let second = stroke.append_sample(
            PenSample {
                x: 1.0,
                y: 0.0,
                pressure: 0.8,
            },
            1.0,
        );
        assert_eq!(first.width_quarter_pixels, second.width_quarter_pixels);
        assert_eq!(second.pressure, 128);
    }

    #[test]
    fn stores_two_segment_distance_in_quarter_view_pixels() {
        let mut stroke = FinelinerStrokeBuilder::new(FinelinerThickness::Thin);
        let first = stroke.append_sample(
            PenSample {
                x: 0.0,
                y: 0.0,
                pressure: 0.5,
            },
            2.0,
        );
        let second = stroke.append_sample(
            PenSample {
                x: 3.0,
                y: 4.0,
                pressure: 0.5,
            },
            2.0,
        );
        let third = stroke.append_sample(
            PenSample {
                x: 6.0,
                y: 8.0,
                pressure: 0.5,
            },
            2.0,
        );
        assert_eq!(first.two_segment_distance_quarters, 0);
        assert_eq!(second.two_segment_distance_quarters, 50);
        assert_eq!(third.two_segment_distance_quarters, 100);
    }
}
