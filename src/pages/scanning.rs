use crate::app::ScanProgress;
use crate::backend::WorkerCommand;
use crate::theme::Theme;
use crate::widgets::{create_primary_button, create_progress_bar, create_title};
use crossbeam_channel::Sender;
use fltk::{
    enums::Color, frame::Frame, group::Group, prelude::*, text::TextBuffer, text::TextDisplay,
};
use std::cell::RefCell;
use std::rc::Rc;

pub struct ScanningPage;

impl ScanningPage {
    pub fn new(
        win_w: i32,
        win_h: i32,
        progress: &ScanProgress,
        log_lines: &[String],
        cmd_tx: Sender<WorkerCommand>,
    ) -> Self {
        let theme = Theme::global();

        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 200;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0, 0, title_w, title_h, "Scanning",
        )));

        let prog = Rc::new(RefCell::new(create_progress_bar(0, 0, 0, 30)));
        prog.borrow_mut().set_value(progress.percent as f64);

        let file_text = format!("Current: {}", progress.current_file);
        let current_file = Rc::new(RefCell::new(Frame::new(0, 0, 0, 30, file_text.as_str())));
        current_file.borrow_mut().set_label_color(theme.text);
        current_file.borrow_mut().set_label_size(theme.font_size);

        let start_y = 170;
        let found_text = format!("Files Found: {}", progress.files_found);
        let files_found = Rc::new(RefCell::new(Frame::new(0, 0, 300, 30, found_text.as_str())));
        files_found.borrow_mut().set_label_color(theme.text);
        files_found.borrow_mut().set_label_size(theme.font_size);

        let log_label = Rc::new(RefCell::new(Frame::new(0, 0, 200, 30, "Recovery Log:")));
        log_label.borrow_mut().set_label_color(theme.text);
        log_label.borrow_mut().set_label_size(theme.font_size);

        let mut log_buf = TextBuffer::default();
        log_buf.set_text(&log_lines.join("\n"));

        let log_panel = Rc::new(RefCell::new(TextDisplay::new(0, 0, 0, 0, "")));
        {
            let mut p = log_panel.borrow_mut();
            p.set_buffer(log_buf);
            p.set_color(Color::from_rgb(20, 20, 20));
            p.set_text_color(theme.text);
            p.set_text_font(fltk::enums::Font::Courier);
            p.set_text_size(theme.font_size - 4);
            let len = p.buffer().map(|b| b.length()).unwrap_or(0);
            let lines = p.count_lines(0, len, true);
            p.set_insert_position(len);
            p.scroll(lines, 0);
        }

        let stop_btn = Rc::new(RefCell::new({
            let tx = cmd_tx.clone();
            let mut b = create_primary_button(0, 0, 180, 50, "Stop");
            b.set_callback(move |_| {
                let _ = tx.send(WorkerCommand::StopScan);
            });
            b
        }));

        let rel = (
            title.clone(),
            prog.clone(),
            current_file.clone(),
            files_found.clone(),
            log_label.clone(),
            log_panel.clone(),
            stop_btn.clone(),
        );
        let relayout = move |w: i32, h: i32| {
            rel.0.borrow_mut().set_pos((w - title_w) / 2, 20);
            rel.1.borrow_mut().resize(20, 80, w - 40, 30);
            rel.2.borrow_mut().resize(20, 120, w - 40, 30);
            rel.3.borrow_mut().set_pos(20, start_y);
            rel.4.borrow_mut().set_pos(20, start_y + 50);
            rel.5
                .borrow_mut()
                .resize(20, start_y + 85, w - 40, h - (start_y + 85) - 100);
            rel.6.borrow_mut().set_pos(w - 200, h - 80);
        };

        relayout(win_w, win_h);
        page.resize_callback(move |_, _, _, w, h| relayout(w, h));

        page.end();
        page.show();

        Self
    }
}
