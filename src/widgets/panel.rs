use crate::theme::Theme;
use fltk::{group::Group, prelude::*};

pub fn create_panel(x: i32, y: i32, w: i32, h: i32) -> Group {
    let theme = Theme::global();
    let mut panel = Group::new(x, y, w, h, "");
    panel.set_color(theme.background);
    panel.end();
    panel
}
