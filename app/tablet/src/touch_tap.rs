use crate::view_transform::Point;

const MAXIMUM_TAP_TRAVEL: f64 = 40.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TapSurface {
    DocumentLibrary,
    Toolbar,
}

pub(crate) struct TouchTap {
    surface: TapSurface,
    start: Point,
    current: Point,
}

impl TouchTap {
    pub fn start(surface: TapSurface, position: Point) -> Self {
        Self {
            surface,
            start: position,
            current: position,
        }
    }

    pub fn move_to(&mut self, position: Point) {
        self.current = position;
    }

    pub fn finish(self) -> Option<(TapSurface, Point)> {
        let distance = (self.current.x - self.start.x).hypot(self.current.y - self.start.y);
        (distance <= MAXIMUM_TAP_TRAVEL).then_some((self.surface, self.current))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_stationary_tap_and_rejects_drag() {
        let mut tap = TouchTap::start(TapSurface::Toolbar, Point { x: 10.0, y: 10.0 });
        tap.move_to(Point { x: 34.0, y: 38.0 });
        assert_eq!(tap.finish().unwrap().0, TapSurface::Toolbar);

        let mut drag = TouchTap::start(TapSurface::DocumentLibrary, Point { x: 0.0, y: 0.0 });
        drag.move_to(Point { x: 41.0, y: 0.0 });
        assert!(drag.finish().is_none());
    }
}
