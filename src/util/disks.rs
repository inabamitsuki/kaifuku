use std::path::Path;

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub device: String,
    pub model: String,
    pub capacity: u64,
    pub sector_size: u64,
    pub removable: bool,
}

#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub offset: u64,
    pub size: u64,
    pub name: String,
}

pub fn enumerate_disks() -> Vec<DiskInfo> {
    let mut disks = Vec::new();
    let sys_block = Path::new("/sys/block");

    let entries = match std::fs::read_dir(sys_block) {
        Ok(e) => e,
        Err(_) => return disks,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();

        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("dm-") {
            continue;
        }

        let dev_path = format!("/dev/{}", name);
        if !Path::new(&dev_path).exists() {
            continue;
        }

        let capacity = read_disk_capacity(&entry.path()).unwrap_or(0);
        if capacity == 0 {
            continue;
        }
        let model =
            read_first_line(entry.path().join("device/model")).unwrap_or_else(|| name.clone());
        let sector_size = read_first_line(entry.path().join("queue/hw_sector_size"))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(512);

        let removable = read_first_line(entry.path().join("removable"))
            .map(|s| s == "1")
            .unwrap_or(false);

        disks.push(DiskInfo {
            device: dev_path,
            model,
            capacity,
            sector_size,
            removable,
        });
    }

    disks.sort_by(|a, b| a.device.cmp(&b.device));
    disks
}

pub fn enumerate_partitions(device: &str) -> Vec<PartitionInfo> {
    let mut partitions = Vec::new();
    let dev_name = device.strip_prefix("/dev/").unwrap_or(device);
    let sys_dir = Path::new("/sys/block").join(dev_name);

    let entries = match std::fs::read_dir(&sys_dir) {
        Ok(e) => e,
        Err(_) => return partitions,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();

        // Partition entries start with the disk name (e.g., sda1, nvme0n1p1)
        if !name.starts_with(dev_name) || name == dev_name {
            continue;
        }

        // Check it's a partition (has a 'start' file)
        let start_path = entry.path().join("start");
        if !start_path.exists() {
            continue;
        }

        let start = match std::fs::read_to_string(&start_path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            Some(v) => v,
            None => continue,
        };

        let size = match std::fs::read_to_string(entry.path().join("size"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            Some(v) => v,
            None => continue,
        };

        let label = read_first_line(entry.path().join("uevent"))
            .map(|s| {
                s.lines()
                    .find(|l| l.starts_with("PARTNAME="))
                    .map(|l| l.trim_start_matches("PARTNAME=").to_string())
                    .unwrap_or_else(|| name.clone())
            })
            .unwrap_or_else(|| name.clone());

        partitions.push(PartitionInfo {
            offset: start * 512,
            size: size * 512,
            name: label,
        });
    }

    partitions.sort_by(|a, b| a.offset.cmp(&b.offset));
    partitions
}

fn read_disk_capacity(dev_entry: &Path) -> Option<u64> {
    let size_path = dev_entry.join("size");
    std::fs::read_to_string(size_path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|sectors| sectors * 512)
}

fn read_first_line(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
