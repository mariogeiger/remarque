use remarque_core::color::Color;
use remarque_core::fineliner::FinelinerThickness;

pub const PANEL_X: usize = 24;
pub const PANEL_Y: usize = 12;
pub const PANEL_HEIGHT: usize = 96;
pub const BUTTON_Y: usize = 24;
pub const BUTTON_HEIGHT: usize = 72;
pub const LIBRARY_BUTTON_X: usize = 40;
pub const LIBRARY_BUTTON_WIDTH: usize = 120;
pub const THIN_BUTTON_X: usize = 224;
pub const MEDIUM_BUTTON_X: usize = 312;
pub const THICK_BUTTON_X: usize = 400;
pub const EXTRA_THICK_BUTTON_X: usize = 488;
pub const PRESET_BUTTON_WIDTH: usize = 72;
pub const BLACK_BUTTON_X: usize = 656;
pub const GRAY_BUTTON_X: usize = 728;
pub const BLUE_BUTTON_X: usize = 800;
pub const RED_BUTTON_X: usize = 872;
pub const COLOR_BUTTON_WIDTH: usize = 56;
pub const PAGE_INDICATOR_X: usize = 1016;
pub const ADD_PAGE_BUTTON_X: usize = 1200;
pub const ADD_PAGE_BUTTON_WIDTH: usize = 144;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    SelectThickness(FinelinerThickness),
    SelectColor(Color),
    ShowLibrary,
    InsertBlankPage,
    None,
}

pub fn map_x_to_action(x: usize) -> ToolbarAction {
    match x {
        LIBRARY_BUTTON_X..=159 => ToolbarAction::ShowLibrary,
        THIN_BUTTON_X..=295 => ToolbarAction::SelectThickness(FinelinerThickness::Thin),
        MEDIUM_BUTTON_X..=383 => ToolbarAction::SelectThickness(FinelinerThickness::Medium),
        THICK_BUTTON_X..=471 => ToolbarAction::SelectThickness(FinelinerThickness::Thick),
        EXTRA_THICK_BUTTON_X..=559 => {
            ToolbarAction::SelectThickness(FinelinerThickness::ExtraThick)
        }
        BLACK_BUTTON_X..=711 => ToolbarAction::SelectColor(Color::Black),
        GRAY_BUTTON_X..=783 => ToolbarAction::SelectColor(Color::Gray),
        BLUE_BUTTON_X..=855 => ToolbarAction::SelectColor(Color::Blue),
        RED_BUTTON_X..=927 => ToolbarAction::SelectColor(Color::Red),
        ADD_PAGE_BUTTON_X..=1343 => ToolbarAction::InsertBlankPage,
        _ => ToolbarAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_four_thickness_presets() {
        assert_eq!(
            map_x_to_action(EXTRA_THICK_BUTTON_X),
            ToolbarAction::SelectThickness(FinelinerThickness::ExtraThick)
        );
        assert_eq!(map_x_to_action(560), ToolbarAction::None);
    }

    #[test]
    fn maps_color_swatch_to_stroke_color() {
        assert_eq!(
            map_x_to_action(BLUE_BUTTON_X),
            ToolbarAction::SelectColor(Color::Blue)
        );
    }

    #[test]
    fn maps_document_buttons() {
        assert_eq!(
            map_x_to_action(LIBRARY_BUTTON_X),
            ToolbarAction::ShowLibrary
        );
        assert_eq!(
            map_x_to_action(ADD_PAGE_BUTTON_X),
            ToolbarAction::InsertBlankPage
        );
    }
}
