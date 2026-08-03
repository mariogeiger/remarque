use crate::color::Color;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PenSample {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub two_segment_distance_quarters: u16,
    pub width_quarter_pixels: u16,
    pub direction: u8,
    pub pressure: u8,
}

impl StrokePoint {
    pub const ENCODED_SIZE: usize = 14;

    pub fn encode(self) -> [u8; Self::ENCODED_SIZE] {
        let mut bytes = [0; Self::ENCODED_SIZE];
        bytes[0..4].copy_from_slice(&self.x.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.y.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.two_segment_distance_quarters.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.width_quarter_pixels.to_le_bytes());
        bytes[12] = self.direction;
        bytes[13] = self.pressure;
        bytes
    }

    pub fn decode(bytes: [u8; Self::ENCODED_SIZE]) -> Self {
        Self {
            x: f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            y: f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            two_segment_distance_quarters: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
            width_quarter_pixels: u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
            direction: bytes[12],
            pressure: bytes[13],
        }
    }

    pub fn interpolate(self, other: Self, amount: f32) -> Self {
        let interpolate_u16 = |left: u16, right: u16| {
            (f32::from(left) + (f32::from(right) - f32::from(left)) * amount).round() as u16
        };
        let interpolate_u8 = |left: u8, right: u8| {
            (f32::from(left) + (f32::from(right) - f32::from(left)) * amount).round() as u8
        };
        Self {
            x: self.x + (other.x - self.x) * amount,
            y: self.y + (other.y - self.y) * amount,
            two_segment_distance_quarters: interpolate_u16(
                self.two_segment_distance_quarters,
                other.two_segment_distance_quarters,
            ),
            width_quarter_pixels: interpolate_u16(
                self.width_quarter_pixels,
                other.width_quarter_pixels,
            ),
            direction: interpolate_u8(self.direction, other.direction),
            pressure: interpolate_u8(self.pressure, other.pressure),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Stroke {
    pub points: Vec<StrokePoint>,
    pub color: Color,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_point_round_trips() {
        let point = StrokePoint {
            x: 12.5,
            y: -8.25,
            two_segment_distance_quarters: 73,
            width_quarter_pixels: 16,
            direction: 201,
            pressure: 94,
        };
        assert_eq!(StrokePoint::decode(point.encode()), point);
    }
}
