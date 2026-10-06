use serde::Serialize;
use std::fs;
use std::io;
use std::path::Path;

/// A PCI device. `class` is the full 24-bit class register from sysfs (`0x030000` for a VGA controller).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PciDevice {
    pub address: String,
    pub vendor: u16,
    pub device: u16,
    pub class: u32,
}

impl PciDevice {
    pub fn base_class(&self) -> u8 {
        (self.class >> 16) as u8
    }

    /// Base class and subclass as four hex digits ("0300"), the form used in profiles.
    pub fn class_code(&self) -> String {
        format!("{:04x}", self.class >> 8)
    }

    pub fn vendor_hex(&self) -> String {
        format!("{:04x}", self.vendor)
    }

    pub fn device_hex(&self) -> String {
        format!("{:04x}", self.device)
    }

    pub fn id(&self) -> String {
        format!("{}:{}", self.vendor_hex(), self.device_hex())
    }

    /// Display controllers: VGA (00), 3D (02) and "other" (80) subclasses of base class 03.
    pub fn is_gpu(&self) -> bool {
        self.base_class() == 0x03 && matches!((self.class >> 8) & 0xff, 0x00 | 0x02 | 0x80)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsbDevice {
    pub address: String,
    pub vendor: u16,
    pub product: u16,
}

impl UsbDevice {
    pub fn id(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor, self.product)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Chassis {
    Notebook,
    Desktop,
    Unknown,
}

impl Chassis {
    pub fn as_str(self) -> &'static str {
        match self {
            Chassis::Notebook => "notebook",
            Chassis::Desktop => "desktop",
            Chassis::Unknown => "unknown",
        }
    }

    /// SMBIOS chassis types: 8 portable, 9 laptop, 10 notebook, 11 handheld, 14 sub-notebook,
    /// 30 tablet, 31 convertible, 32 detachable.
    fn from_smbios(value: u32) -> Self {
        match value {
            8 | 9 | 10 | 11 | 14 | 30 | 31 | 32 => Chassis::Notebook,
            3 | 4 | 5 | 6 | 7 | 13 | 15 | 16 | 17 | 23 | 24 | 35 | 36 => Chassis::Desktop,
            _ => Chassis::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Hypervisor {
    Vmware,
    Virtualbox,
    Qemu,
    Hyperv,
    Xen,
    Parallels,
    /// A hypervisor is present (CPU flag) but it is none of the above.
    Other,
}

impl Hypervisor {
    pub fn as_str(self) -> &'static str {
        match self {
            Hypervisor::Vmware => "vmware",
            Hypervisor::Virtualbox => "virtualbox",
            Hypervisor::Qemu => "qemu",
            Hypervisor::Hyperv => "hyperv",
            Hypervisor::Xen => "xen",
            Hypervisor::Parallels => "parallels",
            Hypervisor::Other => "other",
        }
    }
}

/// Identify the hypervisor the way `systemd-detect-virt` does for full VMs: DMI vendor/product strings first,
/// then PCI vendor ids of the virtual devices, then the CPU's `hypervisor` flag as a last resort.
pub fn detect_hypervisor(
    dmi: &[&str],
    pci: &[PciDevice],
    cpu_hypervisor_flag: bool,
) -> Option<Hypervisor> {
    let has = |needle: &str| {
        dmi.iter().any(|s| {
            s.to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        })
    };
    // Short names must be whole words so "Xenon" or "Okvm" on real hardware do not match.
    let has_word = |word: &str| {
        dmi.iter().any(|s| {
            s.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|token| token.eq_ignore_ascii_case(word))
        })
    };
    if has("vmware") {
        return Some(Hypervisor::Vmware);
    }
    if has("virtualbox") || has("innotek") {
        return Some(Hypervisor::Virtualbox);
    }
    if has("qemu") || has_word("kvm") || has_word("bochs") {
        return Some(Hypervisor::Qemu);
    }
    if has("parallels") {
        return Some(Hypervisor::Parallels);
    }
    if has_word("xen") {
        return Some(Hypervisor::Xen);
    }
    if dmi.iter().any(|s| s.contains("Microsoft")) && has("virtual machine") {
        return Some(Hypervisor::Hyperv);
    }
    for dev in pci {
        match dev.vendor {
            0x15ad => return Some(Hypervisor::Vmware),
            0x80ee => return Some(Hypervisor::Virtualbox),
            0x1af4 | 0x1b36 | 0x1234 => return Some(Hypervisor::Qemu),
            0x1414 => return Some(Hypervisor::Hyperv),
            _ => {}
        }
    }
    cpu_hypervisor_flag.then_some(Hypervisor::Other)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hardware {
    pub pci: Vec<PciDevice>,
    pub usb: Vec<UsbDevice>,
    pub chassis: Chassis,
    /// `vendor_id` from /proc/cpuinfo: GenuineIntel, AuthenticAMD, ...
    pub cpu_vendor: String,
    /// `Some` when running inside a virtual machine.
    pub hypervisor: Option<Hypervisor>,
}

impl Hardware {
    pub fn gpus(&self) -> impl Iterator<Item = &PciDevice> {
        self.pci.iter().filter(|d| d.is_gpu())
    }

    /// An integrated Intel/AMD GPU together with an NVIDIA GPU (Optimus-style laptops).
    pub fn hybrid_graphics(&self) -> bool {
        let has = |vendor: u16| self.gpus().any(|g| g.vendor == vendor);
        has(0x10de) && (has(0x8086) || has(0x1002))
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Parse a sysfs hex value with or without a `0x` prefix.
fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.trim().trim_start_matches("0x");
    u32::from_str_radix(digits, 16).ok()
}

fn read_hex(path: &Path) -> Option<u32> {
    read_trimmed(path).and_then(|s| parse_hex(&s))
}

/// Read the hardware description from a sysfs/procfs tree rooted at `root` ("/" on a live system).
/// Devices whose registers cannot be read are skipped rather than failing the whole scan.
pub fn scan(root: &Path) -> io::Result<Hardware> {
    let mut pci = Vec::new();
    if let Ok(entries) = fs::read_dir(root.join("sys/bus/pci/devices")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let (Some(vendor), Some(device), Some(class)) = (
                read_hex(&path.join("vendor")),
                read_hex(&path.join("device")),
                read_hex(&path.join("class")),
            ) else {
                continue;
            };
            pci.push(PciDevice {
                address: entry.file_name().to_string_lossy().into_owned(),
                vendor: vendor as u16,
                device: device as u16,
                class,
            });
        }
    }
    pci.sort_by(|a, b| a.address.cmp(&b.address));

    let mut usb = Vec::new();
    if let Ok(entries) = fs::read_dir(root.join("sys/bus/usb/devices")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let (Some(vendor), Some(product)) = (
                read_hex(&path.join("idVendor")),
                read_hex(&path.join("idProduct")),
            ) else {
                continue;
            };
            usb.push(UsbDevice {
                address: entry.file_name().to_string_lossy().into_owned(),
                vendor: vendor as u16,
                product: product as u16,
            });
        }
    }
    usb.sort_by(|a, b| a.address.cmp(&b.address));

    let chassis = read_trimmed(&root.join("sys/class/dmi/id/chassis_type"))
        .and_then(|s| s.parse::<u32>().ok())
        .map(Chassis::from_smbios)
        .unwrap_or(Chassis::Unknown);

    let cpuinfo = read_trimmed(&root.join("proc/cpuinfo")).unwrap_or_default();
    let cpu_vendor = cpuinfo
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "vendor_id").then(|| value.trim().to_string())
        })
        .unwrap_or_default();
    let cpu_hypervisor_flag = cpuinfo.lines().any(|line| {
        line.split_once(':').is_some_and(|(key, value)| {
            key.trim() == "flags" && value.split_whitespace().any(|f| f == "hypervisor")
        })
    });

    let dmi: Vec<String> = ["sys_vendor", "product_name", "bios_vendor"]
        .iter()
        .filter_map(|name| read_trimmed(&root.join("sys/class/dmi/id").join(name)))
        .collect();
    let dmi_refs: Vec<&str> = dmi.iter().map(String::as_str).collect();
    let hypervisor = detect_hypervisor(&dmi_refs, &pci, cpu_hypervisor_flag);

    Ok(Hardware {
        pci,
        usb,
        chassis,
        cpu_vendor,
        hypervisor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_code_is_base_and_subclass_not_subclass_and_prog_if() {
        // sysfs reports 0x030000 for a VGA controller and 0x040300 for HD Audio.
        let gpu = PciDevice {
            address: "0000:01:00.0".into(),
            vendor: 0x10de,
            device: 0x2191,
            class: 0x030000,
        };
        let audio = PciDevice {
            address: "0000:00:1f.3".into(),
            vendor: 0x8086,
            device: 0xa348,
            class: 0x040300,
        };
        assert_eq!(gpu.class_code(), "0300");
        assert!(gpu.is_gpu());
        assert_eq!(audio.class_code(), "0403");
        assert!(
            !audio.is_gpu(),
            "an audio controller must never be treated as a GPU"
        );
    }

    #[test]
    fn gpu_subclasses() {
        let dev = |class| PciDevice {
            address: String::new(),
            vendor: 0x8086,
            device: 0,
            class,
        };
        assert!(dev(0x030000).is_gpu());
        assert!(dev(0x030200).is_gpu(), "3D controller");
        assert!(dev(0x038000).is_gpu(), "other display controller");
        assert!(!dev(0x030100).is_gpu(), "XGA is not a GPU we configure");
        assert!(!dev(0x020000).is_gpu());
    }

    #[test]
    fn hex_parsing_accepts_prefix_and_bare_digits() {
        assert_eq!(parse_hex("0x10de\n"), Some(0x10de));
        assert_eq!(parse_hex("1d6b"), Some(0x1d6b));
        assert_eq!(parse_hex("zz"), None);
    }

    fn pci(vendor: u16) -> PciDevice {
        PciDevice {
            address: String::new(),
            vendor,
            device: 0,
            class: 0x030000,
        }
    }

    #[test]
    fn hypervisors_are_identified_from_dmi_strings() {
        let detect = |dmi: &[&str]| detect_hypervisor(dmi, &[], true);
        assert_eq!(
            detect(&["VMware, Inc.", "VMware Virtual Platform"]),
            Some(Hypervisor::Vmware)
        );
        assert_eq!(
            detect(&["innotek GmbH", "VirtualBox"]),
            Some(Hypervisor::Virtualbox)
        );
        assert_eq!(
            detect(&["QEMU", "Standard PC (Q35 + ICH9, 2009)"]),
            Some(Hypervisor::Qemu)
        );
        assert_eq!(
            detect(&["Microsoft Corporation", "Virtual Machine"]),
            Some(Hypervisor::Hyperv)
        );
        assert_eq!(detect(&["Xen", "HVM domU"]), Some(Hypervisor::Xen));
        assert_eq!(
            detect(&["Parallels Software International Inc."]),
            Some(Hypervisor::Parallels)
        );
    }

    #[test]
    fn hypervisor_falls_back_to_pci_vendors_then_the_cpu_flag() {
        assert_eq!(
            detect_hypervisor(&["Some Vendor"], &[pci(0x15ad)], false),
            Some(Hypervisor::Vmware)
        );
        assert_eq!(
            detect_hypervisor(&[], &[pci(0x80ee)], false),
            Some(Hypervisor::Virtualbox)
        );
        assert_eq!(
            detect_hypervisor(&[], &[pci(0x8086)], true),
            Some(Hypervisor::Other)
        );
    }

    #[test]
    fn real_hardware_is_not_a_vm() {
        assert_eq!(
            detect_hypervisor(
                &["LENOVO", "20XW", "LENOVO"],
                &[pci(0x8086), pci(0x10de)],
                false
            ),
            None
        );
        assert_eq!(
            detect_hypervisor(&["Microsoft Corporation", "Surface Laptop 4"], &[], false),
            None,
            "a Surface is not Hyper-V"
        );
        assert_eq!(
            detect_hypervisor(&["Xenon Systems", "Okvm-1000"], &[], false),
            None,
            "substrings of real vendors"
        );
    }

    #[test]
    fn smbios_chassis_types() {
        assert_eq!(Chassis::from_smbios(10), Chassis::Notebook);
        assert_eq!(Chassis::from_smbios(31), Chassis::Notebook);
        assert_eq!(Chassis::from_smbios(3), Chassis::Desktop);
        assert_eq!(Chassis::from_smbios(1), Chassis::Unknown);
    }
}
