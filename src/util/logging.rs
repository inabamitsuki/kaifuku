use log::{LevelFilter, Log, Metadata, Record};
use std::fs::OpenOptions;
use std::io::Write;

struct KaifukuLogger {
    file: std::sync::Mutex<Option<std::fs::File>>,
}

impl Log for KaifukuLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let msg = format!(
            "[{} {}] {}\n",
            record.level(),
            record.target(),
            record.args()
        );
        let _ = std::io::stderr().write_all(msg.as_bytes());
        if let Ok(f) = self.file.lock() {
            if let Some(ref file) = *f {
                let _ = (&*file).write_all(msg.as_bytes());
            }
        }
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
        if let Ok(f) = self.file.lock() {
            if let Some(ref file) = *f {
                let _ = (&*file).flush();
            }
        }
    }
}

static LOGGER: KaifukuLogger = KaifukuLogger {
    file: std::sync::Mutex::new(None),
};

/// Initialize logging for the application
pub fn init_logging(save_log: bool) {
    let file = if save_log {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/kaifuku.log")
            .ok()
    } else {
        None
    };

    if let Ok(mut f) = LOGGER.file.lock() {
        *f = file;
    }
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(LevelFilter::Info));
}
