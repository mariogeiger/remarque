use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::fineliner::FinelinerThickness;
use crate::notebook::DrawingTool;
use crate::quit_label;
use crate::render_fineliner::{FinelinerRasterPoint, render_fineliner_raster_points};
use crate::toolbar;

pub(crate) const HEIGHT: usize = 112;

pub(crate) fn draw_toolbar(
    image: &mut BgraImage,
    selected_tool: DrawingTool,
    thickness: FinelinerThickness,
    color: Color,
    document_open: bool,
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
    draw_toolbar_button(
        image,
        toolbar::PEN_BUTTON_X,
        toolbar::TOOL_BUTTON_WIDTH,
        selected_tool == DrawingTool::Fineliner,
        SELECTED,
        PANEL,
    );
    draw_toolbar_button(
        image,
        toolbar::ERASER_BUTTON_X,
        toolbar::TOOL_BUTTON_WIDTH,
        selected_tool == DrawingTool::Eraser,
        SELECTED,
        PANEL,
    );
    draw_pen_icon(image, toolbar::PEN_BUTTON_X);
    draw_eraser_icon(image, toolbar::ERASER_BUTTON_X);
    draw_toolbar_separator(image, 320);

    for (x, preset) in [
        (toolbar::THIN_BUTTON_X, FinelinerThickness::Thin),
        (toolbar::MEDIUM_BUTTON_X, FinelinerThickness::Medium),
        (toolbar::THICK_BUTTON_X, FinelinerThickness::Thick),
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
    draw_toolbar_separator(image, 640);
    for (x, swatch) in [
        (toolbar::BLACK_BUTTON_X, Color::Black),
        (toolbar::GRAY_BUTTON_X, Color::Gray),
        (toolbar::BLUE_BUTTON_X, Color::Blue),
        (toolbar::RED_BUTTON_X, Color::Red),
    ] {
        draw_color_swatch(image, x, swatch, swatch == color, SELECTED, PANEL);
    }
    draw_toolbar_separator(image, 1008);
    draw_close_document_button(image, document_open, PANEL);
    image.fill_rounded_rectangle(
        toolbar::QUIT_BUTTON_X,
        toolbar::BUTTON_Y,
        toolbar::QUIT_BUTTON_WIDTH,
        toolbar::BUTTON_HEIGHT,
        18.0,
        [0xf3, 0xdc, 0xda],
    );
    draw_quit_label(image);
}

fn draw_close_document_button(image: &mut BgraImage, document_open: bool, panel_rgb: [u8; 3]) {
    let ink = if document_open {
        [0x35, 0x35, 0x34]
    } else {
        [0xb8, 0xb6, 0xb0]
    };
    draw_toolbar_button(
        image,
        toolbar::CLOSE_BUTTON_X,
        toolbar::CLOSE_BUTTON_WIDTH,
        false,
        panel_rgb,
        panel_rgb,
    );
    let x = toolbar::CLOSE_BUTTON_X;
    image.fill_rounded_rectangle(x + 43, 35, 42, 50, 5.0, ink);
    image.fill_rounded_rectangle(x + 47, 39, 34, 42, 3.0, panel_rgb);
    let point = |x: f32, y: f32| FinelinerRasterPoint { x, y, width: 5.0 };
    render_fineliner_raster_points(
        image,
        &[point(x as f32 + 76.0, 62.0), point(x as f32 + 98.0, 84.0)],
        if document_open {
            Color::Black
        } else {
            Color::Gray
        },
    );
    render_fineliner_raster_points(
        image,
        &[point(x as f32 + 98.0, 62.0), point(x as f32 + 76.0, 84.0)],
        if document_open {
            Color::Black
        } else {
            Color::Gray
        },
    );
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

fn draw_pen_icon(image: &mut BgraImage, button_x: usize) {
    let point = |x: f32, y: f32| FinelinerRasterPoint { x, y, width: 6.0 };
    render_fineliner_raster_points(
        image,
        &[
            point(button_x as f32 + 38.0, 74.0),
            point(button_x as f32 + 82.0, 46.0),
        ],
        Color::Black,
    );
}

fn draw_eraser_icon(image: &mut BgraImage, button_x: usize) {
    let point = |x: f32, y: f32| FinelinerRasterPoint { x, y, width: 24.0 };
    render_fineliner_raster_points(
        image,
        &[
            point(button_x as f32 + 42.0, 72.0),
            point(button_x as f32 + 78.0, 48.0),
        ],
        Color::Gray,
    );
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

fn draw_quit_label(image: &mut BgraImage) {
    const TEXT_RGB: [u8; 3] = [0x18, 0x18, 0x18];
    const RASTER_TEXT_DARKNESS: u16 = 255 - 0x18;
    let mut pixel_index = 0;
    for &(run_length, raster_darkness) in quit_label::ALPHA_RUNS {
        let coverage = ((u16::from(raster_darkness) * 255 + RASTER_TEXT_DARKNESS / 2)
            / RASTER_TEXT_DARKNESS)
            .min(255) as u8;
        for _ in 0..run_length {
            if coverage != 0 {
                image.blend_rgb_coverage(
                    toolbar::QUIT_BUTTON_X + pixel_index % quit_label::WIDTH,
                    toolbar::BUTTON_Y + pixel_index / quit_label::WIDTH,
                    TEXT_RGB,
                    coverage,
                );
            }
            pixel_index += 1;
        }
    }
    debug_assert_eq!(pixel_index, quit_label::WIDTH * quit_label::HEIGHT);
}
