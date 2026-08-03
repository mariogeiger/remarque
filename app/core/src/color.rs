#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Color {
    Black,
    Gray,
    White,
    Blue,
    Red,
    Green,
    Cyan,
    Magenta,
    Yellow,
    Orange,
}

impl Color {
    pub const fn scene_id(self) -> u8 {
        match self {
            Self::Black => 0,
            Self::Gray => 1,
            Self::White => 2,
            Self::Blue => 6,
            Self::Red => 7,
            Self::Green => 10,
            Self::Cyan => 11,
            Self::Magenta => 12,
            Self::Yellow => 13,
            Self::Orange => 14,
        }
    }

    pub const fn rgb(self) -> [u8; 3] {
        match self {
            Self::Black => [0x00, 0x00, 0x00],
            Self::Gray => [0x7a, 0x77, 0x76],
            Self::White => [0xff, 0xff, 0xff],
            Self::Blue => [0x30, 0x4a, 0xe0],
            Self::Red => [0xc2, 0x31, 0x32],
            Self::Green => [0x91, 0xda, 0x71],
            Self::Cyan => [0x74, 0xd2, 0xe8],
            Self::Magenta => [0xc0, 0x7f, 0xd2],
            Self::Yellow => [0xfa, 0xe7, 0x19],
            Self::Orange => [0xfe, 0xb2, 0x00],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_matches_the_native_scene_model() {
        assert_eq!(Color::Gray.rgb(), [0x7a, 0x77, 0x76]);
        assert_eq!(Color::Orange.rgb(), [0xfe, 0xb2, 0x00]);
        assert_eq!(Color::Magenta.scene_id(), 12);
    }
}
