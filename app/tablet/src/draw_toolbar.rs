use crate::bgra_image::BgraImage;
use crate::color::Color;
use crate::draw_text::{draw_text, measure_text_width};
use crate::fineliner::FinelinerThickness;
use crate::toolbar::{self, ToolbarAction, ToolbarActionRegion};

pub(crate) const HEIGHT: usize = 84;
const BACKGROUND: [u8; 3] = [0xff, 0xff, 0xff];
const PANEL: [u8; 3] = [0xf7, 0xf6, 0xf2];
const SHADOW: [u8; 3] = [0xd8, 0xd6, 0xd0];
const SELECTED: [u8; 3] = [0xd8, 0xe8, 0xf6];
const ICON: [u8; 3] = [0x38, 0x38, 0x36];

pub(crate) fn draw_toolbar(
    image: &mut BgraImage,
    thickness: FinelinerThickness,
    color: Color,
    page_number: u32,
    page_count: u32,
) {
    image.fill_rectangle(0, 0, image.width(), HEIGHT, BACKGROUND);
    image.fill_rounded_rectangle(
        toolbar::PANEL_X,
        toolbar::PANEL_Y + 2,
        toolbar::PANEL_WIDTH,
        toolbar::PANEL_HEIGHT,
        20.0,
        SHADOW,
    );
    image.fill_rounded_rectangle(
        toolbar::PANEL_X,
        toolbar::PANEL_Y,
        toolbar::PANEL_WIDTH,
        toolbar::PANEL_HEIGHT,
        20.0,
        PANEL,
    );

    for region in toolbar::ACTION_REGIONS {
        match region.action {
            ToolbarAction::ShowLibrary => draw_library_button(image, region),
            ToolbarAction::SelectThickness(preset) => {
                draw_thickness_button(image, region, preset, preset == thickness)
            }
            ToolbarAction::SelectColor(swatch) => {
                draw_color_button(image, region, swatch, swatch == color)
            }
            ToolbarAction::InsertBlankPage => draw_add_page_button(image, region),
            ToolbarAction::None => {}
        }
    }
    for x in toolbar::SEPARATOR_XS {
        draw_toolbar_separator(image, x);
    }
    draw_page_indicator(image, page_number, page_count);
}

fn draw_library_button(image: &mut BgraImage, region: ToolbarActionRegion) {
    draw_toolbar_button(image, region, false);
    let icon_width = 42;
    let icon_height = 28;
    let x = region.x + (region.width - icon_width) / 2;
    let y = toolbar::BUTTON_Y + (toolbar::BUTTON_HEIGHT - icon_height) / 2 + 3;
    image.fill_rounded_rectangle(x, y, icon_width, icon_height, 6.0, ICON);
    image.fill_rounded_rectangle(x + 4, y + 4, icon_width - 8, icon_height - 8, 3.0, PANEL);
    image.fill_rounded_rectangle(x + 3, y - 6, 20, 11, 4.0, ICON);
}

fn draw_thickness_button(
    image: &mut BgraImage,
    region: ToolbarActionRegion,
    thickness: FinelinerThickness,
    selected: bool,
) {
    draw_toolbar_button(image, region, selected);
    let diameter = match thickness {
        FinelinerThickness::Thin => 8,
        FinelinerThickness::Medium => 14,
        FinelinerThickness::Thick => 20,
        FinelinerThickness::ExtraThick => 28,
    };
    image.fill_rounded_rectangle(
        region.x + (region.width - diameter) / 2,
        toolbar::BUTTON_Y + (toolbar::BUTTON_HEIGHT - diameter) / 2,
        diameter,
        diameter,
        diameter as f32 * 0.5,
        [0x25, 0x25, 0x24],
    );
}

fn draw_color_button(
    image: &mut BgraImage,
    region: ToolbarActionRegion,
    color: Color,
    selected: bool,
) {
    draw_toolbar_button(image, region, selected);
    let diameter = 24;
    image.fill_rounded_rectangle(
        region.x + (region.width - diameter) / 2,
        toolbar::BUTTON_Y + (toolbar::BUTTON_HEIGHT - diameter) / 2,
        diameter,
        diameter,
        diameter as f32 * 0.5,
        color.rgb(),
    );
}

fn draw_add_page_button(image: &mut BgraImage, region: ToolbarActionRegion) {
    draw_toolbar_button(image, region, false);
    let page_x = region.x + 16;
    let page_y = toolbar::BUTTON_Y + 11;
    image.fill_rounded_rectangle(page_x, page_y, 28, 34, 4.0, ICON);
    image.fill_rounded_rectangle(page_x + 3, page_y + 3, 22, 28, 2.0, PANEL);
    image.fill_rounded_rectangle(page_x + 34, page_y + 15, 18, 4, 2.0, ICON);
    image.fill_rounded_rectangle(page_x + 41, page_y + 8, 4, 18, 2.0, ICON);
}

fn draw_toolbar_button(image: &mut BgraImage, region: ToolbarActionRegion, selected: bool) {
    image.fill_rounded_rectangle(
        region.x,
        toolbar::BUTTON_Y,
        region.width,
        toolbar::BUTTON_HEIGHT,
        14.0,
        if selected { SELECTED } else { PANEL },
    );
}

fn draw_toolbar_separator(image: &mut BgraImage, x: usize) {
    image.fill_rounded_rectangle(x, 22, 2, 40, 1.0, [0xd2, 0xd0, 0xca]);
}

fn draw_page_indicator(image: &mut BgraImage, page_number: u32, page_count: u32) {
    let text = format!("{page_number}/{page_count}");
    let text_width = measure_text_width(&text, 20).min(toolbar::PAGE_INDICATOR_WIDTH);
    draw_text(
        image,
        toolbar::PAGE_INDICATOR_X + (toolbar::PAGE_INDICATOR_WIDTH - text_width) / 2,
        55,
        &text,
        20,
        toolbar::PAGE_INDICATOR_WIDTH,
        [0x55, 0x55, 0x52],
    );
}
