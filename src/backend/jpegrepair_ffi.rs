use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uchar, c_ulong, c_void};

extern "C" {
    fn jp_repair_mem(
        inbuf: *const c_uchar,
        inlen: c_ulong,
        outbuf: *mut *mut c_uchar,
        outlen: *mut c_ulong,
        op_count: c_int,
        ops: *const *const c_char,
    ) -> c_int;
    fn free(ptr: *mut c_void);
}

/// Apply a jpegrepair operation sequence to an in-memory JPEG.
///
/// `ops` is a flat token list, e.g. `["dest", "0", "0", "delete", "1"]`.
/// Returns the repaired JPEG bytes on success, or `None` if the source could
/// not be decoded / re-encoded (i.e. the damaged file was not a usable JPEG
/// coefficient stream).
pub fn jpegrepair_mem(input: &[u8], ops: &[&str]) -> Option<Vec<u8>> {
    if input.is_empty() || ops.is_empty() {
        return None;
    }

    let tokens: Vec<CString> = ops
        .iter()
        .map(|s| CString::new(*s).ok())
        .collect::<Option<_>>()?;
    let ptrs: Vec<*const c_char> = tokens.iter().map(|t| t.as_ptr()).collect();

    let mut outbuf: *mut c_uchar = std::ptr::null_mut();
    let mut outlen: c_ulong = 0;

    let ret = unsafe {
        jp_repair_mem(
            input.as_ptr(),
            input.len() as c_ulong,
            &mut outbuf,
            &mut outlen,
            ptrs.len() as c_int,
            ptrs.as_ptr(),
        )
    };

    if ret != 0 || outbuf.is_null() || outlen == 0 {
        unsafe {
            if !outbuf.is_null() {
                free(outbuf as *mut c_void);
            }
        }
        return None;
    }

    let slice = unsafe { std::slice::from_raw_parts(outbuf as *const u8, outlen as usize) };
    let out = slice.to_vec();
    unsafe {
        free(outbuf as *mut c_void);
    }
    Some(out)
}
