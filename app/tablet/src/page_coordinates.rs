use remarque_core::stroke::Stroke;

pub(crate) fn migrate_fit_page_strokes_to_fit_width(
    pages: &mut [Vec<Stroke>],
    page_sizes: &[[f64; 2]],
    screen_width: usize,
    screen_height: usize,
    page_top: usize,
) {
    let available_height = screen_height.saturating_sub(page_top) as f64;
    for (strokes, [page_width, page_height]) in pages.iter_mut().zip(page_sizes) {
        let old_scale = (screen_width as f64 / page_width).min(available_height / page_height);
        let old_width = page_width * old_scale;
        let old_height = page_height * old_scale;
        let old_x = (screen_width as f64 - old_width) * 0.5;
        let old_y = page_top as f64 + (available_height - old_height) * 0.5;
        let fit_width_ratio = screen_width as f64 / old_width;
        for stroke in strokes {
            for point in &mut stroke.points {
                point.x = ((f64::from(point.x) - old_x) * fit_width_ratio) as f32;
                point.y = ((f64::from(point.y) - old_y) * fit_width_ratio) as f32;
                point.two_segment_distance_quarters =
                    scale_quarter_pixels(point.two_segment_distance_quarters, fit_width_ratio);
                point.width_quarter_pixels =
                    scale_quarter_pixels(point.width_quarter_pixels, fit_width_ratio);
            }
        }
    }
}

fn scale_quarter_pixels(value: u16, scale: f64) -> u16 {
    (f64::from(value) * scale)
        .round()
        .clamp(0.0, f64::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use remarque_core::color::Color;
    use remarque_core::stroke::StrokePoint;

    #[test]
    fn centered_fit_page_coordinates_become_page_local_fit_width_coordinates() {
        let mut pages = vec![vec![Stroke {
            points: vec![StrokePoint {
                x: 275.0,
                y: 100.0,
                two_segment_distance_quarters: 9,
                width_quarter_pixels: 9,
                direction: 0,
                pressure: 0,
            }],
            color: Color::Black,
        }]];
        migrate_fit_page_strokes_to_fit_width(&mut pages, &[[500.0, 1000.0]], 1000, 1000, 100);
        let point = pages[0][0].points[0];
        assert_eq!(point.x, 0.0);
        assert_eq!(point.y, 0.0);
        assert_eq!(point.width_quarter_pixels, 20);
        assert_eq!(point.two_segment_distance_quarters, 20);
    }
}
