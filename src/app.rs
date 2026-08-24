use crate::backend::{Worker, WorkerEvent};
use crate::pages::{
    AboutPage, AdvancedPage, DeepRepairPage, ExperimentalPage, ImageScanPage, MenuPage,
    RecoveryPage, RepairPage, ScanPage, ScanningPage, SettingsPage,
};
use crate::theme::Theme;
use crate::util::config::Config;
use crate::util::init_logging;
use anyhow::Result;
use crossbeam_channel::{unbounded, Receiver, Sender};
use fltk::{app::App as FlApp, prelude::*, window::Window};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub enum Page {
    Menu,
    Scan,
    Scanning,
    Recovery,
    ImageScan,
    Advanced,
    Experimental,
    Repair,
    DeepRepair,
    Settings,
    About,
}

#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub percent: u32,
    pub current_file: String,
    pub files_found: u64,
}

#[derive(Debug, Clone)]
pub struct RecoveredFile {
    pub filename: String,
    pub extension: String,
    pub size: u64,
    pub status: String,
}

pub struct App {
    fltk_app: FlApp,
    window: Window,
    current_page: Page,
    worker: Worker,
    worker_rx: Receiver<WorkerEvent>,
    nav_tx: Sender<Page>,
    nav_rx: Receiver<Page>,
    scan_progress: ScanProgress,
    recovered_files: Vec<RecoveredFile>,
    log_lines: Vec<String>,
    last_scan_rebuild: std::time::Instant,
    scan_start_time: Option<std::time::Instant>,
    last_output_dir: String,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load(&Config::default_path()).unwrap_or_default();
        init_logging(config.auto_save_log);
        let fltk_app = FlApp::default();
        Theme::apply();

        let current_scale = fltk::app::screen_scale(0);
        fltk::app::set_screen_scale(0, current_scale * 1.25);

        let mut window = Window::default()
            .with_size(1024, 768)
            .with_label("Kaifuku - PhotoRec Frontend");

        window.make_resizable(true);
        window.size_range(800, 600, 0, 0);

        let worker = Worker::new();
        let worker_rx = worker.event_receiver();
        let (nav_tx, nav_rx) = unbounded();

        window.set_color(Theme::global().background);

        if let Ok(icon) = fltk::image::PngImage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/photorec_64x64.png"
        )) {
            window.set_icon(Some(icon));
        }

        window.end();
        window.show();
        window.maximize();
        window.set_on_top();

        let mut app = Self {
            fltk_app,
            window,
            current_page: Page::Menu,
            worker,
            worker_rx,
            nav_tx,
            nav_rx,
            scan_progress: ScanProgress::default(),
            recovered_files: Vec::new(),
            log_lines: Vec::new(),
            last_scan_rebuild: std::time::Instant::now(),
            scan_start_time: None,
            last_output_dir: "/tmp/recovery".to_string(),
        };

        app.show_page(Page::Menu);
        Ok(app)
    }

    pub fn run(&mut self) {
        while self.fltk_app.wait() {
            while let Ok(page) = self.nav_rx.try_recv() {
                self.show_page(page);
            }
            while let Ok(event) = self.worker_rx.try_recv() {
                self.handle_worker_event(event);
            }
        }
    }

    fn show_page(&mut self, page: Page) {
        self.current_page = page;
        let (win_w, win_h) = (self.window.width(), self.window.height());

        self.window.clear();
        self.window.begin();

        match page {
            Page::Menu => {
                MenuPage::new(win_w, win_h, self.nav_tx.clone());
            }
            Page::Scan => {
                ScanPage::new(
                    win_w,
                    win_h,
                    self.nav_tx.clone(),
                    self.worker.sender.clone(),
                );
            }
            Page::ImageScan => {
                ImageScanPage::new(
                    win_w,
                    win_h,
                    self.nav_tx.clone(),
                    self.worker.sender.clone(),
                );
            }
            Page::Scanning => {
                ScanningPage::new(
                    win_w,
                    win_h,
                    &self.scan_progress,
                    &self.log_lines,
                    self.worker.sender.clone(),
                );
            }
            Page::Recovery => {
                RecoveryPage::new(
                    win_w,
                    win_h,
                    Arc::new(self.recovered_files.clone()),
                    self.scan_start_time,
                    &self.last_output_dir,
                    self.nav_tx.clone(),
                );
            }
            Page::Advanced => {
                AdvancedPage::new(win_w, win_h, self.nav_tx.clone());
            }
            Page::Experimental => {
                ExperimentalPage::new(win_w, win_h, self.nav_tx.clone());
            }
            Page::Repair => {
                RepairPage::new(win_w, win_h, self.nav_tx.clone());
            }
            Page::DeepRepair => {
                DeepRepairPage::new(win_w, win_h, self.nav_tx.clone());
            }
            Page::Settings => {
                SettingsPage::new(win_w, win_h, self.nav_tx.clone());
            }
            Page::About => {
                AboutPage::new(win_w, win_h, self.nav_tx.clone());
            }
        }

        self.window.end();
        self.window.redraw();
    }

    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Ready => {
                log::info!("Worker ready");
            }
            WorkerEvent::ScanStarted { output_dir } => {
                log::info!("Scan started");
                self.last_output_dir = output_dir;
                self.scan_progress = ScanProgress::default();
                self.recovered_files.clear();
                self.log_lines.clear();
                self.push_log(format!(
                    "Recovery started — output: {}",
                    self.last_output_dir
                ));
                self.last_scan_rebuild = std::time::Instant::now();
                self.scan_start_time = Some(std::time::Instant::now());
                self.show_page(Page::Scanning);
            }
            WorkerEvent::ProgressUpdate {
                percent,
                current_file,
                files_found,
            } => {
                self.scan_progress = ScanProgress {
                    percent,
                    current_file,
                    files_found,
                };
                if matches!(self.current_page, Page::Scanning) {
                    let now = std::time::Instant::now();
                    if now.duration_since(self.last_scan_rebuild).as_millis() > 250 {
                        self.last_scan_rebuild = now;
                        self.show_page(Page::Scanning);
                    }
                }
            }
            WorkerEvent::FileFound {
                filename,
                extension,
                size,
            } => {
                self.recovered_files.push(RecoveredFile {
                    filename: filename.clone(),
                    extension,
                    size,
                    status: "Recovered".to_string(),
                });
                let size = if size >= 1024 * 1024 {
                    format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
                } else if size >= 1024 {
                    format!("{:.1} KiB", size as f64 / 1024.0)
                } else {
                    format!("{} B", size)
                };
                self.push_log(format!("Recovered: {} ({})", filename, size));
            }
            WorkerEvent::LogMessage(msg) => {
                self.push_log(msg);
            }
            WorkerEvent::ScanComplete => {
                log::info!(
                    "Scan complete, {} files recovered",
                    self.recovered_files.len()
                );
                self.push_log(format!(
                    "Scan complete — {} files recovered",
                    self.recovered_files.len()
                ));
                self.show_page(Page::Recovery);
            }
            WorkerEvent::ScanStopped => {
                log::info!("Scan stopped");
                self.push_log("Scan stopped by user".to_string());
                self.show_page(Page::Menu);
            }
            WorkerEvent::Error(err) => {
                log::error!("Worker error: {}", err);
                self.push_log(format!("Error: {}", err));
            }
        }
    }

    /// Append a timestamped line to the on-screen recovery log, keeping only
    /// the most recent `LOG_LINE_LIMIT` lines so memory stays bounded.
    fn push_log(&mut self, msg: impl AsRef<str>) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| {
                let secs = d.as_secs() % 86400;
                format!(
                    "{:02}:{:02}:{:02}",
                    secs / 3600,
                    (secs % 3600) / 60,
                    secs % 60
                )
            })
            .unwrap_or_else(|_| "--:--:--".to_string());
        let line = format!("[{}] {}", ts, msg.as_ref());
        self.log_lines.push(line);
        const LOG_LINE_LIMIT: usize = 5000;
        if self.log_lines.len() > LOG_LINE_LIMIT {
            let excess = self.log_lines.len() - LOG_LINE_LIMIT;
            self.log_lines.drain(..excess);
        }
    }
}
