use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_long, c_ulonglong, c_void};

pub type KaifukuCtx = *mut c_void;

pub type KaifukuProgressCb = Option<
    unsafe extern "C" fn(
        percent: c_int,
        current_file: *const c_char,
        files_found: c_ulonglong,
        user_data: *mut c_void,
    ),
>;
pub type KaifukuFileFoundCb = Option<
    unsafe extern "C" fn(
        filename: *const c_char,
        extension: *const c_char,
        size: c_ulonglong,
        user_data: *mut c_void,
    ),
>;
pub type KaifukuLogCb =
    Option<unsafe extern "C" fn(message: *const c_char, user_data: *mut c_void) -> c_int>;
pub type KaifukuExtensionCb =
    Option<unsafe extern "C" fn(extension: *const c_char, user_data: *mut c_void)>;
pub type KaifukuNtfsUnformatCb = Option<
    unsafe extern "C" fn(
        ctx: KaifukuCtx,
        recup_dir: *const c_char,
        dir_num: u32,
        dir_filter: *const c_char,
        part_offset: u64,
        part_size: u64,
        sector_size: u32,
        user_data: *mut c_void,
    ) -> u64,
>;

#[repr(C)]
pub struct KaifukuCallbacks {
    pub progress: KaifukuProgressCb,
    pub file_found: KaifukuFileFoundCb,
    pub log_msg: KaifukuLogCb,
    pub ntfs_unformat: KaifukuNtfsUnformatCb,
    pub user_data: *mut c_void,
}

/// An absolute byte range on the scanned device. Used to tell the C carve
/// pass which sectors the filesystem-structure pass already consumed.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KaifukuRange {
    pub offset: u64,
    pub size: u64,
}

extern "C" {
    fn kaifuku_init() -> KaifukuCtx;
    fn kaifuku_start_scan(
        ctx: KaifukuCtx,
        device_path: *const c_char,
        output_dir: *const c_char,
        callbacks: KaifukuCallbacks,
        part_offset: u64,
        part_size: u64,
    ) -> c_int;
    fn kaifuku_is_running(ctx: KaifukuCtx) -> c_int;
    fn kaifuku_stop_requested(ctx: KaifukuCtx) -> c_int;
    fn kaifuku_stop(ctx: KaifukuCtx);
    fn kaifuku_destroy(ctx: KaifukuCtx);
    fn kaifuku_enumerate_extensions(cb: KaifukuExtensionCb, user_data: *mut c_void);
    fn kaifuku_set_file_filter(extensions: *const *const c_char, count: usize);
    fn kaifuku_set_frag_reassembly(ctx: KaifukuCtx, enabled: c_int) -> c_int;
    fn kaifuku_set_filesystem_pass(ctx: KaifukuCtx, enabled: c_int) -> c_int;
    fn kaifuku_set_directory_filter(ctx: KaifukuCtx, dir_path: *const c_char) -> c_int;
    fn kaifuku_pread(ctx: KaifukuCtx, offset: u64, buf: *mut u8, count: usize) -> c_long;
    fn kaifuku_mark_used_ranges(ctx: KaifukuCtx, ranges: *const KaifukuRange, count: usize);
}

pub fn init() -> Option<KaifukuCtx> {
    let ctx = unsafe { kaifuku_init() };
    if ctx.is_null() {
        None
    } else {
        Some(ctx)
    }
}

pub fn start_scan(
    ctx: KaifukuCtx,
    device: &str,
    output_dir: &str,
    callbacks: KaifukuCallbacks,
    part_offset: u64,
    part_size: u64,
) -> Result<(), String> {
    let device_c = CString::new(device).map_err(|e| e.to_string())?;
    let output_c = CString::new(output_dir).map_err(|e| e.to_string())?;
    let ret = unsafe {
        kaifuku_start_scan(
            ctx,
            device_c.as_ptr(),
            output_c.as_ptr(),
            callbacks,
            part_offset,
            part_size,
        )
    };
    if ret == 0 {
        Ok(())
    } else {
        Err("Failed to start scan".to_string())
    }
}

pub fn is_running(ctx: KaifukuCtx) -> bool {
    unsafe { kaifuku_is_running(ctx) != 0 }
}

/// True when a stop has been requested. Used by long-running passes to abort.
pub fn stop_requested(ctx: KaifukuCtx) -> bool {
    if ctx.is_null() {
        true
    } else {
        unsafe { kaifuku_stop_requested(ctx) != 0 }
    }
}

pub fn stop(ctx: KaifukuCtx) {
    unsafe { kaifuku_stop(ctx) }
}

pub fn destroy(ctx: KaifukuCtx) {
    if !ctx.is_null() {
        unsafe { kaifuku_destroy(ctx) }
    }
}

extern "C" fn collect_extension(extension: *const c_char, user_data: *mut c_void) {
    let out = unsafe { &mut *(user_data as *mut Vec<String>) };
    if !extension.is_null() {
        let s = unsafe { CStr::from_ptr(extension) }
            .to_string_lossy()
            .to_string();
        out.push(s);
    }
}

/// Enumerate every file extension known to the scan engine.
pub fn enumerate_extensions() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    unsafe {
        kaifuku_enumerate_extensions(
            Some(collect_extension),
            &mut out as *mut Vec<String> as *mut c_void,
        );
    }
    out
}

/// Restrict the scan engine to the given file extensions.
/// An empty slice resets to the engine defaults (all formats enabled).
pub fn set_file_filter(extensions: &[String]) {
    if extensions.is_empty() {
        unsafe { kaifuku_set_file_filter(std::ptr::null(), 0) };
        return;
    }
    let cstrs: Vec<CString> = extensions
        .iter()
        .map(|e| CString::new(e.as_str()).unwrap_or_default())
        .collect();
    let ptrs: Vec<*const c_char> = cstrs.iter().map(|c| c.as_ptr()).collect();
    unsafe { kaifuku_set_file_filter(ptrs.as_ptr(), ptrs.len()) };
}

/// Toggle the brute-force fragmented-file reassembly pass. When enabled the
/// engine runs its `photorec_bf` pass after the normal carve (paranoia 2).
pub fn set_frag_reassembly(ctx: KaifukuCtx, enabled: bool) {
    if !ctx.is_null() {
        unsafe {
            kaifuku_set_frag_reassembly(ctx, enabled as c_int);
        }
    }
}

/// Toggle the filesystem-structure pass (FAT unformat). When enabled, files
/// are first recovered by following the FAT cluster table, then the normal
/// carve runs on the remaining space. No-op on non-FAT partitions.
pub fn set_filesystem_pass(ctx: KaifukuCtx, enabled: bool) {
    if !ctx.is_null() {
        unsafe {
            kaifuku_set_filesystem_pass(ctx, enabled as c_int);
        }
    }
}

/// Restrict the filesystem-structure pass to a single directory (Windows
/// style path, e.g. `\\Documents`). Pass `None` or empty to recover the whole
/// volume with the structure pass.
pub fn set_directory_filter(ctx: KaifukuCtx, dir_path: Option<&str>) {
    if ctx.is_null() {
        return;
    }
    let cstr = dir_path
        .filter(|s| !s.is_empty())
        .and_then(|s| CString::new(s).ok());
    let ptr = cstr
        .as_ref()
        .map(|c| c.as_ptr())
        .unwrap_or(std::ptr::null());
    unsafe {
        kaifuku_set_directory_filter(ctx, ptr);
    }
}

/// Read `buf.len()` bytes at absolute byte `offset` through the scan disk.
/// Returns the number of bytes actually read.
pub fn pread(ctx: KaifukuCtx, offset: u64, buf: &mut [u8]) -> Result<usize, String> {
    if ctx.is_null() {
        return Err("null scan context".to_string());
    }
    if buf.is_empty() {
        return Ok(0);
    }
    let r = unsafe { kaifuku_pread(ctx, offset, buf.as_mut_ptr(), buf.len()) };
    if r < 0 {
        Err("pread failed".to_string())
    } else {
        Ok(r as usize)
    }
}

/// Remove the given absolute byte ranges from the carve search space so the
/// carve pass skips them. Called from the ntfs_unformat callback.
pub fn mark_used_ranges(ctx: KaifukuCtx, ranges: &[KaifukuRange]) {
    if ctx.is_null() || ranges.is_empty() {
        return;
    }
    unsafe {
        kaifuku_mark_used_ranges(ctx, ranges.as_ptr(), ranges.len());
    }
}
