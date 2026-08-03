use remarque_core::color::Color;
use remarque_core::fineliner::FinelinerThickness;

pub const PANEL_X: usize = 24;
pub const PANEL_Y: usize = 12;
pub const PANEL_HEIGHT: usize = 96;
pub const BUTTON_Y: usize = 24;
pub const BUTTON_HEIGHT: usize = 72;
pub const PEN_BUTTON_X: usize = 40;
pub const ERASER_BUTTON_X: usize = 176;
pub const TOOL_BUTTON_WIDTH: usize = 120;
pub const THIN_BUTTON_X: usize = 352;
pub const MEDIUM_BUTTON_X: usize = 440;
pub const THICK_BUTTON_X: usize = 528;
pub const PRESET_BUTTON_WIDTH: usize = 72;
pub const BLACK_BUTTON_X: usize = 696;
pub const GRAY_BUTTON_X: usize = 768;
pub const BLUE_BUTTON_X: usize = 840;
pub const RED_BUTTON_X: usize = 912;
pub const COLOR_BUTTON_WIDTH: usize = 56;
pub const QUIT_BUTTON_X: usize = 1444;
pub const QUIT_BUTTON_WIDTH: usize = 136;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    SelectFineliner,
    SelectEraser,
    SelectThickness(FinelinerThickness),
    SelectColor(Color),
    ExitApplication,
    None,
}

pub fn map_x_to_action(x: usize) -> ToolbarAction {
    match x {
        PEN_BUTTON_X..=159 => ToolbarAction::SelectFineliner,
        ERASER_BUTTON_X..=295 => ToolbarAction::SelectEraser,
        THIN_BUTTON_X..=423 => ToolbarAction::SelectThickness(FinelinerThickness::Thin),
        MEDIUM_BUTTON_X..=511 => ToolbarAction::SelectThickness(FinelinerThickness::Medium),
        THICK_BUTTON_X..=599 => ToolbarAction::SelectThickness(FinelinerThickness::Thick),
        BLACK_BUTTON_X..=751 => ToolbarAction::SelectColor(Color::Black),
        GRAY_BUTTON_X..=823 => ToolbarAction::SelectColor(Color::Gray),
        BLUE_BUTTON_X..=895 => ToolbarAction::SelectColor(Color::Blue),
        RED_BUTTON_X..=967 => ToolbarAction::SelectColor(Color::Red),
        QUIT_BUTTON_X..=1579 => ToolbarAction::ExitApplication,
        _ => ToolbarAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_quit_button_to_application_exit() {
        assert_eq!(
            map_x_to_action(QUIT_BUTTON_X),
            ToolbarAction::ExitApplication
        );
        assert_eq!(map_x_to_action(1579), ToolbarAction::ExitApplication);
        assert_eq!(map_x_to_action(1443), ToolbarAction::None);
    }

    #[test]
    fn maps_color_swatch_to_stroke_color() {
        assert_eq!(
            map_x_to_action(BLUE_BUTTON_X),
            ToolbarAction::SelectColor(Color::Blue)
        );
    }
}
