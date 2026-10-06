use crate::nvidia;
use crate::profile::{FileSpec, Profile};
use crate::sysfs::{Hardware, PciDevice, UsbDevice};
use serde::Serialize;

/// One device and the profile chosen for it (if any).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceMatch {
    pub address: String,
    pub id: String,
    pub class: String,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub chassis: String,
    pub cpu_vendor: String,
    pub hybrid_graphics: bool,
    pub hypervisor: Option<String>,
    pub devices: Vec<DeviceMatch>,
    /// Profile ids in application order, each once.
    pub profiles: Vec<String>,
    pub repos: Vec<String>,
    pub packages: Vec<String>,
    pub services: Vec<String>,
    pub files: Vec<FileSpec>,
    pub notes: Vec<String>,
}

fn contains_ci(list: &[String], value: &str) -> bool {
    list.iter().any(|item| item.eq_ignore_ascii_case(value))
}

fn pci_matches(profile: &Profile, dev: &PciDevice) -> bool {
    let m = &profile.matches;
    if m.is_usb() || !m.is_device_level() {
        return false;
    }
    if !m.class.is_empty() && !contains_ci(&m.class, &dev.class_code()) {
        return false;
    }
    if !m.vendor.is_empty() && !contains_ci(&m.vendor, &dev.vendor_hex()) {
        return false;
    }
    if !m.device.is_empty() && !contains_ci(&m.device, &dev.device_hex()) {
        return false;
    }
    if m.nvidia_branch.is_some() || m.nvidia_unsupported {
        if dev.vendor != 0x10de {
            return false;
        }
        let branch = nvidia::branch_for(&dev.device_hex());
        if let Some(wanted) = &m.nvidia_branch {
            if branch != Some(wanted.as_str()) {
                return false;
            }
        }
        if m.nvidia_unsupported && branch.is_some() {
            return false;
        }
    }
    true
}

fn usb_matches(profile: &Profile, dev: &UsbDevice) -> bool {
    profile.matches.is_usb() && contains_ci(&profile.matches.usb, &dev.id())
}

fn system_matches(profile: &Profile, hw: &Hardware) -> bool {
    let m = &profile.matches;
    if m.is_device_level() {
        return false;
    }
    if !m.cpu_vendor.is_empty() && !contains_ci(&m.cpu_vendor, &hw.cpu_vendor) {
        return false;
    }
    if !m.chassis.is_empty() && !contains_ci(&m.chassis, hw.chassis.as_str()) {
        return false;
    }
    if let Some(hybrid) = m.hybrid_graphics {
        if hybrid != hw.hybrid_graphics() {
            return false;
        }
    }
    if !m.vm.is_empty()
        && !hw
            .hypervisor
            .is_some_and(|h| contains_ci(&m.vm, h.as_str()))
    {
        return false;
    }
    if let Some(bare_metal) = m.bare_metal {
        if bare_metal == hw.hypervisor.is_some() {
            return false;
        }
    }
    // A profile with no criteria at all must not apply to every machine.
    !(m.cpu_vendor.is_empty()
        && m.chassis.is_empty()
        && m.hybrid_graphics.is_none()
        && m.vm.is_empty()
        && m.bare_metal.is_none())
}

fn best<'a>(candidates: impl Iterator<Item = &'a Profile>) -> Option<&'a Profile> {
    // max_by_key keeps the last maximum; reverse so the first-listed profile wins ties.
    candidates
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .max_by_key(|p| p.priority)
}

fn push_unique<T: PartialEq + Clone>(list: &mut Vec<T>, items: &[T]) {
    for item in items {
        if !list.contains(item) {
            list.push(item.clone());
        }
    }
}

/// Decide what the machine needs. Pure: reads nothing and changes nothing.
pub fn build(hw: &Hardware, profiles: &[Profile]) -> Plan {
    let mut plan = Plan {
        chassis: hw.chassis.as_str().to_string(),
        cpu_vendor: hw.cpu_vendor.clone(),
        hybrid_graphics: hw.hybrid_graphics(),
        hypervisor: hw.hypervisor.map(|h| h.as_str().to_string()),
        ..Plan::default()
    };
    let mut chosen: Vec<&Profile> = Vec::new();
    for dev in &hw.pci {
        let picked = best(profiles.iter().filter(|p| pci_matches(p, dev)));
        // Report GPUs even when nothing matched, so the user sees what was looked at.
        if picked.is_some() || dev.is_gpu() {
            plan.devices.push(DeviceMatch {
                address: dev.address.clone(),
                id: dev.id(),
                class: dev.class_code(),
                profile: picked.map(|p| p.id.clone()),
            });
        }
        if let Some(p) = picked {
            if !chosen.iter().any(|c| c.id == p.id) {
                chosen.push(p);
            }
        }
    }
    for dev in &hw.usb {
        if let Some(p) = best(profiles.iter().filter(|p| usb_matches(p, dev))) {
            plan.devices.push(DeviceMatch {
                address: dev.address.clone(),
                id: dev.id(),
                class: "usb".to_string(),
                profile: Some(p.id.clone()),
            });
            if !chosen.iter().any(|c| c.id == p.id) {
                chosen.push(p);
            }
        }
    }
    for p in profiles.iter().filter(|p| system_matches(p, hw)) {
        if !chosen.iter().any(|c| c.id == p.id) {
            chosen.push(p);
        }
    }

    for p in chosen {
        plan.profiles.push(p.id.clone());
        push_unique(&mut plan.repos, &p.repos);
        push_unique(&mut plan.packages, &p.packages);
        push_unique(&mut plan.services, &p.services);
        push_unique(&mut plan.files, &p.files);
        push_unique(&mut plan.notes, &p.notes);
    }
    plan
}
