use crate::bgra_image::BgraImage;
use fontdue::{Font, FontSettings};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const FONT_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ui-font.ttf"));

struct RasterGlyph {
    width: usize,
    height: usize,
    xmin: isize,
    ymin: isize,
    advance: f32,
    coverage: Vec<u8>,
}

pub(crate) fn draw_text(
    image: &mut BgraImage,
    x: usize,
    baseline_y: usize,
    text: &str,
    pixel_size: u16,
    max_width: usize,
    rgb: [u8; 3],
) -> usize {
    cache_glyphs(text, pixel_size);
    let glyphs = glyphs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut pen_x = x as f32;
    let maximum_x = x.saturating_add(max_width) as f32;
    let mut previous = None;
    for character in text.chars() {
        if let Some(previous) = previous {
            pen_x += font()
                .horizontal_kern(previous, character, pixel_size as f32)
                .unwrap_or(0.0);
        }
        let glyph = glyphs
            .get(&(character, pixel_size))
            .expect("glyph was cached");
        if pen_x + glyph.advance > maximum_x {
            break;
        }
        let glyph_x = pen_x.round() as isize + glyph.xmin;
        let glyph_y = baseline_y as isize - glyph.height as isize - glyph.ymin;
        for row in 0..glyph.height {
            let y = glyph_y + row as isize;
            if y < 0 || y >= image.height() as isize {
                continue;
            }
            for column in 0..glyph.width {
                let x = glyph_x + column as isize;
                let coverage = glyph.coverage[row * glyph.width + column];
                if coverage != 0 && x >= 0 && x < image.width() as isize {
                    image.blend_rgb_coverage(x as usize, y as usize, rgb, coverage);
                }
            }
        }
        pen_x += glyph.advance;
        previous = Some(character);
    }
    pen_x.round() as usize
}

fn cache_glyphs(text: &str, pixel_size: u16) {
    let mut glyphs = glyphs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for character in text.chars() {
        glyphs.entry((character, pixel_size)).or_insert_with(|| {
            let (metrics, coverage) = font().rasterize(character, pixel_size as f32);
            RasterGlyph {
                width: metrics.width,
                height: metrics.height,
                xmin: metrics.xmin as isize,
                ymin: metrics.ymin as isize,
                advance: metrics.advance_width,
                coverage,
            }
        });
    }
}

fn font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("embedded UI font is valid")
    })
}

fn glyphs() -> &'static Mutex<HashMap<(char, u16), RasterGlyph>> {
    static GLYPHS: OnceLock<Mutex<HashMap<(char, u16), RasterGlyph>>> = OnceLock::new();
    GLYPHS.get_or_init(|| Mutex::new(HashMap::new()))
}
