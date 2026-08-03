use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::draw_text::draw_text;
use crate::fineliner::FinelinerThickness;
use crate::toolbar;

pub(crate) const HEIGHT: usize = 112;

pub(crate) fn draw_toolbar(
    image: &mut BgraImage,
    thickness: FinelinerThickness,
    color: Color,
    page_number: u32,
    page_count: u32,
) {
    const BACKGROUND: [u8; 3] = [0xff, 0xff, 0xff];
    const PANEL: [u8; 3] = [0xf7, 0xf6, 0xf2];
    const SHADOW: [u8; 3] = [0xd8, 0xd6, 0xd0];
    const SELECTED: [u8; 3] = [0xd8, 0xe8, 0xf6];
    image.fill_rectangle(0, 0, image.width(), HEIGHT, BACKGROUND);
    let panel_width = image.width().saturating_sub(toolbar::PANEL_X * 2);
    image.fill_rounded_rectangle(
        toolbar::PANEL_X,
        toolbar::PANEL_Y + 3,
        panel_width,
        toolbar::PANEL_HEIGHT,
        24.0,
        SHADOW,
    );
    image.fill_rounded_rectangle(
        toolbar::PANEL_X,
        toolbar::PANEL_Y,
        panel_width,
        toolbar::PANEL_HEIGHT,
        24.0,
        PANEL,
    );
    draw_library_button(image, PANEL);
    draw_toolbar_separator(image, 184);

    for (x, preset) in [
        (toolbar::THIN_BUTTON_X, FinelinerThickness::Thin),
        (toolbar::MEDIUM_BUTTON_X, FinelinerThickness::Medium),
        (toolbar::THICK_BUTTON_X, FinelinerThickness::Thick),
        (
            toolbar::EXTRA_THICK_BUTTON_X,
            FinelinerThickness::ExtraThick,
        ),
    ] {
        draw_toolbar_button(
            image,
            x,
            toolbar::PRESET_BUTTON_WIDTH,
            preset == thickness,
            SELECTED,
            PANEL,
        );
        let diameter = match preset {
            FinelinerThickness::Thin => 10,
            FinelinerThickness::Medium => 18,
            FinelinerThickness::Thick => 26,
            FinelinerThickness::ExtraThick => 34,
        };
        image.fill_rounded_rectangle(
            x + (toolbar::PRESET_BUTTON_WIDTH - diameter) / 2,
            60 - diameter / 2,
            diameter,
            diameter,
            diameter as f32 * 0.5,
            [0x25, 0x25, 0x24],
        );
    }
    draw_toolbar_separator(image, 592);
    for (x, swatch) in [
        (toolbar::BLACK_BUTTON_X, Color::Black),
        (toolbar::GRAY_BUTTON_X, Color::Gray),
        (toolbar::BLUE_BUTTON_X, Color::Blue),
        (toolbar::RED_BUTTON_X, Color::Red),
    ] {
        draw_color_swatch(image, x, swatch, swatch == color, SELECTED, PANEL);
    }
    draw_toolbar_separator(image, 960);
    draw_text(
        image,
        toolbar::PAGE_INDICATOR_X,
        72,
        &format!("{page_number}/{page_count}"),
        24,
        78,
        [0x55, 0x55, 0x52],
    );
    draw_add_page_button(image, PANEL);
}

fn draw_library_button(image: &mut BgraImage, panel_rgb: [u8; 3]) {
    draw_toolbar_button(
        image,
        toolbar::LIBRARY_BUTTON_X,
        toolbar::LIBRARY_BUTTON_WIDTH,
        false,
        panel_rgb,
        panel_rgb,
    );
    let x = toolbar::LIBRARY_BUTTON_X;
    image.fill_rounded_rectangle(x + 26, 47, 68, 38, 7.0, [0x38, 0x38, 0x36]);
    image.fill_rounded_rectangle(x + 31, 52, 58, 28, 4.0, panel_rgb);
    image.fill_rounded_rectangle(x + 28, 39, 32, 15, 5.0, [0x38, 0x38, 0x36]);
}

fn draw_add_page_button(image: &mut BgraImage, panel_rgb: [u8; 3]) {
    draw_toolbar_button(
        image,
        toolbar::ADD_PAGE_BUTTON_X,
        toolbar::ADD_PAGE_BUTTON_WIDTH,
        false,
        panel_rgb,
        panel_rgb,
    );
    let x = toolbar::ADD_PAGE_BUTTON_X;
    image.fill_rounded_rectangle(x + 34, 37, 44, 48, 5.0, [0x38, 0x38, 0x36]);
    image.fill_rounded_rectangle(x + 38, 41, 36, 40, 3.0, panel_rgb);
    image.fill_rounded_rectangle(x + 78, 58, 28, 6, 3.0, [0x38, 0x38, 0x36]);
    image.fill_rounded_rectangle(x + 89, 47, 6, 28, 3.0, [0x38, 0x38, 0x36]);
}

fn draw_toolbar_button(
    image: &mut BgraImage,
    x: usize,
    width: usize,
    selected: bool,
    selected_rgb: [u8; 3],
    panel_rgb: [u8; 3],
) {
    image.fill_rounded_rectangle(
        x,
        toolbar::BUTTON_Y,
        width,
        toolbar::BUTTON_HEIGHT,
        18.0,
        if selected { selected_rgb } else { panel_rgb },
    );
}

fn draw_toolbar_separator(image: &mut BgraImage, x: usize) {
    image.fill_rounded_rectangle(x, 36, 2, 48, 1.0, [0xd2, 0xd0, 0xca]);
}

fn draw_color_swatch(
    image: &mut BgraImage,
    x: usize,
    color: Color,
    selected: bool,
    selected_rgb: [u8; 3],
    panel_rgb: [u8; 3],
) {
    draw_toolbar_button(
        image,
        x,
        toolbar::COLOR_BUTTON_WIDTH,
        selected,
        selected_rgb,
        panel_rgb,
    );
    image.fill_rounded_rectangle(x + 14, 46, 28, 28, 14.0, color.rgb());
}
