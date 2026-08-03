use remarque_core::view_transform::Point;

const EDGE_WIDTH: f64 = 140.0;
const MINIMUM_HORIZONTAL_TRAVEL: f64 = 120.0;
const HORIZONTAL_DOMINANCE: f64 = 1.5;

pub(crate) fn page_delta_from_edge_swipe(
    start: Point,
    end: Point,
    screen_width: f64,
) -> Option<i32> {
    let horizontal = end.x - start.x;
    let vertical = end.y - start.y;
    if horizontal.abs() < MINIMUM_HORIZONTAL_TRAVEL
        || horizontal.abs() < vertical.abs() * HORIZONTAL_DOMINANCE
    {
        return None;
    }
    if start.x <= EDGE_WIDTH && horizontal > 0.0 {
        Some(-1)
    } else if start.x >= screen_width - EDGE_WIDTH && horizontal < 0.0 {
        Some(1)
    } else {
        None
    }
}

pub(crate) fn starts_at_page_edge(point: Point, screen_width: f64) -> bool {
    point.x <= EDGE_WIDTH || point.x >= screen_width - EDGE_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inward_swipes_change_page_in_the_expected_direction() {
        assert_eq!(
            page_delta_from_edge_swipe(
                Point { x: 20.0, y: 500.0 },
                Point { x: 220.0, y: 510.0 },
                1620.0,
            ),
            Some(-1)
        );
        assert_eq!(
            page_delta_from_edge_swipe(
                Point {
                    x: 1600.0,
                    y: 500.0
                },
                Point {
                    x: 1400.0,
                    y: 490.0
                },
                1620.0,
            ),
            Some(1)
        );
    }

    #[test]
    fn center_outward_short_and_vertical_swipes_do_nothing() {
        let width = 1620.0;
        assert!(starts_at_page_edge(Point { x: 20.0, y: 500.0 }, width));
        assert!(!starts_at_page_edge(Point { x: 800.0, y: 500.0 }, width));
        assert_eq!(
            page_delta_from_edge_swipe(
                Point { x: 800.0, y: 500.0 },
                Point { x: 500.0, y: 500.0 },
                width,
            ),
            None
        );
        assert_eq!(
            page_delta_from_edge_swipe(
                Point { x: 20.0, y: 500.0 },
                Point { x: 100.0, y: 500.0 },
                width,
            ),
            None
        );
        assert_eq!(
            page_delta_from_edge_swipe(
                Point { x: 20.0, y: 500.0 },
                Point { x: 180.0, y: 800.0 },
                width,
            ),
            None
        );
    }
}
