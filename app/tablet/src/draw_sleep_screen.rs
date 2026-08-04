use crate::bgra_image::BgraImage;
use crate::draw_text::{draw_text, measure_text_width};

const PAPER: [u8; 3] = [0xf8, 0xf7, 0xf2];
const INK: [u8; 3] = [0x25, 0x25, 0x24];
const MUTED_INK: [u8; 3] = [0x65, 0x65, 0x61];

pub(crate) fn draw_sleep_screen(image: &mut BgraImage) {
    image.fill_rectangle(0, 0, image.width(), image.height(), PAPER);
    let center_x = image.width() / 2;
    draw_centered_text(image, center_x, 900, "Remarque", 64, INK);
    image.fill_rounded_rectangle(center_x - 80, 962, 160, 5, 2.5, MUTED_INK);
    draw_centered_text(image, center_x, 1090, "En veille", 104, INK);
    draw_centered_text(
        image,
        center_x,
        1200,
        "Appuyez sur le bouton pour réveiller la tablette",
        31,
        MUTED_INK,
    );
}

fn draw_centered_text(
    image: &mut BgraImage,
    center_x: usize,
    baseline_y: usize,
    text: &str,
    pixel_size: u16,
    rgb: [u8; 3],
) {
    let width = measure_text_width(text, pixel_size);
    draw_text(
        image,
        center_x.saturating_sub(width / 2),
        baseline_y,
        text,
        pixel_size,
        width,
        rgb,
    );
}
