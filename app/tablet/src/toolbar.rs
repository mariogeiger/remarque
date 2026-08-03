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

#[derive(Clone, Copy)]
struct ToolbarButton {
    x: usize,
    width: usize,
    action: ToolbarAction,
}

const BUTTONS: [ToolbarButton; 10] = [
    ToolbarButton {
        x: LIBRARY_BUTTON_X,
        width: LIBRARY_BUTTON_WIDTH,
        action: ToolbarAction::ShowLibrary,
    },
    ToolbarButton {
        x: THIN_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::Thin),
    },
    ToolbarButton {
        x: MEDIUM_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::Medium),
    },
    ToolbarButton {
        x: THICK_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::Thick),
    },
    ToolbarButton {
        x: EXTRA_THICK_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::ExtraThick),
    },
    ToolbarButton {
        x: BLACK_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Black),
    },
    ToolbarButton {
        x: GRAY_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Gray),
    },
    ToolbarButton {
        x: BLUE_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Blue),
    },
    ToolbarButton {
        x: RED_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Red),
    },
    ToolbarButton {
        x: ADD_PAGE_BUTTON_X,
        width: ADD_PAGE_BUTTON_WIDTH,
        action: ToolbarAction::InsertBlankPage,
    },
];

pub fn toolbar_action_at_x(x: usize) -> ToolbarAction {
    BUTTONS
        .iter()
        .find(|button| x >= button.x && x < button.x + button.width)
        .map_or(ToolbarAction::None, |button| button.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_four_thickness_presets() {
        assert_eq!(
            toolbar_action_at_x(EXTRA_THICK_BUTTON_X),
            ToolbarAction::SelectThickness(FinelinerThickness::ExtraThick)
        );
        assert_eq!(toolbar_action_at_x(560), ToolbarAction::None);
    }

    #[test]
    fn maps_color_swatch_to_stroke_color() {
        assert_eq!(
            toolbar_action_at_x(BLUE_BUTTON_X),
            ToolbarAction::SelectColor(Color::Blue)
        );
    }

    #[test]
    fn maps_document_buttons() {
        assert_eq!(
            toolbar_action_at_x(LIBRARY_BUTTON_X),
            ToolbarAction::ShowLibrary
        );
        assert_eq!(
            toolbar_action_at_x(ADD_PAGE_BUTTON_X),
            ToolbarAction::InsertBlankPage
        );
    }
}
