use std::io::ErrorKind;
use std::process::{Command, Stdio};

pub fn has_device_access() -> bool {
    let candidates = [
        "/dev/sda",
        "/dev/sdb",
        "/dev/nvme0n1",
        "/dev/nvme0n2",
        "/dev/hda",
        "/dev/mmcblk0",
        "/dev/vda",
        "/dev/xvda",
    ];
    for dev in &candidates {
        match std::fs::OpenOptions::new().read(true).open(dev) {
            Ok(f) => {
                drop(f);
                return true;
            }
            Err(e) if e.kind() == ErrorKind::PermissionDenied => return false,
            _ => continue,
        }
    }
    false
}

/// Attempt to obtain root privileges. Returns `true` if the process has device
/// access (already elevated). If an elevation helper was launched, this function
/// waits for it and exits, mirroring its exit code. If no helper is available the
/// user is warned and `false` is returned.
pub fn try_elevate() -> bool {
    if has_device_access() {
        return true;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut child = try_lxqt_sudo(&exe, &args).or_else(|| try_pkexec_env(&exe, &args));
    if let Some(ref mut child) = child {
        // Elevated process launched successfully — wait for it, then mirror exit code
        let status = child.wait().ok();
        let code = status.and_then(|s| s.code()).unwrap_or(0);
        std::process::exit(code);
    }

    warn_no_elevation();
    false
}

fn warn_no_elevation() {
    let msg = "Kaifuku needs root privileges to access storage devices, but no privilege \
               elevation tool (lxqt-sudo or pkexec) was found.\n\n\
               Please run the application as root, or install lxqt-sudo or pkexec and try again.";
    let shown = Command::new("zenity")
        .args(["--error", "--title=Kaifuku", &format!("--text={}", msg)])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        || Command::new("kdialog")
            .args(["--error", msg])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    if !shown {
        eprintln!("ERROR: {}", msg);
    }
}

fn try_lxqt_sudo(exe: &std::path::Path, args: &[String]) -> Option<std::process::Child> {
    let mut cmd = Command::new("lxqt-sudo");
    cmd.arg(exe);
    for a in args {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.spawn().ok()
}

fn try_pkexec_env(exe: &std::path::Path, args: &[String]) -> Option<std::process::Child> {
    let mut cmd = Command::new("pkexec");
    cmd.arg("env");
    if let Ok(d) = std::env::var("DISPLAY") {
        cmd.arg(format!("DISPLAY={}", d));
    }
    if let Ok(w) = std::env::var("WAYLAND_DISPLAY") {
        cmd.arg(format!("WAYLAND_DISPLAY={}", w));
    }
    if let Ok(x) = std::env::var("XAUTHORITY") {
        cmd.arg(format!("XAUTHORITY={}", x));
    }
    cmd.arg(exe);
    for a in args {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.spawn().ok()
}
