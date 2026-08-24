use crate::app::Page;
use crate::theme::Theme;
use crate::widgets::{create_primary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{enums::Align, group::Group, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub struct MenuPage;

impl MenuPage {
    pub fn new(win_w: i32, win_h: i32, nav_tx: Sender<Page>) -> Self {
        let theme = Theme::global();

        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 200;
        let title_h = 40;
        let version_w = 200;
        let version_h = 30;
        let btn_w = 200;
        let btn_h = 60;
        let spacing = 20;

        let title = Rc::new(RefCell::new(create_title(
            0, 0, title_w, title_h, "Kaifuku",
        )));
        {
            let mut t = title.borrow_mut();
            t.set_align(Align::Center);
        }

        let version_text = format!("Version {}", env!("CARGO_PKG_VERSION"));
        let version = Rc::new(RefCell::new(fltk::frame::Frame::new(
            0,
            0,
            version_w,
            version_h,
            version_text.as_str(),
        )));
        {
            let mut v = version.borrow_mut();
            v.set_label_color(theme.text);
            v.set_label_size(theme.font_size);
            v.set_align(Align::Center);
        }

        let scan_btn = Rc::new(RefCell::new(create_primary_button(
            0,
            0,
            btn_w,
            btn_h,
            "Scan & Recover",
        )));
        let advanced_btn = Rc::new(RefCell::new(create_primary_button(
            0,
            0,
            btn_w,
            btn_h,
            "Advanced Mode",
        )));
        let repair_btn = Rc::new(RefCell::new(create_primary_button(
            0,
            0,
            btn_w,
            btn_h,
            "File Repair",
        )));
        let settings_btn = Rc::new(RefCell::new(create_primary_button(
            0, 0, btn_w, btn_h, "Settings",
        )));
        let about_btn = Rc::new(RefCell::new(create_primary_button(
            0, 0, btn_w, btn_h, "About",
        )));
        let exit_btn = Rc::new(RefCell::new(create_primary_button(
            0, 0, btn_w, btn_h, "Exit",
        )));

        {
            let nav = nav_tx.clone();
            scan_btn.borrow_mut().set_callback(move |_| {
                let _ = nav.send(Page::Scan);
            });
        }
        {
            let nav = nav_tx.clone();
            advanced_btn.borrow_mut().set_callback(move |_| {
                let _ = nav.send(Page::Advanced);
            });
        }
        {
            let nav = nav_tx.clone();
            repair_btn.borrow_mut().set_callback(move |_| {
                let _ = nav.send(Page::Repair);
            });
        }
        {
            let nav = nav_tx.clone();
            settings_btn.borrow_mut().set_callback(move |_| {
                let _ = nav.send(Page::Settings);
            });
        }
        {
            let nav = nav_tx.clone();
            about_btn.borrow_mut().set_callback(move |_| {
                let _ = nav.send(Page::About);
            });
        }
        exit_btn.borrow_mut().set_callback(|_| {
            std::process::exit(0);
        });

        let layout_w = title.clone();
        let layout_v = version.clone();
        let layout_scan = scan_btn.clone();
        let layout_adv = advanced_btn.clone();
        let layout_rep = repair_btn.clone();
        let layout_set = settings_btn.clone();
        let layout_abt = about_btn.clone();
        let layout_exit = exit_btn.clone();

        let relayout = move |w: i32, h: i32| {
            let block_h = 40 + 10 + 30 + 20 + btn_h * 3 + spacing * 2;
            let top = ((h - block_h) / 2).max(10);
            layout_w.borrow_mut().set_pos((w - title_w) / 2, top);
            layout_v.borrow_mut().set_pos((w - version_w) / 2, top + 50);
            let start_x = (w - (btn_w * 2 + spacing)) / 2;
            let start_y = top + 100;
            layout_scan.borrow_mut().set_pos(start_x, start_y);
            layout_adv
                .borrow_mut()
                .set_pos(start_x, start_y + (btn_h + spacing));
            layout_rep
                .borrow_mut()
                .set_pos(start_x, start_y + (btn_h + spacing) * 2);
            let right_x = start_x + btn_w + spacing;
            layout_set.borrow_mut().set_pos(right_x, start_y);
            layout_abt
                .borrow_mut()
                .set_pos(right_x, start_y + (btn_h + spacing));
            layout_exit
                .borrow_mut()
                .set_pos(right_x, start_y + (btn_h + spacing) * 2);
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
