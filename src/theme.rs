use fltk::dialog::FileChooserType;
use fltk::enums::Color;
use fltk::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static THEME: OnceLock<Theme> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub primary: Color,
    pub text: Color,
    pub font_size: i32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::Black,
            primary: Color::from_rgb(56, 168, 194),
            text: Color::from_rgb(255, 255, 255),
            font_size: 16,
        }
    }
}

impl Theme {
    pub fn global() -> &'static Theme {
        THEME.get_or_init(|| Theme::default())
    }

    pub fn apply() {
        let theme = Theme::global();
        fltk::app::set_scheme(fltk::app::Scheme::Gtk);
        let (r, g, b) = theme.background.to_rgb();
        fltk::app::set_background_color(r, g, b);
        fltk::app::set_background2_color(30, 30, 30);
        fltk::app::set_foreground_color(255, 255, 255);
        let (r, g, b) = theme.primary.to_rgb();
        fltk::app::set_selection_color(r, g, b);
    }

    pub fn font() -> &'static str {
        "Arial"
    }
}

/// Open file selection dialog, themed to match the app and floated above the
/// always-on-top main window.
pub fn file_chooser<P: AsRef<Path>>(title: &str, pattern: &str, dir: P) -> Option<String> {
    chooser_loop(FileChooserType::Single, title, pattern, dir.as_ref(), "")
}

/// Directory selection dialog, themed and floated like the file dialog.
pub fn dir_chooser<P: AsRef<Path>>(title: &str, dir: P) -> Option<String> {
    chooser_loop(FileChooserType::Directory, title, "*", dir.as_ref(), "")
}

/// Save-file dialog. `default_name` pre-fills the file-name field.
pub fn save_file_chooser<P: AsRef<Path>>(
    title: &str,
    pattern: &str,
    default_name: &str,
    dir: P,
) -> Option<String> {
    chooser_loop(
        FileChooserType::Create,
        title,
        pattern,
        dir.as_ref(),
        default_name,
    )
}

fn chooser_loop<P: AsRef<Path>>(
    typ: FileChooserType,
    title: &str,
    pattern: &str,
    dir: P,
    default_name: &str,
) -> Option<String> {
    let mut chooser =
        fltk::dialog::FileChooser::new(dir.as_ref(), &build_filter(pattern), typ, title);
    if !default_name.is_empty() {
        chooser.set_value(default_name);
    }
    style_chooser(&mut chooser);
    let mut win =
        unsafe { fltk::window::Window::from_widget_ptr(chooser.window().as_widget_ptr()) };
    style_window(&mut win);
    darken_tree(&win);
    win.redraw();
    chooser.show();
    win.set_on_top();
    win.wait_for_expose();
    win.redraw();
    while chooser.shown() {
        fltk::app::wait();
    }
    chooser.value(1)
}

fn build_filter(pattern: &str) -> String {
    if pattern.is_empty() || pattern == "*" {
        return "All Files (*)".to_string();
    }
    let (name, exts) = parse_pattern(pattern);
    if exts.is_empty() {
        return "All Files (*)".to_string();
    }
    let globs: Vec<String> = exts.iter().map(|e| format!("*.{e}")).collect();
    format!("{name} ({})\tAll Files (*)", globs.join(" "))
}

fn style_chooser(c: &mut fltk::dialog::FileChooser) {
    let theme = Theme::global();
    c.set_color(theme.background);
    c.set_text_color(theme.text);
    c.set_text_size(theme.font_size);
    c.set_icon_size(theme.font_size as u8);
    c.set_text_font(fltk::enums::Font::Helvetica);
    if let Some(mut b) = c.new_button() {
        style_button(&mut b);
    }
    if let Some(mut b) = c.show_hidden_button() {
        style_muted_button(&mut b);
    }
}

/// Recursively apply dark field backgrounds and white text to the widget tree
/// of the file chooser window, so it matches the app theme instead of FLTK's
/// default light backgrounds.
fn darken_tree<G: GroupExt + WidgetExt>(group: &G) {
    let theme = Theme::global();
    for i in 0..group.children() {
        let Some(mut w) = group.child(i) else {
            continue;
        };
        w.set_color(field_color());
        w.set_selection_color(theme.primary);
        w.set_label_color(theme.text);
        if let Some(g) = w.as_group() {
            darken_tree(&g);
        }
    }
}

fn path_to_string(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// Convert a "*.{a,b,c}" or "*.ext" filter pattern into an rfd filter.
fn parse_pattern(pattern: &str) -> (String, Vec<String>) {
    let p = pattern.trim();
    let content = p.strip_prefix("*.").unwrap_or(p);
    let exts: Vec<String> = if content.starts_with('{') && content.ends_with('}') {
        content[1..content.len() - 1]
            .split(',')
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty())
            .collect()
    } else if !content.is_empty() {
        vec![content.to_string()]
    } else {
        Vec::new()
    };
    let name = if exts.len() == 1 {
        format!(".{}", exts[0])
    } else {
        "Files".to_string()
    };
    (name, exts)
}

// ---------------------------------------------------------------------------
// Consistent widget styling helpers so dialogs/windows match the main theme.
// ---------------------------------------------------------------------------

/// A dark field color (inputs, text areas, menus).
pub fn field_color() -> Color {
    Color::from_rgb(30, 30, 30)
}

/// A muted label color for secondary/hint text.
pub fn muted_color() -> Color {
    Color::from_rgb(150, 150, 150)
}

/// Style a window's background to match the app background.
pub fn style_window<W: WidgetExt>(win: &mut W) {
    win.set_color(Theme::global().background);
}

/// Style an input/choice/text-style widget.
pub fn style_field<W: WidgetExt + DisplayExt>(w: &mut W) {
    w.set_color(field_color());
    w.set_label_color(Theme::global().text);
    w.set_text_color(Theme::global().text);
}

/// Style a single-line text input.
pub fn style_input<W: WidgetExt + InputExt>(w: &mut W) {
    w.set_color(field_color());
    w.set_label_color(Theme::global().text);
    w.set_text_color(Theme::global().text);
}

/// Style a dropdown choice/menu.
pub fn style_choice<W: WidgetExt + MenuExt>(w: &mut W) {
    w.set_color(field_color());
    w.set_label_color(Theme::global().text);
    w.set_text_color(Theme::global().text);
}

/// Style a label/frame: transparent background, theme text color.
pub fn style_label<W: WidgetExt>(w: &mut W) {
    w.set_color(Theme::global().background);
    if w.label_color() != Color::White && w.label_color() != Color::from_rgb(0, 0, 0) {
        w.set_label_color(Theme::global().text);
    }
    w.set_label_size(Theme::global().font_size);
    w.set_frame(fltk::enums::FrameType::NoBox);
}

/// Style a primary action button (matches the main UI).
pub fn style_button<W: ButtonExt + WidgetExt>(b: &mut W) {
    b.set_color(Theme::global().primary);
    b.set_selection_color(Theme::global().primary);
    b.set_label_color(Theme::global().text);
    b.set_label_size(Theme::global().font_size);
    b.set_frame(fltk::enums::FrameType::RoundUpBox);
}

/// Style a destructive/muted button.
pub fn style_muted_button<W: ButtonExt + WidgetExt>(b: &mut W) {
    b.set_color(Color::from_rgb(40, 120, 140));
    b.set_selection_color(Color::from_rgb(40, 120, 140));
    b.set_label_color(Theme::global().text);
    b.set_label_size(Theme::global().font_size);
    b.set_frame(fltk::enums::FrameType::RoundUpBox);
}

// ---------------------------------------------------------------------------
// Themed modal dialog helpers (replace the default light FLTK message boxes).
// ---------------------------------------------------------------------------

/// Themed modal "OK" info dialog. Blocks until the user clicks OK or closes it.
pub fn message(title: &str, msg: &str) {
    dialog(title, msg, &["OK"]);
}

/// Themed modal choice dialog returning the index of the selected button, or
/// `None` when the dialog was dismissed via the window close button.
pub fn choice(title: &str, msg: &str, buttons: &[&str]) -> Option<i32> {
    dialog(title, msg, buttons)
}

fn dialog(title: &str, msg: &str, buttons: &[&str]) -> Option<i32> {
    let theme = Theme::global();
    let scr = fltk::app::screen_size();
    let est_lines = msg.lines().count().max(3) as i32;
    let w = if msg.chars().count() > 80 { 620 } else { 520 };
    let h = 120 + est_lines * 18;
    let x = (((scr.0 - w as f64) / 2.0).max(0.0)) as i32;
    let y = (((scr.1 - h as f64) / 2.0).max(0.0)) as i32;

    let mut win = fltk::window::Window::new(x, y, w, h, title);
    win.set_color(theme.background);

    let mut msg_box = fltk::frame::Frame::new(20, 20, w - 40, h - 80, msg);
    style_label(&mut msg_box);
    msg_box.set_align(fltk::enums::Align::Inside | fltk::enums::Align::Wrap);
    msg_box.set_label_size(theme.font_size - 1);

    let n = buttons.len().clamp(1, 3) as i32;
    let btn_w = (w / n) - 20;
    let btn_h = 40;
    let btn_y = h - 50;
    let selected: std::rc::Rc<std::cell::RefCell<Option<i32>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    for (i, label) in buttons.iter().enumerate().take(3) {
        let i = i as i32;
        let bx = if n == 1 {
            (w - btn_w) / 2
        } else if n == 2 {
            20 + i * ((w - 40) / 2) + if i == 1 { ((w - 40) / 2) - btn_w } else { 0 }
        } else {
            20 + i * ((w - 40) / 3)
                + if i == 2 {
                    ((w - 40) / 3) * 2 - btn_w
                } else {
                    if i == 1 {
                        ((w - 40) / 3) - btn_w
                    } else {
                        0
                    }
                }
        };
        let mut btn = fltk::button::Button::new(bx, btn_y, btn_w, btn_h, &label[..]);
        if i == n - 1 {
            style_button(&mut btn);
        } else {
            style_muted_button(&mut btn);
        }
        let selected = selected.clone();
        btn.set_callback(move |_| *selected.borrow_mut() = Some(i));
    }

    win.end();
    win.show();
    win.make_modal(true);
    while selected.borrow().is_none() && win.visible() {
        fltk::app::wait();
    }
    let result = *selected.borrow();
    win.hide();
    result
}
