use crate::theme::Theme;
use fltk::{enums::Color, misc::Progress, prelude::*};

/// Creates a progress bar with the theme styling
pub fn create_progress_bar(x: i32, y: i32, w: i32, h: i32) -> Progress {
    let theme = Theme::global();
    let mut progress = Progress::new(x, y, w, h, "");

    // Set colors
    progress.set_color(Color::from_rgb(30, 30, 30)); // Dark gray background
    progress.set_selection_color(theme.primary); // Cyan progress

    progress
}
