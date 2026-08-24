use crate::app::Page;
use crate::theme::Theme;
use crate::widgets::{create_secondary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{
    button::Button,
    enums::Color,
    frame::Frame,
    group::Group,
    prelude::*,
    text::{TextBuffer, TextDisplay},
    window::Window,
};
use std::cell::RefCell;
use std::rc::Rc;

pub struct AboutPage;

impl AboutPage {
    pub fn new(win_w: i32, win_h: i32, nav_tx: Sender<Page>) -> Self {
        let theme = Theme::global();
        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 200;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0,
            0,
            title_w,
            title_h,
            "About Kaifuku",
        )));

        let start_y = 80;
        let line_height = 30;

        let lines = [
            ("Kaifuku", theme.font_size + 4),
            ("Version 3.9.3", theme.font_size),
            ("Data Recovery Utility Based on Photorec", theme.font_size),
            ("Copyright © 2024 Kaifuku Team", theme.font_size),
            ("Licensed under GNU General Public License", theme.font_size),
            ("Powered by PhotoRec by Christophe GRENIER", theme.font_size),
            ("https://www.cgsecurity.org/", theme.font_size),
        ];

        let mut line_frames: Vec<Rc<RefCell<Frame>>> = Vec::new();
        for (i, (text, size)) in lines.iter().enumerate() {
            let frame = Rc::new(RefCell::new(Frame::new(
                20,
                start_y + i as i32 * line_height,
                win_w - 40,
                line_height,
                *text,
            )));
            {
                let mut f = frame.borrow_mut();
                f.set_label_color(theme.text);
                f.set_label_size(*size);
            }
            line_frames.push(frame);
        }

        let license_btn = Rc::new(RefCell::new({
            let mut b = create_secondary_button(0, 0, 200, 50, "License Agreement");
            b.set_callback(|_| {
                let theme = Theme::global();
                let mut win = Window::new(300, 200, 600, 420, "License Agreement");
                win.set_color(theme.background);

                let mut buf = TextBuffer::default();
                buf.set_text("!!! test !!!");

                let mut display = TextDisplay::new(10, 10, 580, 350, "");
                display.set_buffer(buf);
                display.set_text_color(theme.text);
                display.set_color(Color::from_rgb(30, 30, 30));

                let mut close_btn = Button::new(250, 370, 100, 35, "Close");
                close_btn.set_color(theme.primary);
                close_btn.set_label_color(theme.text);
                close_btn.set_callback(|btn| {
                    if let Some(mut w) = btn.parent() {
                        w.hide();
                    }
                });

                win.end();
                win.show();
            });
            b
        }));

        let back_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_secondary_button(0, 0, 180, 50, "Back");
            b.set_callback(move |_| {
                let _ = nav.send(Page::Menu);
            });
            b
        }));

        let rel = (title.clone(), license_btn.clone(), back_btn.clone());
        let relayout = move |w: i32, h: i32| {
            rel.0.borrow_mut().set_pos((w - title_w) / 2, 20);
            rel.1.borrow_mut().set_pos(20, h - 80);
            rel.2.borrow_mut().set_pos(w - 200, h - 80);
        };

        relayout(win_w, win_h);
        page.resize_callback(move |_, _, _, w, h| relayout(w, h));

        let spacer = fltk::frame::Frame::new(0, 0, 0, 0, "");
        page.resizable(&spacer);
        page.end();
        page.show();
        Self
    }
}
