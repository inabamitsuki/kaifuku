use crate::theme::Theme;
use fltk::{frame::Frame, prelude::*};

pub fn create_title(x: i32, y: i32, w: i32, h: i32, text: &str) -> Frame {
    let theme = Theme::global();
    let mut frame = Frame::new(x, y, w, h, text);
    frame.set_label_color(theme.text);
    frame.set_label_size(theme.font_size + 12);
    frame.set_label_font(fltk::enums::Font::HelveticaBold);
    frame
}
