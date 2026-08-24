use crate::app::Page;
use crate::backend::repair::deep_scan;
use crate::theme::{self, Theme};
use crate::util::disks::{enumerate_disks, DiskInfo};
use crate::util::perms::has_device_access;
use crate::widgets::{create_primary_button, create_secondary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{
    app,
    browser::HoldBrowser,
    button::{Button, RadioRoundButton},
    enums::{Align, Color, Font, FrameType, Shortcut},
    frame::Frame,
    group::Group,
    input::Input,
    menu::{Choice, MenuFlag},
    prelude::*,
    text::{TextBuffer, TextDisplay},
    window::Window,
};
use std::cell::RefCell;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct AdvancedPage;

impl AdvancedPage {
    pub fn new(win_w: i32, win_h: i32, nav_tx: Sender<Page>) -> Self {
        let _theme = Theme::global();
        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 200;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0,
            0,
            title_w,
            title_h,
            "Advanced Mode",
        )));

        let start_y = 80;
        let btn_w = 250;
        let btn_h = 50;
        let spacing = 20;

        let options: [(&str, fn()); 4] = [
            ("Hex Editor", hex_editor),
            ("Signature Scanner", signature_scanner),
            ("Recovery Log", recovery_log),
            ("Live USB Creator", live_usb_creator),
        ];

        for (i, &(label, handler)) in options.iter().enumerate() {
            let row = i as i32 / 2;
            let col = i as i32 % 2;
            let x = 20 + col * (btn_w + spacing);
            let y = start_y + row * (btn_h + spacing);
            let mut btn = create_primary_button(x, y, btn_w, btn_h, label);
            btn.set_callback(move |_| handler());
        }

        let image_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_primary_button(0, 0, btn_w, btn_h, "Recover From Image");
            b.set_callback(move |_| {
                let _ = nav.send(Page::ImageScan);
            });
            b
        }));

        let exp_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_secondary_button(0, 0, 250, 50, "Experimental Features");
            b.set_callback(move |_| {
                let _ = nav.send(Page::Experimental);
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

        let rel = (
            title.clone(),
            image_btn.clone(),
            exp_btn.clone(),
            back_btn.clone(),
        );
        let relayout = move |w: i32, h: i32| {
            rel.0.borrow_mut().set_pos((w - title_w) / 2, 20);
            rel.1
                .borrow_mut()
                .set_pos(20, start_y + 2 * (btn_h + spacing));
            rel.2.borrow_mut().set_pos(20, h - 80);
            rel.3.borrow_mut().set_pos(w - 200, h - 80);
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

fn make_hex_dump(data: &[u8], modified: &[bool], max_bytes: usize) -> String {
    let show = data.len().min(max_bytes);
    if show == 0 {
        return String::from(
            "Offset      00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F  ASCII\n(empty file)",
        );
    }
    let mut out = String::with_capacity(show * 5);
    out.push_str("Offset      00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F  ASCII\n");
    for off in (0..show).step_by(16) {
        let end = (off + 16).min(show);
        out.push_str(&format!("{:08X}  ", off));
        for i in off..(off + 16) {
            if i < end {
                if i < modified.len() && modified[i] {
                    out.push_str(&format!("{:02X}*", data[i]));
                } else {
                    out.push_str(&format!("{:02X} ", data[i]));
                }
            } else {
                out.push_str("   ");
            }
            if i == off + 7 {
                out.push(' ');
            }
        }
        out.push(' ');
        for i in off..end {
            let c = data[i];
            out.push(if c.is_ascii_graphic() || c == b' ' {
                c as char
            } else {
                '.'
            });
        }
        out.push('\n');
    }
    if data.len() > max_bytes {
        out.push_str(&format!(
            "... showing {} of {} bytes\n",
            max_bytes,
            data.len()
        ));
    }
    out
}

pub fn hex_editor() {
    let path = match theme::file_chooser("Open File for Hex Editor", "*", ".") {
        Some(p) => p,
        None => return,
    };
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            theme::message("Alert", &format!("Cannot read file: {}", e));
            return;
        }
    };
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default()
        .to_string();

    struct HexState {
        data: Vec<u8>,
        modified: Vec<bool>,
        path: String,
        dirty: bool,
    }
    let state = Rc::new(RefCell::new(HexState {
        modified: vec![false; data.len()],
        path: path.clone(),
        dirty: false,
        data,
    }));

    const W: i32 = 900;
    const H: i32 = 540;
    let hex_h = H - 140;
    let scr = app::screen_size();
    let wx = ((scr.0 - W as f64) / 2.0) as i32;
    let wy = ((scr.1 - H as f64) / 2.0) as i32;

    let win_holder: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
    let win_title = format!("Hex Editor — {}", name);
    let mut win = Window::new(wx.max(0), wy.max(0), W, H, win_title.as_str());
    theme::style_window(&mut win);
    *win_holder.borrow_mut() = Some(win.clone());

    let hex_init = make_hex_dump(&state.borrow().data, &state.borrow().modified, 65536);
    let mut buf = TextBuffer::default();
    buf.set_text(&hex_init);

    let mut display = TextDisplay::new(0, 0, W, hex_h, "");
    theme::style_field(&mut display);
    display.set_buffer(buf.clone());
    display.set_text_font(Font::Courier);
    display.set_text_size(13);
    display.set_frame(FrameType::DownBox);
    win.resizable(&display);

    let st_init = format!(
        "Modified: 0  |  Total: {}",
        fmt_size(state.borrow().data.len() as u64)
    );
    let mut status = Frame::new(10, hex_h + 5, W - 20, 22, st_init.as_str());
    theme::style_label(&mut status);
    status.set_align(Align::Left | Align::Inside);

    let mut offset_label = Frame::new(10, hex_h + 35, 50, 26, "Offset:");
    theme::style_label(&mut offset_label);
    offset_label.set_align(Align::Left | Align::Inside);
    let mut offset_input = Input::new(60, hex_h + 35, 100, 26, "");
    theme::style_input(&mut offset_input);
    offset_input.set_value("0x00000000");

    let mut value_label = Frame::new(175, hex_h + 35, 45, 26, "Value:");
    theme::style_label(&mut value_label);
    value_label.set_align(Align::Left | Align::Inside);
    let mut value_input = Input::new(220, hex_h + 35, 55, 26, "");
    theme::style_input(&mut value_input);

    let mut apply_btn = Button::new(285, hex_h + 35, 70, 26, "Apply");
    theme::style_muted_button(&mut apply_btn);
    let mut revert_btn = Button::new(365, hex_h + 35, 70, 26, "Revert");
    theme::style_muted_button(&mut revert_btn);

    let btn_y = hex_h + 70;
    let mut save_btn = Button::new(10, btn_y, 90, 30, "Save");
    theme::style_button(&mut save_btn);
    let mut save_as_btn = Button::new(110, btn_y, 100, 30, "Save As...");
    theme::style_button(&mut save_as_btn);
    let mut close_btn = Button::new(W - 100, btn_y, 90, 30, "Close");
    theme::style_button(&mut close_btn);

    apply_btn.set_callback({
        let state = state.clone();
        let mut buf = buf.clone();
        let mut status = status.clone();
        let offset_input = offset_input.clone();
        let mut value_input = value_input.clone();
        move |_| {
            let off_str = offset_input.value();
            let val_str = value_input.value();
            let offset = match usize::from_str_radix(off_str.trim_start_matches("0x"), 16) {
                Ok(v) => v,
                Err(_) => {
                    theme::message("Alert", "Invalid offset (must be hex)");
                    return;
                }
            };
            let value = match u8::from_str_radix(val_str.trim_start_matches("0x"), 16) {
                Ok(v) => v,
                Err(_) => {
                    theme::message("Alert", "Invalid value (must be 00-FF)");
                    return;
                }
            };
            {
                let s = state.borrow();
                if offset >= s.data.len() {
                    theme::message("Alert", "Offset exceeds file size");
                    return;
                }
            }
            {
                let mut s = state.borrow_mut();
                s.data[offset] = value;
                s.modified[offset] = true;
                s.dirty = true;
                let mod_count = s.modified.iter().filter(|&&m| m).count();
                status.set_label(&format!(
                    "Modified: {}  |  Total: {}",
                    mod_count,
                    fmt_size(s.data.len() as u64)
                ));
            }
            value_input.set_value("");
            let s = state.borrow();
            buf.set_text(&make_hex_dump(&s.data, &s.modified, 65536));
        }
    });

    revert_btn.set_callback({
        let state = state.clone();
        let mut buf = buf.clone();
        let mut status = status.clone();
        let path = path.clone();
        move |_| {
            let reload = match std::fs::read(&path) {
                Ok(d) => d,
                Err(e) => {
                    theme::message("Alert", &format!("Revert failed: {}", e));
                    return;
                }
            };
            {
                let mut s = state.borrow_mut();
                s.data = reload;
                s.modified.iter_mut().for_each(|m| *m = false);
                s.dirty = false;
                status.set_label(&format!(
                    "Modified: 0  |  Total: {}",
                    fmt_size(s.data.len() as u64)
                ));
            }
            let s = state.borrow();
            buf.set_text(&make_hex_dump(&s.data, &s.modified, 65536));
        }
    });

    save_btn.set_callback({
        let state = state.clone();
        let mut buf = buf.clone();
        let mut status = status.clone();
        move |_| {
            let mut s = state.borrow_mut();
            if std::fs::write(&s.path, &s.data).is_err() {
                theme::message("Alert", "Failed to save file");
                return;
            }
            s.dirty = false;
            s.modified.iter_mut().for_each(|m| *m = false);
            status.set_label(&format!(
                "Modified: 0  |  Total: {}",
                fmt_size(s.data.len() as u64)
            ));
            buf.set_text(&make_hex_dump(&s.data, &s.modified, 65536));
        }
    });

    save_as_btn.set_callback({
        let state = state.clone();
        let mut buf = buf.clone();
        let mut status = status.clone();
        move |_| {
            let new_path = match theme::save_file_chooser("Save Hex Editor Output As", "*", "", ".")
            {
                Some(p) => p,
                None => return,
            };
            {
                let s = state.borrow();
                if std::fs::write(&new_path, &s.data).is_err() {
                    theme::message("Alert", "Failed to write file");
                    return;
                }
            }
            let mut s = state.borrow_mut();
            s.path = new_path;
            s.dirty = false;
            s.modified.iter_mut().for_each(|m| *m = false);
            status.set_label(&format!(
                "Modified: 0  |  Total: {}",
                fmt_size(s.data.len() as u64)
            ));
            buf.set_text(&make_hex_dump(&s.data, &s.modified, 65536));
        }
    });

    close_btn.set_callback({
        let win_holder = win_holder.clone();
        let state = state.clone();
        let mut buf = buf.clone();
        let mut status = status.clone();
        move |_| {
            if state.borrow().dirty {
                let ret = theme::choice(
                    "Unsaved Changes",
                    "You have unsaved changes.\nSave before closing?",
                    &["Save", "Discard", "Cancel"],
                );
                match ret {
                    Some(0) => {
                        let mut s = state.borrow_mut();
                        let _ = std::fs::write(&s.path, &s.data);
                        s.dirty = false;
                        s.modified.iter_mut().for_each(|m| *m = false);
                        status.set_label(&format!(
                            "Modified: 0  |  Total: {}",
                            fmt_size(s.data.len() as u64)
                        ));
                        buf.set_text(&make_hex_dump(&s.data, &s.modified, 65536));
                    }
                    Some(1) => {}
                    _ => return,
                }
            }
            if let Some(mut w) = win_holder.borrow_mut().take() {
                w.hide();
            }
        }
    });

    win.set_callback({
        let win_holder = win_holder.clone();
        let state = state.clone();
        let mut buf = buf.clone();
        let mut status = status.clone();
        move |w| {
            if state.borrow().dirty {
                let ret = theme::choice(
                    "Unsaved Changes",
                    "You have unsaved changes.\nSave before closing?",
                    &["Save", "Discard", "Cancel"],
                );
                match ret {
                    Some(0) => {
                        let mut s = state.borrow_mut();
                        let _ = std::fs::write(&s.path, &s.data);
                        s.dirty = false;
                        s.modified.iter_mut().for_each(|m| *m = false);
                        status.set_label(&format!(
                            "Modified: 0  |  Total: {}",
                            fmt_size(s.data.len() as u64)
                        ));
                        buf.set_text(&make_hex_dump(&s.data, &s.modified, 65536));
                    }
                    Some(1) => {}
                    _ => return,
                }
            }
            w.hide();
            *win_holder.borrow_mut() = None;
        }
    });

    win.end();
    win.show();

    // Leak the local handle so the window stays alive after the function returns.
    std::mem::forget(win);
}

pub fn signature_scanner() {
    let path = match theme::file_chooser("Select File to Scan", "*", ".") {
        Some(p) => p,
        None => return,
    };
    signature_scanner_window(&path);
}

fn run_signature_scan(path: &str) -> (String, String) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            return (String::new(), format!("⚠ Cannot read file: {}", e));
        }
    };
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default()
        .to_string();
    let started = Instant::now();
    let hits = deep_scan(&data);
    let elapsed = started.elapsed();

    // Group hits by label, preserving insertion order.
    let mut groups: Vec<(&'static str, Vec<usize>)> = Vec::new();
    for h in &hits {
        if let Some((_, offs)) = groups.iter_mut().find(|(l, _)| *l == h.label) {
            offs.push(h.offset);
        } else {
            groups.push((h.label, vec![h.offset]));
        }
    }

    let mut out = String::new();
    out.push_str(&format!("Signature Scan — {}\n", name));
    out.push_str(&format!(
        "Size: {} ({} bytes)\n",
        fmt_size(data.len() as u64),
        data.len()
    ));
    out.push_str(&format!(
        "Scanned in {:.0} ms — {} signature match(es)\n",
        elapsed.as_secs_f64() * 1000.0,
        hits.len()
    ));
    out.push('\n');

    if groups.is_empty() {
        out.push_str("No known signatures found in this file.\n");
    } else {
        let primary = groups
            .iter()
            .find(|(_, offs)| offs.contains(&0))
            .map(|(l, _)| *l);
        if let Some(primary) = primary {
            out.push_str(&format!("✓ Detected format: {}\n\n", primary));
        }
        for (label, offs) in &groups {
            out.push_str(&format!("■ {} — {} hit(s):\n", label, offs.len()));
            for o in offs.iter().take(16) {
                out.push_str(&format!("    0x{:08X}\n", o));
            }
            if offs.len() > 16 {
                out.push_str(&format!("    … and {} more\n", offs.len() - 16));
            }
            out.push('\n');
        }
    }
    let info = format!(
        "{}  ·  {}  ·  {} hits  ·  {:.0} ms",
        name,
        fmt_size(data.len() as u64),
        hits.len(),
        elapsed.as_secs_f64() * 1000.0
    );
    (info, out)
}

fn signature_scanner_window(path: &str) {
    const W: i32 = 820;
    const H: i32 = 560;
    let scr = app::screen_size();
    let wx = ((scr.0 - W as f64) / 2.0).max(0.0) as i32;
    let wy = ((scr.1 - H as f64) / 2.0).max(0.0) as i32;

    let win_holder: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
    let mut win = Window::new(wx, wy, W, H, "Signature Scanner");
    theme::style_window(&mut win);
    *win_holder.borrow_mut() = Some(win.clone());

    let mut buf = TextBuffer::default();
    let (info, report) = run_signature_scan(path);
    buf.set_text(&report);

    let mut info_lbl = Frame::new(10, 8, W - 20, 24, info.as_str());
    theme::style_label(&mut info_lbl);
    info_lbl.set_align(Align::Left | Align::Inside);
    info_lbl.set_label_size(13);

    let mut display = TextDisplay::new(10, 38, W - 20, H - 100, "");
    theme::style_field(&mut display);
    display.set_buffer(buf.clone());
    display.set_text_font(Font::Courier);
    display.set_text_size(13);
    display.set_frame(FrameType::DownBox);
    win.resizable(&display);

    let mut copy_btn = Button::new(10, H - 48, 90, 30, "Copy");
    theme::style_button(&mut copy_btn);
    let mut save_btn = Button::new(110, H - 48, 110, 30, "Save As...");
    theme::style_button(&mut save_btn);
    let mut rescan_btn = Button::new(230, H - 48, 120, 30, "Scan Another");
    theme::style_button(&mut rescan_btn);
    let mut close_btn = Button::new(W - 100, H - 48, 90, 30, "Close");
    theme::style_button(&mut close_btn);

    copy_btn.set_callback({
        let buf = buf.clone();
        move |_| {
            let txt = buf.text();
            if !txt.is_empty() {
                app::copy(&txt);
            }
        }
    });

    save_btn.set_callback({
        let buf = buf.clone();
        move |_| {
            if let Some(p) =
                theme::save_file_chooser("Save Scan Report", "*.txt", "signature_scan.txt", ".")
            {
                let _ = std::fs::write(&p, buf.text().as_bytes());
            }
        }
    });

    rescan_btn.set_callback({
        let mut buf = buf.clone();
        let mut info_lbl = info_lbl.clone();
        move |_| {
            if let Some(p) = theme::file_chooser("Select File to Scan", "*", ".") {
                let (info, report) = run_signature_scan(&p);
                info_lbl.set_label(&info);
                buf.set_text(&report);
            }
        }
    });

    close_btn.set_callback({
        let wh = win_holder.clone();
        move |_| {
            if let Some(mut w) = wh.borrow_mut().take() {
                w.hide();
            }
        }
    });
    win.set_callback({
        let wh = win_holder.clone();
        move |w| {
            w.hide();
            *wh.borrow_mut() = None;
        }
    });

    win.end();
    win.show();
    std::mem::forget(win);
}

struct HeaderTemplate {
    name: &'static str,
    ext: &'static str,
    signature: &'static [u8],
    footer: &'static [u8],
    endian: &'static str,
    rows: &'static [(&'static str, &'static str, &'static str)],
}

fn hex_str(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_template(t: &HeaderTemplate) -> String {
    let w_off = t.rows.iter().map(|r| r.0.len()).max().unwrap_or(6).max(8);
    let w_size = t.rows.iter().map(|r| r.1.len()).max().unwrap_or(4).max(6);
    let w_field = t.rows.iter().map(|r| r.2.len()).max().unwrap_or(5);
    let inner_w = w_off + w_size + w_field + 8;

    let title = format!("{} Header Template", t.name);
    let info_lines: Vec<String> = {
        let mut v = Vec::new();
        v.push(format!("  Extensions  : {}", t.ext));
        v.push(format!("  Signature   : {}", hex_str(t.signature)));
        if !t.footer.is_empty() {
            v.push(format!("  Footer      : {}", hex_str(t.footer)));
        }
        v.push(format!("  Byte order  : {}", t.endian));
        v
    };
    let info_w = info_lines
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0);
    let w = inner_w.max(info_w + 2).max(title.chars().count() + 6);

    let mut out = String::new();
    let title_pad = format!("══ {} ", title);
    out.push_str(&format!("╔{}╗\n", pad_fill(&title_pad, '═', w)));
    out.push_str(&format!("║ {}║\n", pad_to(&info_lines[0], w)));
    out.push_str(&format!("║ {}║\n", pad_to(&info_lines[1], w)));
    for line in info_lines.iter().skip(2) {
        out.push_str(&format!("║ {}║\n", pad_to(line, w)));
    }
    out.push_str(&format!("╚{}\n\n", "═".repeat(w)));

    out.push_str(&format!(
        "┌─{:─<w_off$}─┬─{:─<w_size$}─┬─{:─<w_field$}─┐\n",
        "", "", ""
    ));
    out.push_str(&format!(
        "│ {:<w_off$} │ {:<w_size$} │ {:<w_field$} │\n",
        "Offset", "Size", "Field"
    ));
    out.push_str(&format!(
        "├─{:─<w_off$}─┼─{:─<w_size$}─┼─{:─<w_field$}─┤\n",
        "", "", ""
    ));
    for &(off, size, field) in t.rows {
        out.push_str(&format!(
            "│ {:<w_off$} │ {:<w_size$} │ {:<w_field$} │\n",
            off, size, field
        ));
    }
    out.push_str(&format!(
        "└─{:─<w_off$}─┴─{:─<w_size$}─┴─{:─<w_field$}─┘\n",
        "", "", ""
    ));
    out
}

fn pad_to(s: &str, w: usize) -> String {
    let mut s = s.to_string();
    let n = s.chars().count();
    if n < w {
        s.push_str(&" ".repeat(w - n));
    }
    s
}

fn pad_fill(s: &str, fill: char, w: usize) -> String {
    let mut s = s.to_string();
    let n = s.chars().count();
    if n < w {
        s.push_str(&fill.to_string().repeat(w - n));
    }
    s
}

const HEADER_TEMPLATES: &[HeaderTemplate] = &[
    HeaderTemplate {
        name: "JPEG",
        ext: ".jpg .jpeg .jfif",
        signature: &[0xFF, 0xD8, 0xFF],
        footer: &[0xFF, 0xD9],
        endian: "Big-endian",
        rows: &[
            ("0x0000", "2", "SOI marker  (FF D8)"),
            ("0x0002", "2", "APP0 marker (FF E0)"),
            ("0x0004", "2", "Segment length"),
            ("0x0006", "5", "Identifier 'JFIF\\0'"),
            ("0x000B", "2", "Version (major.minor)"),
            ("0x000D", "1", "Units (0=none,1=dpi,2=dpcm)"),
            ("0x000E", "2", "X density"),
            ("0x0010", "2", "Y density"),
            ("0x0012", "2", "Thumbnail X/Y"),
            ("0x0014", "…", "DQT, SOF0, DHT, SOS segments"),
            ("…", "…", "Compressed scan data"),
            ("End", "2", "EOI marker  (FF D9)"),
        ],
    },
    HeaderTemplate {
        name: "PNG",
        ext: ".png",
        signature: b"\x89PNG\r\n\x1a\n",
        footer: &[],
        endian: "Big-endian",
        rows: &[
            ("0x0000", "8", "PNG signature 89 50 4E 47 0D 0A 1A 0A"),
            ("0x0008", "4", "First chunk length (13)"),
            ("0x000C", "4", "Chunk type 'IHDR'"),
            ("0x0010", "4", "Width"),
            ("0x0014", "4", "Height"),
            ("0x0018", "1", "Bit depth"),
            ("0x0019", "1", "Color type"),
            ("0x001A", "1", "Compression method"),
            ("0x001B", "1", "Filter method"),
            ("0x001C", "1", "Interlace method"),
            ("0x001D", "4", "CRC-32 of IHDR"),
            ("0x0021", "…", "PLTE / tRNS / IDAT chunks"),
            ("…", "…", "Compressed image data"),
            ("End", "12", "IEND chunk (len 0 + 'IEND' + CRC)"),
        ],
    },
    HeaderTemplate {
        name: "GIF",
        ext: ".gif",
        signature: b"GIF8",
        footer: &[0x3B],
        endian: "Little-endian",
        rows: &[
            ("0x0000", "3", "Signature 'GIF'"),
            ("0x0003", "3", "Version '87a' or '89a'"),
            ("0x0006", "2", "Logical screen width"),
            ("0x0008", "2", "Logical screen height"),
            ("0x000A", "1", "Packed fields (GCT flag/size)"),
            ("0x000B", "1", "Background color index"),
            ("0x000C", "1", "Pixel aspect ratio"),
            ("0x000D", "…", "Global Color Table (optional)"),
            ("0x000D", "…", "Image descriptors / extensions"),
            ("End", "1", "Trailer byte  (0x3B)"),
        ],
    },
    HeaderTemplate {
        name: "PDF",
        ext: ".pdf",
        signature: b"%PDF",
        footer: b"%%EOF",
        endian: "n/a",
        rows: &[
            ("0x0000", "5", "Header '%PDF-1.x'"),
            ("0x0005", "…", "Body (objects, streams)"),
            ("…", "…", "Cross-reference table"),
            ("…", "…", "Trailer dictionary"),
            ("…", "…", "startxref offset"),
            ("End", "5", "Indicator '%%EOF'"),
        ],
    },
    HeaderTemplate {
        name: "ZIP",
        ext: ".zip .jar .odt .docx",
        signature: &[0x50, 0x4B, 0x03, 0x04],
        footer: &[],
        endian: "Little-endian",
        rows: &[
            ("0x0000", "4", "Local file header sig 'PK\\x03\\x04'"),
            ("0x0004", "2", "Version needed"),
            ("0x0006", "2", "General purpose bit flag"),
            ("0x0008", "2", "Compression method"),
            ("0x000A", "2", "Last mod time"),
            ("0x000C", "2", "Last mod date"),
            ("0x000E", "4", "CRC-32"),
            ("0x0012", "4", "Compressed size"),
            ("0x0016", "4", "Uncompressed size"),
            ("0x001A", "2", "File name length"),
            ("0x001C", "2", "Extra field length"),
            ("0x001E", "…", "File name + extra field"),
        ],
    },
    HeaderTemplate {
        name: "BMP",
        ext: ".bmp .dib",
        signature: b"BM",
        footer: &[],
        endian: "Little-endian",
        rows: &[
            ("0x0000", "2", "Signature 'BM'"),
            ("0x0002", "4", "File size"),
            ("0x0006", "4", "Reserved"),
            ("0x000A", "4", "Pixel data offset"),
            ("0x000E", "4", "DIB header size (40)"),
            ("0x0012", "4", "Width (signed)"),
            ("0x0016", "4", "Height (signed)"),
            ("0x001A", "2", "Color planes (1)"),
            ("0x001C", "2", "Bits per pixel"),
            ("0x001E", "4", "Compression (BI_RGB…)"),
            ("0x0022", "4", "Image size / 0"),
            ("0x0026", "4", "X pixels per meter"),
            ("0x002A", "4", "Y pixels per meter"),
            ("0x002E", "4", "Colors used"),
            ("0x0032", "4", "Important colors"),
            ("0x0036", "…", "Pixel array (BGR)"),
        ],
    },
    HeaderTemplate {
        name: "RIFF",
        ext: ".wav .avi .webp",
        signature: b"RIFF",
        footer: &[],
        endian: "Little-endian",
        rows: &[
            ("0x0000", "4", "Signature 'RIFF'"),
            ("0x0004", "4", "Chunk size (file size − 8)"),
            ("0x0008", "4", "Form type 'WAVE' / 'AVI '"),
            ("0x000C", "4", "Chunk ID"),
            ("0x0010", "4", "Chunk size"),
            ("0x0014", "…", "Chunk data (fmt/data for WAV)"),
            ("…", "…", "More chunks / LIST, movi for AVI"),
        ],
    },
    HeaderTemplate {
        name: "MP3",
        ext: ".mp3",
        signature: b"ID3",
        footer: &[],
        endian: "Big-endian",
        rows: &[
            ("0x0000", "3", "Tag identifier 'ID3'"),
            ("0x0003", "2", "Version (e.g. 03 00)"),
            ("0x0005", "1", "Flags (unsync/extended/exp)"),
            ("0x0006", "4", "Tag size (synchsafe)"),
            ("0x000A", "…", "Frames: ID + size + flags + data"),
            ("…", "4", "Audio frame sync 'FF FB' / 'FF F3'"),
            ("…", "2", "Frame header: version/layer/bitrate"),
            ("…", "…", "Side info + main data"),
        ],
    },
    HeaderTemplate {
        name: "FLAC",
        ext: ".flac",
        signature: b"fLaC",
        footer: &[],
        endian: "Big-endian",
        rows: &[
            ("0x0000", "4", "Signal 'fLaC'"),
            ("0x0004", "1", "Metadata block header (type+last)"),
            ("0x0005", "3", "Metadata block length"),
            ("0x0008", "34", "STREAMINFO block"),
            ("0x002A", "…", "More blocks (VORBIS_COMMENT…)"),
            ("…", "…", "Audio frames (sync 'FF F8')"),
        ],
    },
    HeaderTemplate {
        name: "TIFF",
        ext: ".tif .tiff",
        signature: b"II\x2a\x00",
        footer: &[],
        endian: "II=LE / MM=BE",
        rows: &[
            ("0x0000", "2", "Byte order 'II' (LE) / 'MM' (BE)"),
            ("0x0002", "2", "Version 42 (0x2A)"),
            ("0x0004", "4", "Offset to first IFD"),
            ("0x0008", "2", "Number of IFD entries"),
            ("0x000A", "12·n", "IFD entries (tag,type,count,value)"),
            ("…", "4", "Offset to next IFD / 0"),
        ],
    },
    HeaderTemplate {
        name: "ELF",
        ext: ".elf .o .so",
        signature: &[0x7F, 0x45, 0x4C, 0x46],
        footer: &[],
        endian: "Class-dependent",
        rows: &[
            ("0x0000", "4", "Magic 7F 'E' 'L' 'F'"),
            ("0x0004", "1", "Class (1=32-bit, 2=64-bit)"),
            ("0x0005", "1", "Data encoding (1=LE, 2=BE)"),
            ("0x0006", "1", "Version (1)"),
            ("0x0007", "1", "OS/ABI"),
            ("0x0008", "8", "Padding"),
            ("0x0010", "2", "Type (REL/EXEC/DYN)"),
            ("0x0012", "2", "Machine"),
            ("0x0014", "4", "Version"),
            ("0x0018", "4/8", "Entry point"),
            ("0x001C", "4/8", "Program header offset"),
            ("0x0020", "4/8", "Section header offset"),
            ("0x0028", "4", "Flags"),
            ("0x002C", "2", "ELF header size (52/64)"),
            ("0x002E", "2", "Program header entry size"),
            ("0x0030", "2", "Program header count"),
            ("0x0032", "2", "Section header entry size"),
            ("0x0034", "2", "Section header count"),
            ("0x0036", "2", "String table index"),
        ],
    },
    HeaderTemplate {
        name: "EXE",
        ext: ".exe .dll",
        signature: b"MZ",
        footer: &[],
        endian: "Little-endian",
        rows: &[
            ("0x0000", "2", "DOS magic 'MZ'"),
            ("0x0002", "…", "DOS header / stub"),
            ("0x003C", "4", "e_lfanew → PE header offset"),
            ("e_lfanew", "4", "PE signature 'PE\\x00\\x00'"),
            ("+0x0004", "2", "Machine"),
            ("+0x0006", "2", "Number of sections"),
            ("+0x0008", "4", "TimeDateStamp"),
            ("+0x000C", "4", "Pointer to symbol table"),
            ("+0x0010", "4", "Number of symbols"),
            ("+0x0014", "2", "Size of optional header"),
            ("+0x0016", "2", "Characteristics"),
            ("+0x0018", "…", "Optional header (magic 0x10B/0x20B)"),
            ("…", "…", "Section table + raw data"),
        ],
    },
    HeaderTemplate {
        name: "WebM",
        ext: ".webm .mkv",
        signature: &[0x1A, 0x45, 0xDF, 0xA3],
        footer: &[],
        endian: "VINT",
        rows: &[
            ("0x0000", "4", "EBML element ID 1A 45 DF A3"),
            ("0x0004", "…", "EBML header (version, DocType)"),
            ("…", "…", "Segment element 18 53 80 67"),
            ("…", "…", "Info / Tracks / Cluster blocks"),
        ],
    },
    HeaderTemplate {
        name: "Ogg",
        ext: ".ogg .oga .ogv",
        signature: b"OggS",
        footer: &[],
        endian: "Little-endian",
        rows: &[
            ("0x0000", "4", "Capture pattern 'OggS'"),
            ("0x0004", "1", "Version (0)"),
            ("0x0005", "1", "Header type (BOS/EOS/cont)"),
            ("0x0006", "8", "Granule position"),
            ("0x000E", "4", "Bitstream serial"),
            ("0x0012", "4", "Page sequence"),
            ("0x0016", "4", "CRC-32"),
            ("0x001A", "1", "Segment count"),
            ("0x001B", "n", "Segment table"),
            ("…", "…", "Segment data (packets)"),
        ],
    },
    HeaderTemplate {
        name: "Java Class",
        ext: ".class",
        signature: &[0xCA, 0xFE, 0xBA, 0xBE],
        footer: &[],
        endian: "Big-endian",
        rows: &[
            ("0x0000", "4", "Magic 0xCAFEBABE"),
            ("0x0004", "2", "Minor version"),
            ("0x0006", "2", "Major version (52=Java 8)"),
            ("0x0008", "2", "Constant pool count"),
            ("0x000A", "…", "Constant pool entries"),
            ("…", "2", "Access flags"),
            ("…", "2", "This class (cp index)"),
            ("…", "2", "Super class (cp index)"),
            ("…", "2", "Interfaces count"),
            ("…", "…", "Interfaces / fields / methods"),
        ],
    },
];

pub fn header_template() {
    const W: i32 = 860;
    const H: i32 = 620;
    let scr = app::screen_size();
    let wx = ((scr.0 - W as f64) / 2.0).max(0.0) as i32;
    let wy = ((scr.1 - H as f64) / 2.0).max(0.0) as i32;

    let win_holder: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
    let mut win = Window::new(wx, wy, W, H, "Header Template Generator");
    theme::style_window(&mut win);
    *win_holder.borrow_mut() = Some(win.clone());

    let list_w = 170;
    let mut list = HoldBrowser::new(10, 10, list_w, H - 70, "File Format");
    for t in HEADER_TEMPLATES {
        list.add(t.name);
    }
    list.set_text_size(13);
    list.set_frame(FrameType::DownBox);
    list.set_color(theme::field_color());
    list.set_label_color(theme::muted_color());

    let buf = TextBuffer::default();
    let mut display = TextDisplay::new(list_w + 20, 10, W - list_w - 30, H - 70, "");
    theme::style_field(&mut display);
    display.set_buffer(buf.clone());
    display.set_text_font(Font::Courier);
    display.set_text_size(13);
    display.set_frame(FrameType::DownBox);
    win.resizable(&display);

    let load = Rc::new(RefCell::new(Box::new(|_: i32| {}) as Box<dyn FnMut(i32)>));
    *load.borrow_mut() = Box::new({
        let mut buf = buf.clone();
        move |idx: i32| {
            if idx >= 1 && idx as usize <= HEADER_TEMPLATES.len() {
                buf.set_text(&render_template(&HEADER_TEMPLATES[(idx - 1) as usize]));
            }
        }
    });

    {
        let load = load.clone();
        list.set_callback(move |list| {
            let idx = list.value();
            (load.borrow_mut())(idx);
        });
    }

    let mut copy_btn = Button::new(10, H - 48, 120, 30, "Copy Template");
    theme::style_button(&mut copy_btn);
    let mut close_btn = Button::new(W - 100, H - 48, 90, 30, "Close");
    theme::style_button(&mut close_btn);

    copy_btn.set_callback({
        let buf = buf.clone();
        move |_| {
            let txt = buf.text();
            if !txt.is_empty() {
                app::copy(&txt);
            }
        }
    });

    close_btn.set_callback({
        let wh = win_holder.clone();
        move |_| {
            if let Some(mut w) = wh.borrow_mut().take() {
                w.hide();
            }
        }
    });
    win.set_callback({
        let wh = win_holder.clone();
        move |w| {
            w.hide();
            *wh.borrow_mut() = None;
        }
    });

    win.end();
    list.select(1);
    (load.borrow_mut())(1);
    win.show();
    std::mem::forget(win);
}

pub fn recovery_log() {
    let candidates = [
        "/tmp/repair_output/repair.log",
        "/tmp/recovery_log.txt",
        "/tmp/kaifuku_recovery.log",
    ];
    let mut content = String::new();
    for path in &candidates {
        if Path::new(path).exists() {
            match std::fs::read_to_string(path) {
                Ok(s) => {
                    content = s;
                    break;
                }
                Err(_) => continue,
            }
        }
    }
    if content.is_empty() {
        if let Ok(entries) = std::fs::read_dir("/tmp") {
            let mut logs: Vec<String> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("recovery")
                    || name.contains("photorec")
                    || name.contains("kaifuku")
                {
                    if let Ok(m) = entry.metadata() {
                        if m.is_file() && m.len() < 1_000_000 {
                            if let Ok(d) = std::fs::read_to_string(entry.path()) {
                                logs.push(format!("── {} ──\n{}", name, d));
                            }
                        }
                    }
                }
            }
            if !logs.is_empty() {
                content = logs.join("\n");
            }
        }
    }
    if content.is_empty() {
        theme::message("Recovery Log", "No recovery log files found.\n\nRecovery logs are created during the scan process and saved to the temporary or output directory.");
    } else {
        let truncated = if content.len() > 5000 {
            let t = content[..5000].to_string();
            t + "\n\n... (truncated)"
        } else {
            content
        };
        theme::message("Recovery Log", &truncated);
    }
}

struct DdProgress {
    bytes: u64,
    total: u64,
}

enum DdResult {
    InProgress(DdProgress),
    Finished { success: bool, msg: String },
}

fn check_dd_result(
    state: Arc<Mutex<Option<DdResult>>>,
    mut buf: TextBuffer,
    mut btn: Button,
    mut stop_btn: Button,
    mut close_btn: Button,
    handle: *mut (),
) {
    let mut guard = state.lock().unwrap();
    match guard.take() {
        Some(DdResult::Finished { success, msg }) => {
            buf.append(&format!("{} {}\n", if success { "✅" } else { "❌" }, msg));
            btn.show();
            stop_btn.hide();
            close_btn.show();
            btn.set_label("Write");
            btn.activate();
        }
        Some(DdResult::InProgress(p)) => {
            let pct = if p.total > 0 {
                p.bytes * 100 / p.total
            } else {
                0
            };
            let written = fmt_size(p.bytes);
            let total = fmt_size(p.total);
            buf.append(&format!("Progress: {}% ({} / {})\n", pct, written, total));
            *guard = Some(DdResult::InProgress(p));
            app::repeat_timeout3(0.3, handle);
        }
        None => {
            app::repeat_timeout3(0.3, handle);
        }
    }
}

fn parse_dd_bytes(line: &str) -> Option<u64> {
    let line = line.trim();
    let end = line.find(" bytes ")?;
    line[..end].parse::<u64>().ok()
}

pub fn live_usb_creator() {
    const W: i32 = 640;
    const H: i32 = 490;
    let scr = app::screen_size();
    let wx = ((scr.0 - W as f64) / 2.0) as i32;
    let wy = ((scr.1 - H as f64) / 2.0) as i32;

    let win_holder: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
    let mut win = Window::new(wx.max(0), wy.max(0), W, H, "Live USB Creator");
    theme::style_window(&mut win);
    *win_holder.borrow_mut() = Some(win.clone());

    let can_access = has_device_access();
    let y0 = if can_access { 10 } else { 50 };

    if !can_access {
        let mut banner = Frame::new(
            10,
            10,
            W - 20,
            30,
            "⚠ Root privileges required — run with sudo or pkexec",
        );
        banner.set_label_color(Color::from_rgb(255, 200, 50));
        banner.set_frame(FrameType::DownBox);
        banner.set_color(Color::from_rgb(60, 40, 10));
        banner.set_label_size(13);
    }

    // ISO row
    let mut iso_label = Frame::new(10, y0, 80, 30, "ISO Image:");
    theme::style_label(&mut iso_label);
    iso_label.set_align(Align::Left | Align::Inside);
    let mut iso_input = Input::new(95, y0, W - 210, 30, "");
    theme::style_input(&mut iso_input);
    let mut browse_btn = Button::new(W - 105, y0, 95, 30, "Browse");
    theme::style_muted_button(&mut browse_btn);
    browse_btn.set_callback({
        let mut i = iso_input.clone();
        move |_| {
            if let Some(p) = theme::file_chooser("Select ISO Image", "*.iso", ".") {
                i.set_value(&p);
            }
        }
    });

    // Target USB row
    let y1 = y0 + 50;
    let mut target_label = Frame::new(10, y1, 90, 30, "Target USB:");
    theme::style_label(&mut target_label);
    target_label.set_align(Align::Left | Align::Inside);
    let mut usb_choice = Choice::new(105, y1, W - 120, 30, "");
    theme::style_choice(&mut usb_choice);
    let disk_infos = Rc::new(RefCell::new(Vec::<DiskInfo>::new()));
    for disk in enumerate_disks() {
        if !disk.removable {
            continue;
        }
        let label = format!(
            "{} — {} ({})",
            disk.device,
            fmt_size(disk.capacity),
            disk.model
        );
        usb_choice.add(&label, Shortcut::None, MenuFlag::Normal, |_| {});
        disk_infos.borrow_mut().push(disk);
    }
    if usb_choice.size() > 0 {
        usb_choice.set_value(0);
    }

    // Partition scheme
    let y2 = y1 + 50;
    let mut scheme_label = Frame::new(10, y2, 150, 30, "Partition Scheme:");
    theme::style_label(&mut scheme_label);
    scheme_label.set_align(Align::Left | Align::Inside);
    let mut gpt_radio = RadioRoundButton::new(30, y2 + 30, 220, 25, "GPT (UEFI)");
    gpt_radio.set_value(true);
    theme::style_label(&mut gpt_radio);
    let mut mbr_radio = RadioRoundButton::new(260, y2 + 30, 220, 25, "MBR (Legacy BIOS)");
    theme::style_label(&mut mbr_radio);

    // Status log
    let y3 = y2 + 70;
    let log_h = H - y3 - 70;
    let mut status_buf = TextBuffer::default();
    status_buf.set_text("Ready. Select an ISO image and target USB drive, then click Write.\n");
    let mut status_log = TextDisplay::new(10, y3, W - 20, log_h, "");
    theme::style_field(&mut status_log);
    status_log.set_buffer(status_buf.clone());
    status_log.set_text_font(Font::Courier);
    status_log.set_text_size(12);

    // Bottom buttons
    let y4 = H - 50;
    let mut write_btn = Button::new(W - 220, y4, 100, 35, "Write");
    theme::style_button(&mut write_btn);
    if !can_access {
        write_btn.deactivate();
    }
    let mut stop_btn = Button::new(W - 220, y4, 100, 35, "Stop");
    theme::style_button(&mut stop_btn);
    stop_btn.hide();
    let mut close_btn = Button::new(W - 110, y4, 100, 35, "Close");
    theme::style_button(&mut close_btn);

    // Write callback
    write_btn.set_callback({
        let i = iso_input.clone();
        let choice = usb_choice.clone();
        let infos = disk_infos.clone();
        let gpt = gpt_radio.clone();
        let mut buf = status_buf.clone();
        let mut btn = write_btn.clone();
        let mut sbtn = stop_btn.clone();
        let mut cbtn = close_btn.clone();
        move |_| {
            let iso = i.value();
            if iso.is_empty() || !Path::new(&iso).exists() {
                theme::message("Alert", "Please select a valid ISO file");
                return;
            }
            let idx = choice.value() as usize;
            let disks = infos.borrow();
            if idx >= disks.len() {
                theme::message("Alert", "Please select a target USB drive");
                return;
            }
            let disk = disks[idx].clone();
            drop(disks);
            let target = disk.device.clone();
            let scheme = if gpt.value() { "GPT" } else { "MBR" };

            let iso_name = Path::new(&iso)
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            let warn = format!(
                "⚠ DESTRUCTIVE OPERATION ⚠\n\n\
                 Device:  {} — {}\n\
                 Size:    {}\n\n\
                 Partition scheme: {}\n\
                 ISO:     {}\n\n\
                 All data on this device will be PERMANENTLY DESTROYED.\n\
                 Continue?",
                disk.device,
                disk.model,
                fmt_size(disk.capacity),
                scheme,
                iso_name,
            );
            let ret = theme::choice("Confirm Write", &warn, &["Write", "Cancel"]);
            if ret != Some(0) {
                return;
            }

            buf.append(&format!(
                "== Writing ISO to {} [{}] ==\nSource: {}\nTarget: {}\n",
                target, scheme, iso, target
            ));

            btn.hide();
            sbtn.show();
            cbtn.hide();

            let buf_c = buf.clone();
            let btn_c = btn.clone();
            let sbtn_c = sbtn.clone();
            let cbtn_c = cbtn.clone();
            let target_c = target.clone();
            let iso_c = iso.clone();
            let dd_state: Arc<Mutex<Option<DdResult>>> = Arc::new(Mutex::new(None));
            let dd_child: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));

            // Wire stop button to kill dd
            sbtn.set_callback({
                let dd_c = dd_child.clone();
                let dd_st = dd_state.clone();
                let mut buf2 = buf_c.clone();
                let mut btn2 = btn_c.clone();
                let mut sbtn2 = sbtn_c.clone();
                let mut cbtn2 = cbtn_c.clone();
                move |_| {
                    if let Ok(mut guard) = dd_c.lock() {
                        if let Some(ref mut child) = *guard {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    }
                    *dd_st.lock().unwrap() = Some(DdResult::Finished {
                        success: false,
                        msg: "Cancelled by user.".to_string(),
                    });
                    buf2.append("❌ Cancelled by user.\n");
                    btn2.show();
                    sbtn2.hide();
                    cbtn2.show();
                    app::awake();
                }
            });

            app::add_timeout3(0.3, {
                let dd_s = dd_state.clone();
                let buf_c = buf_c.clone();
                let btn_c = btn_c.clone();
                let sbtn_c = sbtn_c.clone();
                let cbtn_c = cbtn_c.clone();
                move |h| {
                    check_dd_result(
                        dd_s.clone(),
                        buf_c.clone(),
                        btn_c.clone(),
                        sbtn_c.clone(),
                        cbtn_c.clone(),
                        h,
                    );
                }
            });

            std::thread::spawn(move || {
                let total = std::fs::metadata(&iso_c).ok().map(|m| m.len()).unwrap_or(0);
                let child = match std::process::Command::new("dd")
                    .arg(format!("if={}", iso_c))
                    .arg(format!("of={}", target_c))
                    .arg("bs=4M")
                    .arg("status=progress")
                    .arg("conv=fsync")
                    .stderr(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(c) => c,
                    Err(e) => {
                        *dd_state.lock().unwrap() = Some(DdResult::Finished {
                            success: false,
                            msg: format!("Failed to start dd: {}\nTry running the app as root.", e),
                        });
                        app::awake();
                        return;
                    }
                };

                *dd_child.lock().unwrap() = Some(child);
                let mut child = dd_child.lock().unwrap().take().unwrap();

                let stderr = child.stderr.take().unwrap();
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(l) = line {
                        if let Some(bytes) = parse_dd_bytes(&l) {
                            *dd_state.lock().unwrap() =
                                Some(DdResult::InProgress(DdProgress { bytes, total }));
                            app::awake();
                        }
                    }
                }

                let status = child.wait();
                match status {
                    Ok(s) if s.success() => {
                        *dd_state.lock().unwrap() = Some(DdResult::Finished {
                            success: true,
                            msg: "Write complete! You can safely remove the USB drive.".to_string(),
                        });
                    }
                    Ok(s) => {
                        let code = s
                            .code()
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        *dd_state.lock().unwrap() = Some(DdResult::Finished {
                            success: false,
                            msg: format!("dd failed with exit code {}", code),
                        });
                    }
                    Err(e) => {
                        *dd_state.lock().unwrap() = Some(DdResult::Finished {
                            success: false,
                            msg: format!("Failed to run dd: {}\nTry running the app as root.", e),
                        });
                    }
                }
                app::awake();
            });
        }
    });

    // Stop button — will be re-wired per-write above
    // Close button
    close_btn.set_callback({
        let wh = win_holder.clone();
        move |_| {
            if let Some(mut w) = wh.borrow_mut().take() {
                w.hide();
            }
        }
    });

    // Window X
    win.set_callback({
        let wh = win_holder.clone();
        move |w| {
            w.hide();
            *wh.borrow_mut() = None;
        }
    });

    win.end();
    win.show();
    std::mem::forget(win);
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
