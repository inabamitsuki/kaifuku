use crossbeam_channel::Sender;
use std::ffi::CStr;
use std::io;
use std::os::raw::{c_char, c_int, c_ulonglong, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

use crate::backend::ffi::{self, KaifukuCallbacks, KaifukuCtx};
use crate::backend::filetypes::{CategoryMask, FileCategory};
use crate::backend::ntfs;
use crate::backend::worker::WorkerEvent;

pub struct PhotoRecController {
    ctx: Mutex<Option<KaifukuCtx>>,
    callbacks_data: Mutex<*mut c_void>,
}

#[derive(Debug, Clone, Default)]
pub struct PhotoRecOptions {
    /// Which categories of files to recover. `None` (the default) recovers
    /// every category with the engine's default settings.
    pub file_categories: Option<CategoryMask>,
    /// Run the engine's brute-force fragmented-file reassembly pass after the
    /// normal carve. Slower, but can re-join pieces of fragmented files.
    pub frag_reassembly: bool,
    /// Recover files using the FAT filesystem structure first (exact names and
    /// fragmentation handled by following the cluster table), before carving.
    pub fs_pass: bool,
    /// Optional Windows-style path (e.g. `\\Documents`) restricting the
    /// filesystem-structure pass to a single directory.
    pub dir_filter: Option<String>,
}

impl PhotoRecController {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            ctx: Mutex::new(None),
            callbacks_data: Mutex::new(std::ptr::null_mut()),
        })
    }

    /// Free the boxed callback sender that was passed to C as `user_data`.
    /// Only safe once the C scan thread has fully finished (ctx->running == 0).
    pub fn reclaim_callbacks(&self) {
        let ptr = std::mem::replace(
            &mut *self.callbacks_data.lock().unwrap(),
            std::ptr::null_mut(),
        );
        if !ptr.is_null() {
            unsafe {
                drop(Box::from_raw(ptr as *mut Sender<WorkerEvent>));
            }
        }
    }

    pub fn start_scan(
        &self,
        device: &str,
        output_dir: &str,
        options: &PhotoRecOptions,
        event_sender: Sender<WorkerEvent>,
        part_offset: u64,
        part_size: u64,
    ) -> Result<(), String> {
        let mut ctx_guard = self.ctx.lock().unwrap();
        if let Some(old) = ctx_guard.take() {
            ffi::destroy(old);
        }
        let ctx = ffi::init().ok_or("Failed to initialize FFI context")?;
        *ctx_guard = Some(ctx);
        drop(ctx_guard);

        // Apply the file type filter AFTER init: kaifuku_init() resets the
        // enable array back to the engine defaults, so filtering first would
        // be wiped out before the scan starts.
        match options.file_categories {
            Some(mask) if !mask.is_all() => {
                let known = ffi::enumerate_extensions();
                let selected: Vec<String> = known
                    .iter()
                    .filter(|ext| mask.includes(FileCategory::classify(ext)))
                    .cloned()
                    .collect();
                ffi::set_file_filter(&selected);
            }
            _ => {
                // Everything selected: restore the engine defaults.
                ffi::set_file_filter(&[]);
            }
        }

        let ctx_ptr = *self.ctx.lock().unwrap();
        if let Some(ctx) = ctx_ptr {
            ffi::set_frag_reassembly(ctx, options.frag_reassembly);
            ffi::set_filesystem_pass(ctx, options.fs_pass);
            ffi::set_directory_filter(ctx, options.dir_filter.as_deref());
        }

        self.reclaim_callbacks();

        let sender = Box::new(event_sender);
        let sender_ptr = Box::into_raw(sender) as *mut c_void;
        *self.callbacks_data.lock().unwrap() = sender_ptr;

        unsafe extern "C" fn progress_cb(
            percent: c_int,
            current_file: *const c_char,
            files_found: c_ulonglong,
            user_data: *mut c_void,
        ) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let sender = &*(user_data as *const Sender<WorkerEvent>);
                let file = if current_file.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(current_file).to_string_lossy().to_string()
                };
                let _ = sender.send(WorkerEvent::ProgressUpdate {
                    percent: percent as u32,
                    current_file: file,
                    files_found,
                });
            }));
        }

        unsafe extern "C" fn file_found_cb(
            filename: *const c_char,
            extension: *const c_char,
            size: c_ulonglong,
            user_data: *mut c_void,
        ) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let sender = &*(user_data as *const Sender<WorkerEvent>);
                let name = if filename.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(filename).to_string_lossy().to_string()
                };
                let ext = if extension.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(extension).to_string_lossy().to_string()
                };
                let _ = sender.send(WorkerEvent::FileFound {
                    filename: name,
                    extension: ext,
                    size,
                });
            }));
        }

        unsafe extern "C" fn log_cb(message: *const c_char, user_data: *mut c_void) -> c_int {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let sender = &*(user_data as *const Sender<WorkerEvent>);
                let msg = if message.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(message).to_string_lossy().to_string()
                };
                let _ = sender.send(WorkerEvent::LogMessage(msg));
            }));
            0
        }

        // Runs on the C scan thread when the partition is NTFS and the
        // filesystem-structure pass is enabled. Reads sectors and marks used
        // ranges through the same ctx, so this is only callable while the
        // scan is live.
        unsafe extern "C" fn ntfs_unformat_cb(
            ctx: KaifukuCtx,
            recup_dir: *const c_char,
            dir_num: u32,
            dir_filter: *const c_char,
            part_offset: u64,
            part_size: u64,
            sector_size: u32,
            user_data: *mut c_void,
        ) -> u64 {
            let _ = part_size;
            catch_unwind(AssertUnwindSafe(|| {
                let sender = &*(user_data as *const Sender<WorkerEvent>);
                let recup = if recup_dir.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(recup_dir).to_string_lossy().to_string()
                };
                let filter = if dir_filter.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(dir_filter).to_string_lossy().to_string())
                };
                let out_root = std::path::PathBuf::from(format!("{}.{}", recup, dir_num));
                let reader = FfiReader {
                    ctx,
                    part_offset,
                    part_size,
                };
                let volume = match ntfs::NtfsVolume::open(reader, part_offset, sector_size as u16) {
                    Ok(v) => v,
                    Err(_) => return 0,
                };
                let params = ntfs::RecoverParams {
                    filter: filter.as_deref(),
                    out_root: &out_root,
                };
                let mut stop = || ffi::stop_requested(ctx);
                let mut on_file = |path: &str, size: u64| {
                    let name = path.rsplit('/').next().unwrap_or(path).to_string();
                    let ext = name
                        .rsplit('.')
                        .nth(1)
                        .map(|s| s.to_lowercase())
                        .unwrap_or_default();
                    let _ = sender.send(WorkerEvent::FileFound {
                        filename: path.to_string(),
                        extension: ext,
                        size,
                    });
                };
                let mut progress = |files: u64, current: &str| {
                    let label = if current.is_empty() {
                        format!("NTFS structure pass: {} files", files)
                    } else {
                        format!("NTFS: {}", current)
                    };
                    let _ = sender.send(WorkerEvent::ProgressUpdate {
                        percent: 0,
                        current_file: label,
                        files_found: files,
                    });
                };
                let result =
                    match ntfs::recover(&volume, &params, &mut progress, &mut on_file, &mut stop) {
                        Ok(r) => r,
                        Err(_) => return 0,
                    };
                let ranges: Vec<ffi::KaifukuRange> = result
                    .used_ranges
                    .iter()
                    .map(|r| ffi::KaifukuRange {
                        offset: r.offset,
                        size: r.size,
                    })
                    .collect();
                ffi::mark_used_ranges(ctx, &ranges);
                result.files
            }))
            .unwrap_or(0)
        }

        let callbacks = KaifukuCallbacks {
            progress: Some(progress_cb),
            file_found: Some(file_found_cb),
            log_msg: Some(log_cb),
            ntfs_unformat: Some(ntfs_unformat_cb),
            user_data: sender_ptr,
        };

        let result = ffi::start_scan(ctx, device, output_dir, callbacks, part_offset, part_size);
        if result.is_err() {
            self.reclaim_callbacks();
            let mut guard = self.ctx.lock().unwrap();
            if let Some(c) = guard.take() {
                ffi::destroy(c);
            }
        }
        result
    }

    pub fn is_running(&self) -> bool {
        let ctx = self.ctx.lock().unwrap();
        match *ctx {
            Some(ctx) => ffi::is_running(ctx),
            None => false,
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        let ctx = self.ctx.lock().unwrap();
        if let Some(ctx) = *ctx {
            ffi::stop(ctx);
        }
        Ok(())
    }
}

impl Drop for PhotoRecController {
    fn drop(&mut self) {
        self.reclaim_callbacks();
        let ctx = self.ctx.lock().unwrap();
        if let Some(ctx) = *ctx {
            ffi::destroy(ctx);
        }
    }
}

/// `ntfs::Reader` backed by the C scan disk, so the NTFS pass reads exactly
/// what the carve pass will skip. Reads are clamped to the scan area.
struct FfiReader {
    ctx: KaifukuCtx,
    part_offset: u64,
    part_size: u64,
}

impl ntfs::Reader for FfiReader {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let end = if self.part_size == 0 {
            u64::MAX
        } else {
            self.part_offset + self.part_size
        };
        if offset < self.part_offset || offset + buf.len() as u64 > end {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read outside scan area",
            ));
        }
        let mut got = 0usize;
        while got < buf.len() {
            let r = ffi::pread(self.ctx, offset + got as u64, &mut buf[got..])
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "pread failed"))?;
            if r == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read"));
            }
            got += r;
        }
        Ok(())
    }
}
