use crate::theme::Theme;
use fltk::{button::Button, enums::Color, enums::FrameType, prelude::*};

/// Creates a primary button with the exact design requirements:
/// - Large rounded buttons (30px radius equivalent)
/// - Cyan background
/// - White bold text
/// - No gradients
/// - No shadows
pub fn create_primary_button(x: i32, y: i32, w: i32, h: i32, label: &str) -> Button {
    let theme = Theme::global();
    let mut btn = Button::new(x, y, w, h, label);

    // Set cyan background
    btn.set_color(theme.primary);
    btn.set_selection_color(theme.primary);

    // Set white text
    btn.set_label_color(theme.text);
    btn.set_label_size(theme.font_size + 4); // Larger text for buttons

    // Rounded frame (RoundUpBox gives the rounded appearance)
    btn.set_frame(FrameType::RoundUpBox);

    // No shadows or gradients - FLTK defaults are already flat

    btn
}

/// Creates a secondary button with the same styling but different color
pub fn create_secondary_button(x: i32, y: i32, w: i32, h: i32, label: &str) -> Button {
    let theme = Theme::global();
    let mut btn = Button::new(x, y, w, h, label);

    // Darker cyan for secondary
    btn.set_color(Color::from_rgb(40, 120, 140));
    btn.set_selection_color(Color::from_rgb(40, 120, 140));

    // Set white text
    btn.set_label_color(theme.text);
    btn.set_label_size(theme.font_size + 4);

    // Rounded frame
    btn.set_frame(FrameType::RoundUpBox);

    btn
}
