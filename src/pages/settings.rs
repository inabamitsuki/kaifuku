use crate::app::Page;
use crate::theme::Theme;
use crate::util::config::Config;
use crate::widgets::{create_primary_button, create_secondary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{
    button::CheckButton, dialog, enums::Color, frame::Frame, group::Group, menu::Choice,
    menu::MenuFlag, prelude::*,
};
use std::cell::RefCell;
use std::rc::Rc;

pub struct SettingsPage;

impl SettingsPage {
    pub fn new(win_w: i32, win_h: i32, nav_tx: Sender<Page>) -> Self {
        let theme = Theme::global();
        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 200;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0, 0, title_w, title_h, "Settings",
        )));

        let start_y = 80;
        let label_w = 200;
        let field_w = 300;
        let field_h = 35;
        let spacing = 50;

        let mut folder_label = Frame::new(20, start_y, label_w, 30, "Default Recovery Folder:");
        folder_label.set_label_color(theme.text);
        folder_label.set_label_size(theme.font_size);

        let folder_choice = Rc::new(RefCell::new({
            let mut c = Choice::new(20 + label_w, start_y, field_w, field_h, "");
            let cfg = Config::load(&Config::default_path()).unwrap_or_default();
            let default = cfg.default_recovery_folder.clone();
            let mut options = vec![
                default.clone(),
                format!(
                    "{}/recovery",
                    dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                        .to_string_lossy()
                ),
            ];
            for f in &["/media/usb/recovery", "/tmp/recovery"] {
                if !options.iter().any(|o| o == f) {
                    options.push(f.to_string());
                }
            }
            for f in &options {
                c.add(f, fltk::enums::Shortcut::None, MenuFlag::Normal, |_| {});
            }
            c.set_value(0);
            c.set_color(Color::from_rgb(30, 30, 30));
            c.set_text_color(theme.text);
            c
        }));

        let mut log_label = Frame::new(20, start_y + spacing, label_w, 30, "Auto Save Log:");
        log_label.set_label_color(theme.text);
        log_label.set_label_size(theme.font_size);

        let log_check = Rc::new(RefCell::new({
            let mut c = CheckButton::new(20 + label_w, start_y + spacing, field_w, field_h, "");
            c.set_value(true);
            c.set_color(theme.text);
            c
        }));

        let save_btn = Rc::new(RefCell::new({
            let f_choice = folder_choice.clone();
            let l_check = log_check.clone();
            let mut b = create_primary_button(0, 0, 150, 50, "Save");
            b.set_callback(move |_| {
                let cfg = Config {
                    default_recovery_folder: f_choice
                        .borrow()
                        .text(f_choice.borrow().value())
                        .unwrap_or_default(),
                    auto_save_log: l_check.borrow().is_checked(),
                    photorec_path: None,
                };
                let path = Config::default_path();
                if let Err(e) = cfg.save(&path) {
                    dialog::alert(0, 0, &format!("Failed to save settings: {}", e));
                } else {
                    dialog::message(0, 0, "Settings saved successfully.");
                }
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

        let rel = (title.clone(), save_btn.clone(), back_btn.clone());
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
