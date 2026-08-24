use crate::app::Page;
use crate::backend::{PhotoRecOptions, WorkerCommand};
use crate::theme::{self, Theme};
use crate::util::config::Config;
use crate::widgets::{create_primary_button, create_secondary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{enums::Color, frame::Frame, group::Group, prelude::*};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

pub struct ImageScanPage;

impl ImageScanPage {
    pub fn new(
        win_w: i32,
        win_h: i32,
        nav_tx: Sender<Page>,
        cmd_tx: Sender<WorkerCommand>,
    ) -> Self {
        let theme = Theme::global();
        let config = Config::load(&Config::default_path()).unwrap_or_default();
        let default_dir = config.default_recovery_folder.clone();

        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 300;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0,
            0,
            title_w,
            title_h,
            "Recover From Image",
        )));

        let label_w = 120;
        let field_w = 400;
        let field_h = 35;
        let spacing = 50;
        let start_y = 100;
        let anchor_x = 20;

        let img_input = Rc::new(RefCell::new({
            let mut i = fltk::input::Input::new(anchor_x + label_w, start_y, field_w, field_h, "");
            i.set_color(Color::from_rgb(30, 30, 30));
            i.set_text_color(theme.text);
            i.set_selection_color(theme.primary);
            i.set_readonly(true);
            i
        }));

        let browse_btn = Rc::new(RefCell::new({
            let mut b = fltk::button::Button::new(
                anchor_x + label_w + field_w + 10,
                start_y,
                100,
                field_h,
                "Browse",
            );
            b.set_color(theme.primary);
            b.set_label_color(theme.text);
            b.set_label_size(theme.font_size);
            b
        }));

        let info_y = start_y + spacing;
        let info_label = Rc::new(RefCell::new(Frame::new(
            anchor_x,
            info_y,
            500,
            25,
            "No image selected",
        )));
        {
            let mut f = info_label.borrow_mut();
            f.set_label_color(theme.text);
            f.set_label_size(theme.font_size - 2);
        }

        let dest_input = Rc::new(RefCell::new({
            let mut i = fltk::input::Input::new(
                anchor_x + label_w,
                info_y + spacing,
                field_w - 110,
                field_h,
                "",
            );
            i.set_value(&default_dir);
            i.set_color(Color::from_rgb(30, 30, 30));
            i.set_text_color(theme.text);
            i
        }));

        let _dest_browse = Rc::new(RefCell::new({
            let dest_input = dest_input.clone();
            let mut b = fltk::button::Button::new(
                anchor_x + label_w + field_w - 110 + 10,
                info_y + spacing,
                100,
                field_h,
                "Browse",
            );
            b.set_color(theme.primary);
            b.set_label_color(theme.text);
            b.set_label_size(theme.font_size);
            b.set_callback(move |_| {
                if let Some(dir) = theme::dir_chooser("Select Destination Directory", "/tmp") {
                    dest_input.borrow_mut().set_value(&dir);
                }
            });
            b
        }));

        let path = Rc::new(RefCell::new(String::new()));
        let path_browse = path.clone();
        let info_txt = info_label.clone();
        let img_input_browse = img_input.clone();

        browse_btn.borrow_mut().set_callback(move |_| {
            if let Some(fname) =
                theme::file_chooser("Select Disk Image", "*.{dd,img,iso,raw,bin,dmg}", ".")
            {
                *path_browse.borrow_mut() = fname.clone();
                img_input_browse.borrow_mut().set_value(&fname);

                let meta = std::fs::metadata(&fname);
                let size_str = match meta {
                    Ok(m) => {
                        let s = m.len();
                        if s > 1_000_000_000 {
                            format!("{:.2} GB", s as f64 / 1_000_000_000.0)
                        } else if s > 1_000_000 {
                            format!("{:.2} MB", s as f64 / 1_000_000.0)
                        } else {
                            format!("{} bytes", s)
                        }
                    }
                    Err(_) => "unknown size".to_string(),
                };
                let name = Path::new(&fname)
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                info_txt
                    .borrow_mut()
                    .set_label(&format!("{}  ({})", name, size_str));
            }
        });

        let back_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_secondary_button(0, 0, 150, 50, "Back");
            b.set_callback(move |_| {
                let _ = nav.send(Page::Menu);
            });
            b
        }));

        let start_btn = Rc::new(RefCell::new({
            let path_start = path.clone();
            let dest_input_start = dest_input.clone();
            let tx = cmd_tx.clone();
            let default_dir = default_dir.clone();
            let mut b = create_primary_button(0, 0, 180, 50, "Start Scan");
            b.set_callback(move |_| {
                let dev = path_start.borrow().clone();
                if dev.is_empty() {
                    return;
                }
                let out = dest_input_start.borrow().value();
                let cmd = WorkerCommand::StartScan {
                    device: dev,
                    output_dir: if out.is_empty() {
                        default_dir.clone()
                    } else {
                        out
                    },
                    options: PhotoRecOptions::default(),
                    part_offset: 0,
                    part_size: 0,
                    dd_path: None,
                    scan_type: "Deep".to_string(),
                };
                let _ = tx.send(cmd);
            });
            b
        }));

        let rel = (title.clone(), back_btn.clone(), start_btn.clone());
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
