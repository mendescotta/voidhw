//! Copy the few sysfs/procfs files detection reads into a directory, so a machine can be
//! reproduced in tests or attached to a bug report. Serial numbers and other identifying
//! files are never copied.

use std::fs;
use std::io;
use std::path::Path;

const PCI_FILES: [&str; 3] = ["vendor", "device", "class"];
const USB_FILES: [&str; 2] = ["idVendor", "idProduct"];

fn copy_files(src_dir: &Path, dst_dir: &Path, names: &[&str]) -> io::Result<()> {
    fs::create_dir_all(dst_dir)?;
    for name in names {
        if let Ok(content) = fs::read(src_dir.join(name)) {
            fs::write(dst_dir.join(name), content)?;
        }
    }
    Ok(())
}

/// Write a minimal sysfs/procfs tree for the machine rooted at `root` into `out`.
pub fn write(root: &Path, out: &Path) -> io::Result<()> {
    for (bus, files) in [("pci", &PCI_FILES[..]), ("usb", &USB_FILES[..])] {
        let dir = root.join("sys/bus").join(bus).join("devices");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let dst = out
                .join("sys/bus")
                .join(bus)
                .join("devices")
                .join(entry.file_name());
            copy_files(&entry.path(), &dst, files)?;
        }
    }
    copy_files(
        &root.join("sys/class/dmi/id"),
        &out.join("sys/class/dmi/id"),
        &["chassis_type", "sys_vendor", "product_name", "bios_vendor"],
    )?;

    // Only the CPU vendor, model and whether the hypervisor flag is set (first processor).
    if let Ok(cpuinfo) = fs::read_to_string(root.join("proc/cpuinfo")) {
        let kept: Vec<&str> = cpuinfo
            .lines()
            .take_while(|line| !line.trim().is_empty())
            .filter(|line| line.starts_with("vendor_id") || line.starts_with("model name"))
            .collect();
        let hypervisor = cpuinfo
            .lines()
            .take_while(|line| !line.trim().is_empty())
            .any(|line| {
                line.starts_with("flags") && line.split_whitespace().any(|f| f == "hypervisor")
            });
        let mut text = kept.join("\n") + "\n";
        if hypervisor {
            text.push_str("flags\t\t: hypervisor\n");
        }
        fs::create_dir_all(out.join("proc"))?;
        fs::write(out.join("proc/cpuinfo"), text)?;
    }
    Ok(())
}
