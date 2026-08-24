use crate::app::{Page, RecoveredFile};
use crate::backend::filetypes::{CategoryMask, FileCategory, ALL_CATEGORIES};
use crate::theme::Theme;
use crate::widgets::{create_primary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{button::CheckButton, frame::Frame, group::Group, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub struct RecoveryPage;

impl RecoveryPage {
    pub fn new(
        win_w: i32,
        win_h: i32,
        files: Arc<Vec<RecoveredFile>>,
        scan_start_time: Option<std::time::Instant>,
        output_dir: &str,
        nav_tx: Sender<Page>,
    ) -> Self {
        let theme = Theme::global();

        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 260;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0,
            0,
            title_w,
            title_h,
            "Recovery Complete",
        )));

        let elapsed = scan_start_time.map(|t| t.elapsed()).unwrap_or_default();
        let secs = elapsed.as_secs();
        let mins = secs / 60;
        let secs = secs % 60;

        let total_files: u64 = files.len() as u64;
        let total_size: u64 = files.iter().map(|f| f.size).sum();

        let mut per_cat: std::collections::HashMap<FileCategory, (u64, u64)> =
            ALL_CATEGORIES.iter().map(|&c| (c, (0, 0))).collect();
        for f in files.iter() {
            let cat = FileCategory::classify(&f.extension);
            let e = per_cat.get_mut(&cat).unwrap();
            e.0 += 1;
            e.1 += f.size;
        }
        let per_cat: Vec<(FileCategory, u64, u64)> = ALL_CATEGORIES
            .iter()
            .map(|&c| {
                let (n, s) = per_cat[&c];
                (c, n, s)
            })
            .collect();

        let filter = Rc::new(RefCell::new(CategoryMask::all()));

        let start_y = 80;
        let line_h = 28;
        let indent = 40;

        let make_label = |y: i32, text: &str, bold: bool| {
            let mut f = Frame::new(indent, y, win_w - indent * 2, line_h, text);
            f.set_label_color(theme.text);
            f.set_label_size(if bold {
                theme.font_size + 2
            } else {
                theme.font_size - 1
            });
            f
        };

        let total_files_label = Rc::new(RefCell::new(make_label(
            start_y,
            &format!("Total files recovered:  {}", total_files),
            true,
        )));
        let total_size_label = Rc::new(RefCell::new(make_label(
            start_y + line_h,
            &format!("Total size:  {}", Self::fmt_size(total_size)),
            false,
        )));
        let _elapsed_line = make_label(
            start_y + line_h * 2,
            &format!("Time elapsed:  {}m {}s", mins, secs),
            false,
        );

        let _type_header = make_label(start_y + line_h * 4, "File types:", true);

        let row_h = 30;
        let row_start = start_y + line_h * 5;
        let cb_w = 150;
        let mut cat_checks: Vec<Rc<RefCell<CheckButton>>> = Vec::new();
        let mut cat_lines: Vec<Rc<RefCell<Frame>>> = Vec::new();
        for (i, (cat, _n, _s)) in per_cat.iter().enumerate() {
            let y = row_start + i as i32 * row_h;
            let mut cb = CheckButton::new(indent, y, cb_w, row_h, cat.label());
            cb.set_label_color(theme.text);
            cb.set_label_size(theme.font_size - 1);
            cb.set_color(theme.text);
            cb.set_value(true);
            cat_checks.push(Rc::new(RefCell::new(cb)));
            cat_lines.push(Rc::new(RefCell::new(Frame::new(
                indent + cb_w,
                y,
                win_w - indent * 2 - cb_w,
                row_h,
                "",
            ))));
        }

        let _dest = make_label(
            start_y + line_h * 5 + row_h * 6 + line_h,
            &format!("Destination:  {}", output_dir),
            false,
        );

        // Refresh the summary based on which categories are enabled.
        let refresh_rc = Rc::new(RefCell::new(Box::new(|| {}) as Box<dyn FnMut()>));
        {
            let filter_c = filter.clone();
            let total_files_c = total_files_label.clone();
            let total_size_c = total_size_label.clone();
            let per_cat_c = per_cat.clone();
            let cat_lines_c = cat_lines.clone();
            let refresh: Box<dyn FnMut()> = Box::new(move || {
                let mask = *filter_c.borrow();
                let mut tcount: u64 = 0;
                let mut tsize: u64 = 0;
                for (cat, n, s) in &per_cat_c {
                    if mask.includes(*cat) {
                        tcount += n;
                        tsize += s;
                    }
                }
                total_files_c
                    .borrow_mut()
                    .set_label(&format!("Total files recovered:  {}", tcount));
                total_size_c
                    .borrow_mut()
                    .set_label(&format!("Total size:  {}", Self::fmt_size(tsize)));
                for (i, (cat, n, s)) in per_cat_c.iter().enumerate() {
                    let text = if mask.includes(*cat) {
                        format!("  {} files  ({})", n, Self::fmt_size(*s))
                    } else {
                        "  (hidden)".to_string()
                    };
                    if let Some(line) = cat_lines_c.get(i) {
                        line.borrow_mut().set_label(&text);
                    }
                }
            });
            *refresh_rc.borrow_mut() = refresh;
        }

        {
            for (i, (cat, _n, _s)) in per_cat.iter().enumerate() {
                let cat = *cat;
                let cb = cat_checks[i].clone();
                let f = filter.clone();
                let refresh_c = refresh_rc.clone();
                cb.borrow_mut().set_callback(move |btn| {
                    let cur = *f.borrow();
                    let newmask = match cat {
                        FileCategory::Photo => CategoryMask {
                            photo: btn.is_checked(),
                            ..cur
                        },
                        FileCategory::Video => CategoryMask {
                            video: btn.is_checked(),
                            ..cur
                        },
                        FileCategory::Document => CategoryMask {
                            document: btn.is_checked(),
                            ..cur
                        },
                        FileCategory::Audio => CategoryMask {
                            audio: btn.is_checked(),
                            ..cur
                        },
                        FileCategory::Archive => CategoryMask {
                            archive: btn.is_checked(),
                            ..cur
                        },
                        FileCategory::Other => CategoryMask {
                            other: btn.is_checked(),
                            ..cur
                        },
                    };
                    *f.borrow_mut() = newmask;
                    let mut r = refresh_c.borrow_mut();
                    r();
                });
            }
        }

        // Initial refresh so hidden categories render consistently.
        {
            let mut r = refresh_rc.borrow_mut();
            r();
        }

        let back_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_primary_button(0, 0, 200, 50, "Back to Menu");
            b.set_callback(move |_| {
                let _ = nav.send(Page::Menu);
            });
            b
        }));

        let rel = (title.clone(), back_btn.clone());
        let relayout = move |w: i32, h: i32| {
            rel.0.borrow_mut().set_pos((w - title_w) / 2, 20);
            rel.1.borrow_mut().set_pos(w / 2 - 100, h - 80);
        };

        relayout(win_w, win_h);
        page.resize_callback(move |_, _, _, w, h| relayout(w, h));

        let spacer = fltk::frame::Frame::new(0, 0, 0, 0, "");
        page.resizable(&spacer);
        page.end();
        page.show();

        Self
    }

    fn fmt_size(size: u64) -> String {
        if size >= 1_000_000_000 {
            format!("{:.1} GB", size as f64 / 1_000_000_000.0)
        } else if size >= 1_000_000 {
            format!("{:.1} MB", size as f64 / 1_000_000.0)
        } else if size >= 1024 {
            format!("{:.1} KB", size as f64 / 1024.0)
        } else {
            format!("{} B", size)
        }
    }
}
