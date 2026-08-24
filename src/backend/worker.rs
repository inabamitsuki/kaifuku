use crate::backend::photorec::{PhotoRecController, PhotoRecOptions};
use anyhow::Result;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub struct Worker {
    pub sender: Sender<WorkerCommand>,
    receiver: Receiver<WorkerEvent>,
}

impl Worker {
    pub fn new() -> Self {
        let (cmd_sender, cmd_receiver) = unbounded();
        let (event_sender, event_receiver) = unbounded();

        thread::spawn(move || {
            Self::worker_thread(cmd_receiver, event_sender);
        });

        Self {
            sender: cmd_sender,
            receiver: event_receiver,
        }
    }

    pub fn send_command(&self, command: WorkerCommand) -> Result<()> {
        self.sender
            .send(command)
            .map_err(|e| anyhow::anyhow!("Failed to send command: {}", e))?;
        Ok(())
    }

    pub fn receive_event(&self) -> Result<WorkerEvent> {
        self.receiver
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive event: {}", e))
    }

    pub fn has_event(&self) -> bool {
        !self.receiver.is_empty()
    }

    pub fn event_receiver(&self) -> Receiver<WorkerEvent> {
        self.receiver.clone()
    }

    fn worker_thread(receiver: Receiver<WorkerCommand>, event_sender: Sender<WorkerEvent>) {
        let controller = match PhotoRecController::new() {
            Ok(c) => Some(c),
            Err(e) => {
                let _ = event_sender.send(WorkerEvent::Error(e));
                return;
            }
        };

        let _ = event_sender.send(WorkerEvent::Ready);

        loop {
            match receiver.recv() {
                Ok(command) => match command {
                    WorkerCommand::StartScan {
                        device,
                        output_dir,
                        options,
                        part_offset,
                        part_size,
                        dd_path,
                        scan_type,
                    } => {
                        let mut scan_part_size = part_size;
                        if scan_type == "Quick"
                            && (scan_part_size == 0 || scan_part_size > 100_000_000)
                        {
                            scan_part_size = 100_000_000;
                        }
                        let scan_device = if let Some(ref dd_path) = dd_path {
                            let _ = event_sender.send(WorkerEvent::ScanStarted {
                                output_dir: output_dir.clone(),
                            });
                            match Self::create_dd_image(
                                &device,
                                dd_path,
                                scan_part_size,
                                &event_sender,
                                &receiver,
                            ) {
                                Ok(_) => {
                                    let _ = event_sender.send(WorkerEvent::ProgressUpdate {
                                        percent: 100,
                                        current_file: format!(
                                            "DD image ready, starting scan on {}",
                                            dd_path
                                        ),
                                        files_found: 0,
                                    });
                                    dd_path.clone()
                                }
                                Err(e) => {
                                    let _ = event_sender.send(WorkerEvent::Error(e));
                                    continue;
                                }
                            }
                        } else {
                            device.clone()
                        };

                        if let Some(ref ctrl) = controller {
                            match ctrl.start_scan(
                                &scan_device,
                                &output_dir,
                                &options,
                                event_sender.clone(),
                                part_offset,
                                scan_part_size,
                            ) {
                                Ok(_) => {
                                    let _ = event_sender.send(WorkerEvent::ScanStarted {
                                        output_dir: output_dir.clone(),
                                    });
                                    let mut stopped = false;
                                    loop {
                                        match receiver.try_recv() {
                                            Ok(WorkerCommand::StopScan) => {
                                                let _ = ctrl.stop();
                                                stopped = true;
                                            }
                                            Ok(WorkerCommand::Shutdown) => return,
                                            _ => {}
                                        }
                                        if !ctrl.is_running() {
                                            break;
                                        }
                                        thread::sleep(Duration::from_millis(200));
                                    }
                                    // Wait for the C scan thread to fully exit before freeing the
                                    // callback sender it still holds (prevents use-after-free).
                                    while ctrl.is_running() {
                                        thread::sleep(Duration::from_millis(100));
                                    }
                                    ctrl.reclaim_callbacks();
                                    if stopped {
                                        let _ = event_sender.send(WorkerEvent::ScanStopped);
                                    } else {
                                        let _ = event_sender.send(WorkerEvent::ScanComplete);
                                    }
                                }
                                Err(e) => {
                                    let _ = event_sender.send(WorkerEvent::Error(e));
                                }
                            }
                        } else {
                            let _ = event_sender
                                .send(WorkerEvent::Error("Controller not available".to_string()));
                        }
                    }
                    WorkerCommand::StopScan => {
                        if let Some(ref ctrl) = controller {
                            let _ = ctrl.stop();
                            let _ = event_sender.send(WorkerEvent::ScanStopped);
                        }
                    }
                    WorkerCommand::Shutdown => break,
                },
                Err(_) => break,
            }
        }
    }

    fn create_dd_image(
        source: &str,
        dest: &str,
        total_size: u64,
        event_sender: &Sender<WorkerEvent>,
        cmd_receiver: &Receiver<WorkerCommand>,
    ) -> Result<(), String> {
        let block_size = "4M";
        let dest_path = std::path::Path::new(dest);
        if let Some(parent) = dest_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create image directory {}: {}",
                        parent.display(),
                        e
                    )
                })?;
            }
        }
        let mut child = Command::new("dd")
            .arg(format!("if={}", source))
            .arg(format!("of={}", dest))
            .arg(format!("bs={}", block_size))
            .arg("conv=noerror,sync")
            .arg("status=none")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn dd: {}", e))?;

        let _ = event_sender.send(WorkerEvent::ProgressUpdate {
            percent: 0,
            current_file: format!("Creating DD image: {}", dest),
            files_found: 0,
        });

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        return Ok(());
                    }
                    return Err(format!("dd failed with exit code: {:?}", status.code()));
                }
                Ok(None) => {
                    if let Ok(WorkerCommand::StopScan) = cmd_receiver.try_recv() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("DD creation cancelled by user".to_string());
                    }
                    if let Ok(WorkerCommand::Shutdown) = cmd_receiver.try_recv() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("DD creation cancelled (shutdown)".to_string());
                    }
                    let copied = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                    let pct = if total_size > 0 {
                        ((copied * 100) / total_size).min(99) as u32
                    } else {
                        0
                    };
                    let _ = event_sender.send(WorkerEvent::ProgressUpdate {
                        percent: pct,
                        current_file: format!(
                            "Creating DD image: {} — copied {} of {} ({}%)",
                            dest,
                            fmt_bytes(copied),
                            fmt_bytes(total_size),
                            pct
                        ),
                        files_found: 0,
                    });
                    thread::sleep(Duration::from_millis(200));
                }
                Err(e) => {
                    return Err(format!("Failed to monitor dd: {}", e));
                }
            }
        }
    }
}

impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum WorkerCommand {
    StartScan {
        device: String,
        output_dir: String,
        options: PhotoRecOptions,
        part_offset: u64,
        part_size: u64,
        dd_path: Option<String>,
        scan_type: String,
    },
    StopScan,
    Shutdown,
}

fn fmt_bytes(s: u64) -> String {
    if s >= 1_000_000_000 {
        format!("{:.1} GB", s as f64 / 1_000_000_000.0)
    } else if s >= 1_000_000 {
        format!("{:.1} MB", s as f64 / 1_000_000.0)
    } else if s >= 1024 {
        format!("{:.1} KB", s as f64 / 1024.0)
    } else {
        format!("{} B", s)
    }
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready,
    ScanStarted {
        output_dir: String,
    },
    ProgressUpdate {
        percent: u32,
        current_file: String,
        files_found: u64,
    },
    FileFound {
        filename: String,
        extension: String,
        size: u64,
    },
    LogMessage(String),
    ScanComplete,
    ScanStopped,
    Error(String),
}
