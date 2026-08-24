use crate::app::Page;
use crate::pages::advanced::header_template;
use crate::theme;
use crate::widgets::{create_primary_button, create_secondary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{frame::Frame, group::Group, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub struct ExperimentalPage;

impl ExperimentalPage {
    pub fn new(win_w: i32, win_h: i32, nav_tx: Sender<Page>) -> Self {
        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 250;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0,
            0,
            title_w,
            title_h,
            "Experimental Features",
        )));
        let subtitle = Rc::new(RefCell::new(Frame::new(
            0,
            0,
            win_w - 40,
            25,
            "Experimental tools for advanced recovery",
        )));
        {
            let mut s = subtitle.borrow_mut();
            s.set_label_color(theme::muted_color());
            s.set_label_size(14);
        }

        let btn_w = 300;
        let btn_h = 50;
        let spacing = 20;

        let deep_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_primary_button(0, 0, btn_w, btn_h, "Deep Repair Tools");
            b.set_callback(move |_| {
                let _ = nav.send(Page::DeepRepair);
            });
            b
        }));

        let header_btn = Rc::new(RefCell::new({
            let mut b = create_primary_button(0, 0, btn_w, btn_h, "Header Template Generator");
            b.set_callback(move |_| header_template());
            b
        }));

        let back_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_secondary_button(0, 0, 180, 50, "Back");
            b.set_callback(move |_| {
                let _ = nav.send(Page::Advanced);
            });
            b
        }));

        let start_y = 110;
        let rel = (
            title.clone(),
            subtitle.clone(),
            deep_btn.clone(),
            header_btn.clone(),
            back_btn.clone(),
        );
        let relayout = move |w: i32, h: i32| {
            rel.0.borrow_mut().set_pos((w - title_w) / 2, 20);
            rel.1.borrow_mut().resize(20, 55, w - 40, 25);
            let cx = (w - btn_w) / 2;
            rel.2.borrow_mut().set_pos(cx, start_y);
            rel.3.borrow_mut().set_pos(cx, start_y + (btn_h + spacing));
            rel.4.borrow_mut().set_pos(20, h - 80);
        };

        relayout(win_w, win_h);
        page.resize_callback(move |_, _, _, w, h| relayout(w, h));

        let spacer = Frame::new(0, 0, 0, 0, "");
        page.resizable(&spacer);
        page.end();
        page.show();
        Self
    }
}
