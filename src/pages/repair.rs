use crossbeam_channel::{unbounded, Receiver, Sender};
use fltk::{
    app, button::Button, dialog, enums::Color, frame::Frame, group::Group, input::Input,
    misc::Progress, prelude::*, text::TextBuffer, text::TextDisplay,
};

use crate::app::Page;
use crate::backend::repair::{self, FileAnalysis, FileType, RepairEvent};
use crate::theme::{self, Theme};
use crate::widgets::{create_primary_button, create_secondary_button, create_title};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct PageState {
    file_path: String,
    file_name: String,
    file_size: u64,
    analysis: Option<FileAnalysis>,
    output_dir: String,
    repair_rx: Option<Receiver<RepairEvent>>,
    stop_flag: Option<Arc<AtomicBool>>,
    repair_active: bool,
    active: bool,
}

pub struct RepairPage;

impl RepairPage {
    pub fn new(win_w: i32, win_h: i32, nav_tx: Sender<Page>) -> Self {
        let theme = Theme::global();

        let state = Rc::new(RefCell::new(PageState {
            file_path: String::new(),
            file_name: String::new(),
            file_size: 0,
            analysis: None,
            output_dir: "/tmp/repair_output".to_string(),
            repair_rx: None,
            stop_flag: None,
            repair_active: false,
            active: true,
        }));

        let mut page = Group::new(0, 0, win_w, win_h, "");
        page.set_color(theme.background);

        let title_w = 200;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0,
            0,
            title_w,
            title_h,
            "File Repair",
        )));

        let row1_y = 80;
        let label_w = 50;
        let field_w = win_w - 330;

        let mut file_label = Frame::new(20, row1_y, label_w, 35, "File:");
        file_label.set_label_color(theme.text);
        file_label.set_label_size(theme.font_size);

        let mut file_input = Input::new(20 + label_w, row1_y, field_w, 35, "");
        file_input.set_color(Color::from_rgb(30, 30, 30));
        file_input.set_text_color(theme.text);
        file_input.set_readonly(true);

        let state_clone = state.clone();
        let mut file_input_clone = file_input.clone();
        let mut browse_btn =
            fltk::button::Button::new(20 + label_w + field_w + 5, row1_y, 100, 35, "Browse");
        browse_btn.set_color(theme.primary);
        browse_btn.set_label_color(theme.text);
        browse_btn.set_label_size(theme.font_size);
        browse_btn.set_callback(move |_| {
            if let Some(fname) = theme::file_chooser("Select File to Repair", "*", ".") {
                let mut st = state_clone.borrow_mut();
                st.file_path = fname.clone();
                st.file_name = Path::new(&fname)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                st.file_size = std::fs::metadata(&fname).map(|m| m.len()).unwrap_or(0);
                st.analysis = None;
                file_input_clone.set_value(&fname);
            }
        });

        let row2_y = row1_y + 45;

        let mut type_frame = Frame::new(20, row2_y, 250, 25, "Type: \u{2014}");
        type_frame.set_label_color(theme.text);
        type_frame.set_label_size(theme.font_size - 2);

        let mut size_frame = Frame::new(280, row2_y, 200, 25, "Size: \u{2014}");
        size_frame.set_label_color(theme.text);
        size_frame.set_label_size(theme.font_size - 2);

        let mut status_frame = Frame::new(490, row2_y, 350, 25, "Status: No file selected");
        status_frame.set_label_color(theme.text);
        status_frame.set_label_size(theme.font_size - 2);

        let row3_y = row2_y + 40;

        let mut output_label = Frame::new(20, row3_y, 50, 35, "Out:");
        output_label.set_label_color(theme.text);
        output_label.set_label_size(theme.font_size);

        let out_w = field_w - 110;
        let mut output_input = Input::new(20 + 50, row3_y, out_w, 35, "");
        output_input.set_color(Color::from_rgb(30, 30, 30));
        output_input.set_text_color(theme.text);
        output_input.set_value("/tmp/repair_output");

        let state_out = state.clone();
        let mut output_input_clone = output_input.clone();
        let mut out_browse_btn =
            fltk::button::Button::new(20 + 50 + out_w + 5, row3_y, 100, 35, "Browse");
        out_browse_btn.set_color(theme.primary);
        out_browse_btn.set_label_color(theme.text);
        out_browse_btn.set_label_size(theme.font_size);
        out_browse_btn.set_callback(move |_| {
            if let Some(dir) = theme::dir_chooser("Select Output Directory", "") {
                output_input_clone.set_value(&dir);
                state_out.borrow_mut().output_dir = dir;
            }
        });

        let row4_y = row3_y + 50;

        let log_buf = Rc::new(RefCell::new(TextBuffer::default()));

        let state_an = state.clone();
        let mut type_frame_an = type_frame.clone();
        let mut size_frame_an = size_frame.clone();
        let mut status_frame_an = status_frame.clone();
        let log_buf_an = log_buf.clone();

        let mut analyze_btn = create_primary_button(20, row4_y, 150, 40, "Analyze");
        analyze_btn.set_callback(move |_| {
            let st = state_an.borrow();
            if st.file_path.is_empty() {
                dialog::alert(0, 0, "Please select a file first.");
                return;
            }
            let path = st.file_path.clone();
            let name = st.file_name.clone();
            let size = st.file_size;
            drop(st);

            let analysis = match repair::analyze_file(&path) {
                Ok(a) => a,
                Err(e) => {
                    dialog::alert(0, 0, &format!("Analysis failed: {}", e));
                    return;
                }
            };

            {
                let mut st = state_an.borrow_mut();
                st.analysis = Some(analysis.clone());
            }

            type_frame_an.set_label(&format!("Type: {}", analysis.file_type.name()));
            size_frame_an.set_label(&format!("Size: {}", fmt_size(size)));

            let mut buf = log_buf_an.borrow_mut();
            buf.set_text("");
            buf.append(&format!(
                "\u{2554}{0}\u{2557}\n",
                "\u{2550}\u{2550} File Analysis \u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}"
            ));
            buf.append(&format!("\u{2551} File:      {}\n", name));
            buf.append(&format!("\u{2551} Size:      {}\n", fmt_size(size)));
            buf.append(&format!("\u{2551} Format:    {}\n", analysis.file_type.name()));

            let mut status_txt = if analysis.file_type == FileType::Unknown {
                if analysis.embedded_offset.is_some() {
                    buf.append(&format!(
                        "\u{2551} \u{26a0} Embedded PNG found at byte offset {}\n",
                        analysis.embedded_offset.unwrap()
                    ));
                    if !analysis.details.is_empty() {
                        buf.append(&format!("\u{2551}   Chunks:  {}\n", analysis.details));
                    }
                    buf.append(&format!(
                        "\u{2551} \u{2139} Header corrupted \u{2014} repair available\n"
                    ));
                    "\u{26a0} Header damaged \u{2014} repair available".to_string()
                } else {
                    buf.append(&format!(
                        "\u{2551} \u{2717} Unrecognized file format\n"
                    ));
                    "\u{2717} Unknown format".to_string()
                }
            } else {
                if analysis.has_header {
                    buf.append(&format!(
                        "\u{2551} \u{2713} Header: {} (valid)\n",
                        analysis.file_type.header_hex()
                    ));
                } else {
                    buf.append(&format!(
                        "\u{2551} \u{26a0} Header missing \u{2014} repair available\n"
                    ));
                }
                if !analysis.has_header {
                    buf.append(&format!(
                        "\u{2551} \u{2139} Status:  File header is damaged \u{2014} repair recommended\n"
                    ));
                    "\u{26a0} Header damaged \u{2014} repair available".to_string()
                } else if analysis.has_footer {
                    buf.append(&format!(
                        "\u{2551} \u{2713} Footer: {} (present)\n",
                        analysis.file_type.footer_hex()
                    ));
                    buf.append(&format!(
                        "\u{2551} \u{2139} Status:  File appears intact\n"
                    ));
                    "\u{2713} Intact".to_string()
                } else {
                    buf.append(&format!(
                        "\u{2551} \u{2717} Footer: {} (missing/truncated)\n",
                        analysis.file_type.footer_hex()
                    ));
                    buf.append(&format!(
                        "\u{2551} \u{2139} Status:  File is truncated \u{2014} repair recommended\n"
                    ));
                    "\u{26a0} Truncated \u{2014} repair possible".to_string()
                }
            };
            if let Some(warn) = analysis.payload.warn_text() {
                buf.append(&format!("\u{2551} \u{2717} {}\n", warn));
                status_txt = format!(
                    "\u{2717} Cannot repair \u{2014} payload {}",
                    analysis.payload.label()
                );
            }
            buf.append(&format!(
                "\u{255a}{0}\u{255d}\n",
                "\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}"
            ));

            status_frame_an.set_label(&format!("Status: {}", status_txt));
        });

        let mut progress_bar = Progress::new(190, row4_y, 300, 40, "");
        progress_bar.set_color(Color::from_rgb(30, 30, 30));
        progress_bar.set_selection_color(Color::from_rgb(0, 180, 0));
        progress_bar.set_value(0.0);

        let mut repair_btn = create_primary_button(510, row4_y, 150, 40, "Repair");
        let mut stop_btn = create_secondary_button(680, row4_y, 100, 40, "Stop");
        stop_btn.set_color(Color::from_rgb(180, 40, 40));
        stop_btn.set_label_color(Color::White);
        stop_btn.deactivate();

        let state_rpr = state.clone();
        let log_rpr = log_buf.clone();
        let mut prog_rpr = progress_bar.clone();
        let _status_rpr = status_frame.clone();
        let mut repair_btn_clone = repair_btn.clone();
        let mut stop_btn_clone = stop_btn.clone();
        let output_input_rpr = output_input.clone();

        let state_rpr2 = state.clone();
        let log_rpr2 = log_buf.clone();
        let mut prog_rpr2 = progress_bar.clone();
        let mut status_rpr2 = status_frame.clone();
        let mut repair_btn_clone2 = repair_btn.clone();
        let mut stop_btn_clone2 = stop_btn.clone();

        repair_btn.set_callback(move |_| {
            let path;
            let analysis;
            let out_dir;
            {
                let mut st = state_rpr.borrow_mut();
                if st.file_path.is_empty() {
                    dialog::alert(0, 0, "Please select a file first.");
                    return;
                }
                if st.repair_active {
                    return;
                }
                let an = st.analysis.clone();
                if an.is_none() {
                    dialog::alert(0, 0, "Please analyze the file before repairing.");
                    return;
                }
                path = st.file_path.clone();
                analysis = an.unwrap();
                if !analysis.payload.is_recoverable() {
                    dialog::alert(
                        0,
                        0,
                        analysis
                            .payload
                            .warn_text()
                            .unwrap_or("Image data payload cannot be recovered"),
                    );
                    return;
                }
                out_dir = output_input_rpr.value();
                st.output_dir = out_dir.clone();
                let (tx, rx) = unbounded();
                st.repair_rx = Some(rx);
                let stop_flag = Arc::new(AtomicBool::new(false));
                st.stop_flag = Some(stop_flag.clone());
                st.repair_active = true;
                repair::repair_file(path, out_dir, analysis, tx, stop_flag);
            }

            repair_btn_clone.deactivate();
            stop_btn_clone.activate();
            prog_rpr.set_value(0.0);

            {
                let mut buf = log_rpr.borrow_mut();
                buf.append(&format!(
                    "\u{2554}{0}\u{2557}\n",
                    "\u{2550}\u{2550} Repair Process \u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}"
                ));
            }
        });

        stop_btn.set_callback(move |_| {
            let st = state_rpr2.borrow();
            if let Some(ref flag) = st.stop_flag {
                flag.store(true, Ordering::SeqCst);
            }
            let mut buf = log_rpr2.borrow_mut();
            buf.append("\u{2551} \u{2717} Repair cancelled by user\n");
            buf.append(&format!(
                "\u{255a}{0}\u{255d}\n",
                "\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}"
            ));
            stop_btn_clone2.deactivate();

            let mut st = state_rpr2.borrow_mut();
            st.repair_active = false;
            st.stop_flag = None;
            st.repair_rx = None;

            status_rpr2.set_label("Status: \u{2717} Cancelled");
            prog_rpr2.set_value(0.0);
            repair_btn_clone2.activate();
        });

        let row5_y = row4_y + 50;
        let log_h = win_h - row5_y - 100;

        let mut log_display = TextDisplay::new(20, row5_y, win_w - 40, log_h, "");
        log_display.set_buffer(log_buf.borrow().clone());
        log_display.set_color(Color::from_rgb(16, 16, 16));
        log_display.set_text_color(Color::from_rgb(200, 200, 200));
        log_display.set_text_font(fltk::enums::Font::Courier);
        log_display.set_text_size(theme.font_size - 4);

        let nav = nav_tx.clone();
        let state_back = state.clone();
        let back_btn = Rc::new(RefCell::new({
            let nav = nav.clone();
            let state_back = state_back.clone();
            let mut b = create_secondary_button(0, 0, 180, 50, "Back");
            b.set_callback(move |_| {
                {
                    let mut st = state_back.borrow_mut();
                    st.active = false;
                    if let Some(ref flag) = st.stop_flag {
                        flag.store(true, Ordering::SeqCst);
                    }
                    st.stop_flag = None;
                    st.repair_rx = None;
                }
                let _ = nav.send(Page::Menu);
            });
            b
        }));

        let poll_state = state.clone();
        let poll_progress = progress_bar.clone();
        let poll_log = log_buf.clone();
        let poll_status = status_frame.clone();
        let poll_repair_btn = repair_btn.clone();
        let poll_stop_btn = stop_btn.clone();

        #[allow(deprecated)]
        fn poll_fn(
            state: Rc<RefCell<PageState>>,
            mut progress_bar: Progress,
            log_buf: Rc<RefCell<TextBuffer>>,
            mut status_frame: Frame,
            mut repair_btn: Button,
            mut stop_btn: Button,
        ) {
            let mut finished = false;
            {
                let st = state.borrow();
                if !st.active {
                    return;
                }
                if let Some(ref rx) = st.repair_rx {
                    while let Ok(event) = rx.try_recv() {
                        match event {
                            RepairEvent::Started => {}
                            RepairEvent::Progress(p) => {
                                progress_bar.set_value(p.percent);
                                let mut buf = log_buf.borrow_mut();
                                buf.append(&format!(
                                    "\u{2551} {:3.0}%  {}\n",
                                    p.percent, p.message
                                ));
                            }
                            RepairEvent::Log(msg) => {
                                let mut buf = log_buf.borrow_mut();
                                buf.append(&format!("\u{2551} {}\n", msg));
                            }
                            RepairEvent::Complete { output_path, size } => {
                                {
                                    let mut buf = log_buf.borrow_mut();
                                    buf.append("\u{2551} \u{2713} File repaired successfully!\n");
                                    buf.append(&format!("\u{2551}   Output: {}\n", output_path));
                                    buf.append(&format!("\u{2551}   Size:   {}\n", fmt_size(size)));
                                    buf.append(&format!(
                                        "\u{255a}{0}\u{255d}\n",
                                        "\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}"
                                    ));
                                }
                                progress_bar.set_value(100.0);
                                status_frame.set_label(&format!(
                                    "\u{2713} Repaired \u{2192} {}",
                                    output_path
                                ));
                                repair_btn.activate();
                                stop_btn.deactivate();
                                finished = true;
                            }
                            RepairEvent::Error(e) => {
                                {
                                    let mut buf = log_buf.borrow_mut();
                                    buf.append(&format!(
                                        "\u{2551} \u{2717} Repair failed: {}\n",
                                        e
                                    ));
                                    buf.append(&format!(
                                        "\u{255a}{0}\u{255d}\n",
                                        "\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}"
                                    ));
                                }
                                status_frame.set_label(&format!("\u{2717} Repair failed: {}", e));
                                repair_btn.activate();
                                stop_btn.deactivate();
                                finished = true;
                            }
                        }
                    }
                }
                if finished {
                    let mut st = state.borrow_mut();
                    st.repair_active = false;
                    st.stop_flag = None;
                    st.repair_rx = None;
                }
            }
            if !finished && state.borrow().active {
                let s = state.clone();
                let p = progress_bar.clone();
                let l = log_buf.clone();
                let sf = status_frame.clone();
                let rb = repair_btn.clone();
                let sb = stop_btn.clone();
                app::add_timeout(0.1, move || {
                    poll_fn(
                        s.clone(),
                        p.clone(),
                        l.clone(),
                        sf.clone(),
                        rb.clone(),
                        sb.clone(),
                    )
                });
            }
        }

        #[allow(deprecated)]
        app::add_timeout(0.1, {
            let s = poll_state.clone();
            let p = poll_progress.clone();
            let l = poll_log.clone();
            let sf = poll_status.clone();
            let rb = poll_repair_btn.clone();
            let sb = poll_stop_btn.clone();
            move || {
                poll_fn(
                    s.clone(),
                    p.clone(),
                    l.clone(),
                    sf.clone(),
                    rb.clone(),
                    sb.clone(),
                )
            }
        });

        let rel = (title.clone(), back_btn.clone());
        let relayout = move |w: i32, h: i32| {
            rel.0.borrow_mut().set_pos((w - title_w) / 2, 20);
            rel.1.borrow_mut().set_pos(w - 200, h - 80);
        };
        relayout(win_w, win_h);
        page.resize_callback(move |_, _, _, w, h| relayout(w, h));

        page.resizable(&log_display);
        page.end();
        page.show();

        Self
    }
}

fn fmt_size(s: u64) -> String {
    if s >= 1_000_000_000 {
        format!("{:.2} GB", s as f64 / 1_000_000_000.0)
    } else if s >= 1_000_000 {
        format!("{:.2} MB", s as f64 / 1_000_000.0)
    } else if s >= 1024 {
        format!("{:.1} KB", s as f64 / 1024.0)
    } else {
        format!("{} B", s)
    }
}
