use remarque_core::color::Color;
use remarque_core::fineliner::FinelinerThickness;

pub const PANEL_X: usize = 16;
pub const PANEL_Y: usize = 8;
pub const PANEL_WIDTH: usize = 732;
pub const PANEL_HEIGHT: usize = 68;
pub const BUTTON_Y: usize = 14;
pub const BUTTON_HEIGHT: usize = 56;
pub const LIBRARY_BUTTON_X: usize = 24;
pub const LIBRARY_BUTTON_WIDTH: usize = 72;
pub const THIN_BUTTON_X: usize = 116;
pub const MEDIUM_BUTTON_X: usize = 168;
pub const THICK_BUTTON_X: usize = 220;
pub const EXTRA_THICK_BUTTON_X: usize = 272;
pub const PRESET_BUTTON_WIDTH: usize = 52;
pub const BLACK_BUTTON_X: usize = 344;
pub const GRAY_BUTTON_X: usize = 388;
pub const BLUE_BUTTON_X: usize = 432;
pub const RED_BUTTON_X: usize = 476;
pub const YELLOW_BUTTON_X: usize = 520;
pub const COLOR_BUTTON_WIDTH: usize = 44;
pub const PAGE_INDICATOR_X: usize = 586;
pub const PAGE_INDICATOR_WIDTH: usize = 70;
pub const ADD_PAGE_BUTTON_X: usize = 668;
pub const ADD_PAGE_BUTTON_WIDTH: usize = 72;
#[cfg(feature = "takeover")]
pub(crate) const SEPARATOR_XS: [usize; 3] = [106, 334, 574];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    SelectThickness(FinelinerThickness),
    SelectColor(Color),
    ShowLibrary,
    InsertBlankPage,
    None,
}

#[derive(Clone, Copy)]
pub(crate) struct ToolbarActionRegion {
    pub x: usize,
    pub width: usize,
    pub action: ToolbarAction,
}

pub(crate) const ACTION_REGIONS: [ToolbarActionRegion; 11] = [
    ToolbarActionRegion {
        x: LIBRARY_BUTTON_X,
        width: LIBRARY_BUTTON_WIDTH,
        action: ToolbarAction::ShowLibrary,
    },
    ToolbarActionRegion {
        x: THIN_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::Thin),
    },
    ToolbarActionRegion {
        x: MEDIUM_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::Medium),
    },
    ToolbarActionRegion {
        x: THICK_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::Thick),
    },
    ToolbarActionRegion {
        x: EXTRA_THICK_BUTTON_X,
        width: PRESET_BUTTON_WIDTH,
        action: ToolbarAction::SelectThickness(FinelinerThickness::ExtraThick),
    },
    ToolbarActionRegion {
        x: BLACK_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Black),
    },
    ToolbarActionRegion {
        x: GRAY_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Gray),
    },
    ToolbarActionRegion {
        x: BLUE_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Blue),
    },
    ToolbarActionRegion {
        x: RED_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Red),
    },
    ToolbarActionRegion {
        x: YELLOW_BUTTON_X,
        width: COLOR_BUTTON_WIDTH,
        action: ToolbarAction::SelectColor(Color::Yellow),
    },
    ToolbarActionRegion {
        x: ADD_PAGE_BUTTON_X,
        width: ADD_PAGE_BUTTON_WIDTH,
        action: ToolbarAction::InsertBlankPage,
    },
];

pub fn toolbar_action_at_x(x: usize) -> ToolbarAction {
    ACTION_REGIONS
        .iter()
        .find(|region| x >= region.x && x < region.x + region.width)
        .map_or(ToolbarAction::None, |region| region.action)
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
        assert_eq!(toolbar_action_at_x(324), ToolbarAction::None);
    }

    #[test]
    fn maps_five_color_swatches() {
        assert_eq!(
            toolbar_action_at_x(BLUE_BUTTON_X),
            ToolbarAction::SelectColor(Color::Blue)
        );
        assert_eq!(
            toolbar_action_at_x(YELLOW_BUTTON_X),
            ToolbarAction::SelectColor(Color::Yellow)
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

    #[test]
    fn every_action_region_fits_inside_the_compact_panel() {
        assert!(
            ACTION_REGIONS
                .windows(2)
                .all(|pair| pair[0].x + pair[0].width <= pair[1].x)
        );
        let last = ACTION_REGIONS.last().unwrap();
        assert!(last.x + last.width <= PANEL_X + PANEL_WIDTH);
        assert_eq!(
            toolbar_action_at_x(PANEL_X + PANEL_WIDTH),
            ToolbarAction::None
        );
    }
}
