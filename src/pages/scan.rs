use crate::app::Page;
use crate::backend::filetypes::{CategoryMask, ALL_CATEGORIES};
use crate::backend::{PhotoRecOptions, WorkerCommand};
use crate::theme::{self, Theme};
use crate::util::config::Config;
use crate::util::disks::{enumerate_disks, enumerate_partitions};
use crate::util::perms::has_device_access;
use crate::widgets::{create_primary_button, create_secondary_button, create_title};
use crossbeam_channel::Sender;
use fltk::{
    button::CheckButton, enums::Color, frame::Frame, group::Group, menu::Choice, menu::MenuFlag,
    prelude::*,
};
use std::cell::RefCell;
use std::rc::Rc;

pub struct ScanPage;

impl ScanPage {
    pub fn new(
        win_w: i32,
        win_h: i32,
        nav_tx: Sender<Page>,
        cmd_tx: Sender<WorkerCommand>,
    ) -> Self {
        let theme = Theme::global();
        let can_access = has_device_access();
        let config = Config::load(&Config::default_path()).unwrap_or_default();
        let default_dir = config.default_recovery_folder.clone();

        let mut page = Group::new(0, 0, win_w, win_h, "");

        let title_w = 200;
        let title_h = 40;
        let title = Rc::new(RefCell::new(create_title(
            0,
            0,
            title_w,
            title_h,
            "Scan & Recover",
        )));

        let banner = Rc::new(RefCell::new(Frame::new(0, 0, 0, 0, "")));
        if !can_access {
            let mut b = banner.borrow_mut();
            b.set_label("⚠ Root privileges required — run with sudo or pkexec");
            b.set_label_color(Color::from_rgb(255, 200, 50));
            b.set_label_size(theme.font_size - 1);
            b.set_frame(fltk::enums::FrameType::DownBox);
            b.set_color(Color::from_rgb(60, 40, 10));
        }

        let disks = enumerate_disks();
        let disk_names: Vec<String> = disks
            .iter()
            .map(|d| {
                format!(
                    "{} ({})",
                    d.device,
                    if d.model.is_empty() {
                        "Unknown"
                    } else {
                        &d.model
                    }
                )
            })
            .collect();

        let start_y = if can_access { 80 } else { 110 };
        let label_w = 150;
        let field_w = 300;
        let field_h = 35;
        let spacing = 50;

        let disk_label = Rc::new(RefCell::new(Frame::new(
            0,
            0,
            label_w,
            30,
            "Physical Disk:",
        )));
        disk_label.borrow_mut().set_label_color(theme.text);
        disk_label.borrow_mut().set_label_size(theme.font_size);

        let disk_choice = Rc::new(RefCell::new({
            let mut c = Choice::new(0, 0, field_w, field_h, "");
            c.set_color(Color::from_rgb(30, 30, 30));
            c.set_text_color(theme.text);
            c.set_tooltip("Physical storage device (block device) to scan");
            c
        }));

        let part_label = Rc::new(RefCell::new(Frame::new(0, 0, label_w, 30, "Partition:")));
        part_label.borrow_mut().set_label_color(theme.text);
        part_label.borrow_mut().set_label_size(theme.font_size);

        let part_choice = Rc::new(RefCell::new({
            let mut c = Choice::new(0, 0, field_w, field_h, "");
            c.set_color(Color::from_rgb(30, 30, 30));
            c.set_text_color(theme.text);
            c.set_tooltip(
                "Partition to scan. 'Whole Disk' scans everything;\n\
                 picking a single partition makes the filesystem-structure\n\
                 pass available on FAT/NTFS volumes.",
            );
            c
        }));

        let partitions = Rc::new(RefCell::new(Vec::new()));

        let info_label = Rc::new(RefCell::new(Frame::new(
            0,
            0,
            200,
            30,
            "Device Information:",
        )));
        info_label.borrow_mut().set_label_color(theme.text);
        info_label.borrow_mut().set_label_size(theme.font_size);

        let info_dev = Rc::new(RefCell::new(Frame::new(0, 0, 500, 25, "")));
        info_dev.borrow_mut().set_label_color(theme.text);
        info_dev.borrow_mut().set_label_size(theme.font_size - 2);

        let info_cap = Rc::new(RefCell::new(Frame::new(0, 0, 500, 25, "")));
        info_cap.borrow_mut().set_label_color(theme.text);
        info_cap.borrow_mut().set_label_size(theme.font_size - 2);

        let info_sec = Rc::new(RefCell::new(Frame::new(0, 0, 500, 25, "")));
        info_sec.borrow_mut().set_label_color(theme.text);
        info_sec.borrow_mut().set_label_size(theme.font_size - 2);

        let info_model = Rc::new(RefCell::new(Frame::new(0, 0, 500, 25, "")));
        info_model.borrow_mut().set_label_color(theme.text);
        info_model.borrow_mut().set_label_size(theme.font_size - 2);

        {
            let p2 = part_choice.clone();
            let pts2 = partitions.clone();
            let d2 = info_dev.clone();
            let c2 = info_cap.clone();
            let s2 = info_sec.clone();
            let m2 = info_model.clone();
            for (i, disk) in disks.iter().enumerate() {
                let dev_path = disk.device.clone();
                let cap = disk.capacity;
                let sector = disk.sector_size;
                let model = disk.model.clone();
                let p3 = p2.clone();
                let pts3 = pts2.clone();
                let dev2 = dev_path.clone();
                let d3 = d2.clone();
                let c3 = c2.clone();
                let s3 = s2.clone();
                let m3 = m2.clone();
                disk_choice.borrow_mut().add(
                    disk_names[i].as_str(),
                    fltk::enums::Shortcut::None,
                    MenuFlag::Normal,
                    move |_| {
                        let list = enumerate_partitions(&dev2);
                        *pts3.borrow_mut() = list.clone();
                        p3.borrow_mut().clear();
                        if list.is_empty() {
                            p3.borrow_mut().add(
                                "(Whole Device — no partitions)",
                                fltk::enums::Shortcut::None,
                                MenuFlag::Normal,
                                |_| {},
                            );
                        } else {
                            for p in &list {
                                let label = partition_label(p);
                                p3.borrow_mut().add(
                                    label.as_str(),
                                    fltk::enums::Shortcut::None,
                                    MenuFlag::Normal,
                                    |_| {},
                                );
                            }
                        }
                        p3.borrow_mut().set_value(0);
                        d3.borrow_mut().set_label(&format!("Device: {}", dev2));
                        c3.borrow_mut()
                            .set_label(&format!("Capacity: {} GB", cap / 1_000_000_000));
                        s3.borrow_mut()
                            .set_label(&format!("Sector Size: {} bytes", sector));
                        m3.borrow_mut().set_label(&format!("Model: {}", model));
                    },
                );
            }
            if !disk_names.is_empty() {
                disk_choice.borrow_mut().set_value(0);
            }
        }

        if let Some(d) = disks.first() {
            let list = enumerate_partitions(&d.device);
            *partitions.borrow_mut() = list.clone();
            let mut pc = part_choice.borrow_mut();
            pc.clear();
            if list.is_empty() {
                pc.add(
                    "(Whole Device — no partitions)",
                    fltk::enums::Shortcut::None,
                    MenuFlag::Normal,
                    |_| {},
                );
            } else {
                for p in &list {
                    let label = partition_label(p);
                    pc.add(
                        label.as_str(),
                        fltk::enums::Shortcut::None,
                        MenuFlag::Normal,
                        |_| {},
                    );
                }
            }
            pc.set_value(0);
            info_dev
                .borrow_mut()
                .set_label(&format!("Device: {}", d.device));
            info_cap
                .borrow_mut()
                .set_label(&format!("Capacity: {} GB", d.capacity / 1_000_000_000));
            info_sec
                .borrow_mut()
                .set_label(&format!("Sector Size: {} bytes", d.sector_size));
            info_model
                .borrow_mut()
                .set_label(&format!("Model: {}", d.model));
        } else {
            info_dev
                .borrow_mut()
                .set_label("No disks found — run as root?");
        }

        let fs_label = Rc::new(RefCell::new(Frame::new(0, 0, label_w, 30, "Filesystem:")));
        fs_label.borrow_mut().set_label_color(theme.text);
        fs_label.borrow_mut().set_label_size(theme.font_size);

        let fs_choice = Rc::new(RefCell::new({
            let mut c = Choice::new(0, 0, field_w, field_h, "");
            c.add(
                "Auto",
                fltk::enums::Shortcut::None,
                MenuFlag::Normal,
                |_| {},
            );
            c.add("FAT", fltk::enums::Shortcut::None, MenuFlag::Normal, |_| {});
            c.add(
                "NTFS",
                fltk::enums::Shortcut::None,
                MenuFlag::Normal,
                |_| {},
            );
            c.add(
                "EXT2/EXT3/EXT4",
                fltk::enums::Shortcut::None,
                MenuFlag::Normal,
                |_| {},
            );
            c.add(
                "HFS+",
                fltk::enums::Shortcut::None,
                MenuFlag::Normal,
                |_| {},
            );
            c.set_value(0);
            c.set_color(Color::from_rgb(30, 30, 30));
            c.set_text_color(theme.text);
            c.set_tooltip(
                "Filesystem hint for the carve. 'Auto' detects it from the\n\
                 boot sector; the structure pass needs FAT or NTFS.",
            );
            c
        }));

        let scan_type_label = Rc::new(RefCell::new(Frame::new(0, 0, label_w, 30, "Scan Type:")));
        scan_type_label.borrow_mut().set_label_color(theme.text);
        scan_type_label.borrow_mut().set_label_size(theme.font_size);

        let scan_type_state = Rc::new(RefCell::new(String::from("Deep")));
        let scan_type_choice = Rc::new(RefCell::new({
            let mut c = Choice::new(0, 0, field_w, field_h, "");
            let st = scan_type_state.clone();
            c.add(
                "Deep",
                fltk::enums::Shortcut::None,
                MenuFlag::Normal,
                move |_| {
                    *st.borrow_mut() = "Deep".to_string();
                },
            );
            let st = scan_type_state.clone();
            c.add(
                "Quick",
                fltk::enums::Shortcut::None,
                MenuFlag::Normal,
                move |_| {
                    *st.borrow_mut() = "Quick".to_string();
                },
            );
            c.set_value(0);
            c.set_color(Color::from_rgb(30, 30, 30));
            c.set_text_color(theme.text);
            c.set_tooltip(
                "Deep scans the whole space and is slower but more thorough;\n\
                 Quick stops earlier on well-formed files.",
            );
            c
        }));

        let dest_label = Rc::new(RefCell::new(Frame::new(0, 0, label_w, 30, "Destination:")));
        dest_label.borrow_mut().set_label_color(theme.text);
        dest_label.borrow_mut().set_label_size(theme.font_size);

        let dest_input = Rc::new(RefCell::new({
            let mut i = fltk::input::Input::new(0, 0, field_w - 110, field_h, "");
            i.set_value(&default_dir);
            i.set_color(Color::from_rgb(30, 30, 30));
            i.set_text_color(theme.text);
            i.set_tooltip("Folder where recovered files are written");
            i
        }));

        let dest_browse = Rc::new(RefCell::new({
            let dest_input_cb = dest_input.clone();
            let mut b = fltk::button::Button::new(0, 0, 100, field_h, "Browse");
            b.set_color(theme.primary);
            b.set_label_color(theme.text);
            b.set_label_size(theme.font_size);
            b.set_callback(move |_| {
                if let Some(dir) = theme::dir_chooser("Select Destination Directory", "/tmp") {
                    dest_input_cb.borrow_mut().set_value(&dir);
                }
            });
            b
        }));

        let dd_check = Rc::new(RefCell::new({
            let mut c = CheckButton::new(
                0,
                0,
                350,
                field_h,
                "Create DD forensic image before scanning",
            );
            c.set_label_color(theme.text);
            c.set_label_size(theme.font_size);
            c.set_color(theme.text);
            c.set_tooltip(
                "Copy the whole device into an .dd image first, then scan the\n\
                 image. Keeps the original untouched for forensic evidence.",
            );
            c
        }));

        let dd_path_input = Rc::new(RefCell::new(String::new()));
        {
            let dev = disks.first().map(|d| d.device.clone()).unwrap_or_default();
            let dev_name = dev.strip_prefix("/dev/").unwrap_or(&dev);
            let path = format!("/tmp/recovery/{}.dd", dev_name);
            *dd_path_input.borrow_mut() = path;
        }

        let dd_path_frame = Rc::new(RefCell::new(Frame::new(0, 0, 0, 25, "")));
        {
            let mut f = dd_path_frame.borrow_mut();
            f.set_label_color(Color::from_rgb(150, 150, 150));
            f.set_label_size(theme.font_size - 3);
        }
        {
            let p = dd_path_input.borrow().clone();
            dd_path_frame
                .borrow_mut()
                .set_label(&format!("Image: {}", p));
        }

        let dd_browse_btn = Rc::new(RefCell::new({
            let state_dd_path = dd_path_input.clone();
            let dd_path_frame_cb = dd_path_frame.clone();
            let mut b = fltk::button::Button::new(0, 0, 150, 35, "Browse DD...");
            b.set_color(theme.primary);
            b.set_label_color(theme.text);
            b.set_label_size(theme.font_size - 1);
            b.set_callback(move |_| {
                let current = state_dd_path.borrow().clone();
                let default_name = std::path::Path::new(&current)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(fname) = theme::save_file_chooser(
                    "Save DD Image As",
                    "*.{dd,img,raw}",
                    &default_name,
                    &current,
                ) {
                    *state_dd_path.borrow_mut() = fname.clone();
                    dd_path_frame_cb
                        .borrow_mut()
                        .set_label(&format!("Image: {}", fname));
                }
            });
            b
        }));

        let back_btn = Rc::new(RefCell::new({
            let nav = nav_tx.clone();
            let mut b = create_secondary_button(0, 0, 150, 50, "Back");
            b.set_callback(move |_| {
                let _ = nav.send(Page::Menu);
            });
            b
        }));

        let more_open = Rc::new(std::cell::Cell::new(false));

        let relayout_fn: Rc<RefCell<Option<Box<dyn Fn(i32, i32)>>>> =
            Rc::new(RefCell::new(None));
        let last_size = Rc::new(RefCell::new((win_w, win_h)));

        let more_bg = Rc::new(RefCell::new({
            let mut f = Frame::new(0, 0, label_w + field_w, 175, "");
            f.set_frame(fltk::enums::FrameType::DownBox);
            f.set_color(Color::from_rgb(42, 42, 42));
            f.hide();
            f
        }));

        let ft_label = Rc::new(RefCell::new(Frame::new(
            0,
            0,
            label_w,
            30,
            "Recover file types:",
        )));
        ft_label.borrow_mut().set_label_color(theme.text);
        ft_label.borrow_mut().set_label_size(theme.font_size);

        let category_state = Rc::new(RefCell::new(CategoryMask::all()));
        let mut category_boxes: Vec<Rc<RefCell<CheckButton>>> = Vec::new();
        for (i, cat) in ALL_CATEGORIES.iter().enumerate() {
            let st = category_state.clone();
            let mut c = CheckButton::new(0, 0, 180, 26, cat.label());
            c.set_label_color(theme.text);
            c.set_label_size(theme.font_size - 1);
            c.set_color(theme.text);
            c.set_value(true);
            let rc = Rc::new(RefCell::new(c));
            rc.borrow_mut().set_callback(move |btn| {
                let mask = st.borrow().clone();
                let mask = match i {
                    0 => CategoryMask {
                        photo: btn.is_checked(),
                        ..mask
                    },
                    1 => CategoryMask {
                        video: btn.is_checked(),
                        ..mask
                    },
                    2 => CategoryMask {
                        document: btn.is_checked(),
                        ..mask
                    },
                    3 => CategoryMask {
                        audio: btn.is_checked(),
                        ..mask
                    },
                    4 => CategoryMask {
                        archive: btn.is_checked(),
                        ..mask
                    },
                    _ => CategoryMask {
                        other: btn.is_checked(),
                        ..mask
                    },
                };
                *st.borrow_mut() = mask;
            });
            category_boxes.push(rc);
        }
        let cat_photo = category_boxes[0].clone();
        let cat_video = category_boxes[1].clone();
        let cat_document = category_boxes[2].clone();
        let cat_audio = category_boxes[3].clone();
        let cat_archive = category_boxes[4].clone();
        let cat_other = category_boxes[5].clone();

        let ft_state = category_state.clone();
        let cat_all_btn = Rc::new(RefCell::new({
            let boxes = category_boxes.clone();
            let state = ft_state.clone();
            let mut b = create_secondary_button(0, 0, 84, 22, "All");
            b.set_label_size(theme.font_size - 1);
            b.set_tooltip("Enable every file-type category");
            b.set_callback(move |_| {
                *state.borrow_mut() = CategoryMask::all();
                for c in &boxes {
                    c.borrow_mut().set_value(true);
                }
            });
            b
        }));

        let cat_none_btn = Rc::new(RefCell::new({
            let boxes = category_boxes.clone();
            let state = ft_state.clone();
            let mut b = create_secondary_button(0, 0, 84, 22, "None");
            b.set_label_size(theme.font_size - 1);
            b.set_tooltip("Disable every file-type category");
            b.set_callback(move |_| {
                *state.borrow_mut() = CategoryMask {
                    photo: false,
                    video: false,
                    document: false,
                    audio: false,
                    archive: false,
                    other: false,
                };
                for c in &boxes {
                    c.borrow_mut().set_value(false);
                }
            });
            b
        }));

        let frag_check = Rc::new(RefCell::new({
            let mut c = CheckButton::new(
                0,
                0,
                label_w + field_w,
                26,
                "Reassemble fragmented files (slow)",
            );
            c.set_label_color(theme.text);
            c.set_label_size(theme.font_size - 1);
            c.set_color(theme.text);
            c.set_value(false);
            c.set_tooltip(
                "After the normal scan, additionally search other sectors for\n\
                 continuation fragments and stitch files back together.\n\
                 Much slower; only helps when files are badly fragmented.",
            );
            c
        }));

        let dir_filter_input = Rc::new(RefCell::new({
            let mut i = fltk::input::Input::new(0, 0, field_w, 26, "");
            i.set_color(Color::from_rgb(30, 30, 30));
            i.set_text_color(theme.text);
            i.set_tooltip(
                "Optional: recover only this folder with the structure pass.\n\
                 Windows-style path, e.g.  \\Documents  or  \\Documents\\Data.\n\
                 Leave empty to recover the whole volume.",
            );
            i.deactivate();
            i
        }));

        let fs_pass_check = Rc::new(RefCell::new({
            let df_input = dir_filter_input.clone();
            let mut c = CheckButton::new(
                0,
                0,
                label_w + field_w,
                26,
                "Use filesystem structure (FAT/NTFS) — exact names & deleted files",
            );
            c.set_label_color(theme.text);
            c.set_label_size(theme.font_size - 1);
            c.set_color(theme.text);
            c.set_value(false);
            c.set_tooltip(
                "Recover files by following the filesystem itself:\n\
                 • exact folder paths and file names (FAT and NTFS)\n\
                 • fragmentation is handled automatically\n\
                 • deleted files are recovered too — if their data was\n\
                   overwritten, a surviving copy is searched for and rebuilt",
            );
            c.set_callback(move |btn| {
                if btn.is_checked() {
                    df_input.borrow_mut().activate();
                } else {
                    df_input.borrow_mut().deactivate();
                    df_input.borrow_mut().set_value("");
                }
            });
            c
        }));

        let dir_filter_label = Rc::new(RefCell::new({
            let mut f = Frame::new(0, 0, label_w, 26, "Folder (e.g. \\Documents):");
            f.set_label_color(theme.text);
            f.set_label_size(theme.font_size - 1);
            f
        }));

        let more_widgets: Vec<Rc<RefCell<dyn fltk::prelude::WidgetExt>>> = vec![
            more_bg.clone(),
            ft_label.clone(),
            cat_photo.clone(),
            cat_video.clone(),
            cat_document.clone(),
            cat_audio.clone(),
            cat_archive.clone(),
            cat_other.clone(),
            cat_all_btn.clone(),
            cat_none_btn.clone(),
            frag_check.clone(),
            fs_pass_check.clone(),
            dir_filter_label.clone(),
            dir_filter_input.clone(),
        ];
        for w in &more_widgets {
            w.borrow_mut().hide();
        }

        let more_btn = Rc::new(RefCell::new({
            let open = more_open.clone();
            let relayout = relayout_fn.clone();
            let size = last_size.clone();
            let wids = more_widgets.clone();
            let mut b = create_secondary_button(0, 0, 180, 30, "More Options");
            b.set_label_size(theme.font_size - 1);
            b.set_tooltip("Show or hide advanced recovery options");
            b.set_callback(move |btn| {
                let now_open = !open.get();
                open.set(now_open);
                for w in &wids {
                    if now_open {
                        w.borrow_mut().show();
                    } else {
                        w.borrow_mut().hide();
                    }
                }
                btn.set_label(if now_open {
                    "Hide Options"
                } else {
                    "More Options"
                });
                let (w, h) = *size.borrow();
                if let Some(f) = relayout.borrow().as_ref() {
                    f(w, h);
                }
            });
            b
        }));

        let start_btn = Rc::new(RefCell::new({
            let tx = cmd_tx.clone();
            let disk_choice_cb = disk_choice.clone();
            let part_choice_cb = part_choice.clone();
            let partitions_cb = partitions.clone();
            let dd_check_cb = dd_check.clone();
            let dd_path_cb = dd_path_input.clone();
            let scan_type_cb = scan_type_state.clone();
            let dest_input_cb = dest_input.clone();
            let default_dir = default_dir.clone();
            let disks = disks.clone();
            let category_state_cb = category_state.clone();
            let frag_state_cb = frag_check.clone();
            let fs_pass_cb = fs_pass_check.clone();
            let dir_filter_cb = dir_filter_input.clone();
            let mut b = create_primary_button(0, 0, 180, 50, "Start Scan");
            if !can_access {
                b.deactivate();
            }
            b.set_callback(move |_| {
                let list = partitions_cb.borrow();
                let part_idx = part_choice_cb.borrow().value() as usize;
                let (part_offset, part_size) = if part_idx < list.len() {
                    (list[part_idx].offset, list[part_idx].size)
                } else {
                    (0, 0)
                };
                let disk_idx = disk_choice_cb.borrow().value() as usize;
                let dev = if disk_idx < disks.len() {
                    disks[disk_idx].device.clone()
                } else {
                    return;
                };
                let out = dest_input_cb.borrow().value();
                let dd_enabled = dd_check_cb.borrow().is_checked();
                let dd_path = if dd_enabled {
                    Some(dd_path_cb.borrow().clone())
                } else {
                    None
                };
                let mask = *category_state_cb.borrow();
                let fs_pass = fs_pass_cb.borrow().is_checked();
                let dir_filter = if fs_pass {
                    let v = dir_filter_cb.borrow().value();
                    if v.is_empty() {
                        None
                    } else {
                        Some(v)
                    }
                } else {
                    None
                };
                let cmd = WorkerCommand::StartScan {
                    device: dev,
                    output_dir: if out.is_empty() {
                        default_dir.clone()
                    } else {
                        out
                    },
                    options: PhotoRecOptions {
                        file_categories: Some(mask),
                        frag_reassembly: frag_state_cb.borrow().is_checked(),
                        fs_pass,
                        dir_filter,
                    },
                    part_offset,
                    part_size,
                    dd_path,
                    scan_type: scan_type_cb.borrow().clone(),
                };
                let _ = tx.send(cmd);
            });
            b
        }));

        let ft_y = start_y + spacing * 5;

        let rel = (
            title.clone(),
            banner.clone(),
            disk_label.clone(),
            disk_choice.clone(),
            part_label.clone(),
            part_choice.clone(),
            fs_label.clone(),
            fs_choice.clone(),
            scan_type_label.clone(),
            scan_type_choice.clone(),
            dest_label.clone(),
            dest_input.clone(),
            dest_browse.clone(),
            ft_label.clone(),
            cat_photo.clone(),
            cat_video.clone(),
            cat_document.clone(),
            cat_audio.clone(),
            cat_archive.clone(),
            cat_other.clone(),
            info_label.clone(),
            info_dev.clone(),
            info_cap.clone(),
            info_sec.clone(),
            info_model.clone(),
            dd_check.clone(),
            dd_path_frame.clone(),
            dd_browse_btn.clone(),
            back_btn.clone(),
            start_btn.clone(),
            frag_check.clone(),
            fs_pass_check.clone(),
            dir_filter_label.clone(),
            dir_filter_input.clone(),
            cat_all_btn.clone(),
            cat_none_btn.clone(),
            more_btn.clone(),
            more_bg.clone(),
        );
        let relayout = move |w: i32, h: i32| {
            let form_left = (w - (label_w + field_w)) / 2;
            let group_top = ft_y + 50;
            let open = more_open.get();
            let dd_y = if open {
                group_top + 175
            } else {
                group_top
            };
            let info_start_y = dd_y + 125;

            *last_size.borrow_mut() = (w, h);

            rel.0.borrow_mut().set_pos((w - title_w) / 2, 20);
            rel.1.borrow_mut().resize(20, 65, w - 40, 30);

            rel.2.borrow_mut().set_pos(form_left, start_y);
            rel.3.borrow_mut().set_pos(form_left + label_w, start_y);
            rel.4.borrow_mut().set_pos(form_left, start_y + spacing);
            rel.5
                .borrow_mut()
                .set_pos(form_left + label_w, start_y + spacing);
            rel.6.borrow_mut().set_pos(form_left, start_y + spacing * 2);
            rel.7
                .borrow_mut()
                .set_pos(form_left + label_w, start_y + spacing * 2);
            rel.8.borrow_mut().set_pos(form_left, start_y + spacing * 3);
            rel.9
                .borrow_mut()
                .set_pos(form_left + label_w, start_y + spacing * 3);
            rel.10
                .borrow_mut()
                .set_pos(form_left, start_y + spacing * 4);
            rel.11
                .borrow_mut()
                .set_pos(form_left + label_w, start_y + spacing * 4);
            rel.12.borrow_mut().set_pos(
                form_left + label_w + field_w - 110 + 10,
                start_y + spacing * 4,
            );

            rel.36.borrow_mut().set_pos(form_left, ft_y);

            rel.37.borrow_mut().set_pos(form_left, group_top);

            rel.13.borrow_mut().set_pos(form_left, group_top + 10);
            rel.14.borrow_mut().set_pos(form_left, group_top + 38);
            rel.15.borrow_mut().set_pos(form_left + 196, group_top + 38);
            rel.16.borrow_mut().set_pos(form_left + 392, group_top + 38);
            rel.17.borrow_mut().set_pos(form_left, group_top + 64);
            rel.18.borrow_mut().set_pos(form_left + 196, group_top + 64);
            rel.19.borrow_mut().set_pos(form_left + 392, group_top + 64);
            rel.34.borrow_mut().set_pos(form_left + label_w + field_w - 180, group_top + 10);
            rel.35.borrow_mut().set_pos(form_left + label_w + field_w - 92, group_top + 10);
            rel.30.borrow_mut().set_pos(form_left, group_top + 92);
            rel.31.borrow_mut().set_pos(form_left + 30, group_top + 118);
            rel.32.borrow_mut().set_pos(form_left + 30, group_top + 146);
            rel.33
                .borrow_mut()
                .set_pos(form_left + 30 + 195, group_top + 146);

            rel.20.borrow_mut().set_pos(form_left, info_start_y);
            rel.21.borrow_mut().set_pos(form_left, info_start_y + 22);
            rel.22.borrow_mut().set_pos(form_left, info_start_y + 44);
            rel.23.borrow_mut().set_pos(form_left, info_start_y + 66);
            rel.24.borrow_mut().set_pos(form_left, info_start_y + 88);

            rel.25.borrow_mut().set_pos(form_left, dd_y);
            let path_w = (w - 40).min(600);
            rel.27.borrow_mut().set_pos((w - 150) / 2, dd_y + 33);
            rel.26.borrow_mut().resize(20, dd_y + 77, path_w, 25);
            rel.26.borrow_mut().set_pos((w - path_w) / 2, dd_y + 77);

            rel.28.borrow_mut().set_pos(w - 200, h - 80);
            rel.29.borrow_mut().set_pos(w / 2 - 90, h - 80);
        };

        *relayout_fn.borrow_mut() = Some(Box::new(relayout));
        let relayout_cb = relayout_fn.clone();
        if let Some(f) = relayout_fn.borrow().as_ref() {
            f(win_w, win_h);
        }
        page.resize_callback(move |_, _, _, w, h| {
            if let Some(f) = relayout_cb.borrow().as_ref() {
                f(w, h);
            }
        });

        let spacer = fltk::frame::Frame::new(0, 0, 0, 0, "");
        page.resizable(&spacer);
        page.end();
        page.show();

        Self
    }
}

fn partition_label(p: &crate::util::disks::PartitionInfo) -> String {
    if p.offset == 0 && p.size > 0 {
        format!("Whole Disk ({})", p.name)
    } else {
        let mb = p.size / (1024 * 1024);
        let gb = mb / 1024;
        if gb > 0 {
            format!("{}  ({} GB, offset {})", p.name, gb, p.offset)
        } else {
            format!("{}  ({} MB, offset {})", p.name, mb, p.offset)
        }
    }
}
