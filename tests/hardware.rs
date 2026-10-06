use std::fs;
use std::path::{Path, PathBuf};
use voidhw::plan::{self, Plan};
use voidhw::{profile, sysfs};

fn plan_for(root: &Path) -> Plan {
    let hw = sysfs::scan(root).unwrap();
    plan::build(&hw, &profile::embedded())
}

/// Build a throwaway sysfs/procfs tree: (pci address, vendor, device, class) and USB (vendor, product).
fn machine(
    name: &str,
    chassis: u32,
    cpu: &str,
    pci: &[(&str, u16, u16, u32)],
    usb: &[(u16, u16)],
) -> PathBuf {
    let root = std::env::temp_dir().join(format!("voidhw-it-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (address, vendor, device, class) in pci {
        let dir = root.join("sys/bus/pci/devices").join(address);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("vendor"), format!("0x{vendor:04x}\n")).unwrap();
        fs::write(dir.join("device"), format!("0x{device:04x}\n")).unwrap();
        fs::write(dir.join("class"), format!("0x{class:06x}\n")).unwrap();
    }
    for (i, (vendor, product)) in usb.iter().enumerate() {
        let dir = root
            .join("sys/bus/usb/devices")
            .join(format!("1-{}", i + 1));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("idVendor"), format!("{vendor:04x}\n")).unwrap();
        fs::write(dir.join("idProduct"), format!("{product:04x}\n")).unwrap();
    }
    fs::create_dir_all(root.join("sys/class/dmi/id")).unwrap();
    fs::write(
        root.join("sys/class/dmi/id/chassis_type"),
        format!("{chassis}\n"),
    )
    .unwrap();
    fs::create_dir_all(root.join("proc")).unwrap();
    fs::write(
        root.join("proc/cpuinfo"),
        format!("vendor_id\t: {cpu}\nmodel name\t: test\n"),
    )
    .unwrap();
    root
}

const VGA: u32 = 0x030000;
const AUDIO: u32 = 0x040300;
const WIFI: u32 = 0x028000;

fn has(plan: &Plan, package: &str) -> bool {
    plan.packages.iter().any(|p| p == package)
}

#[test]
fn real_notebook_snapshot_gets_nvidia_and_not_a_mesa_profile_for_its_audio_chip() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notebook-gtx1660ti-ax-wifi");
    let plan = plan_for(&root);
    assert!(
        plan.profiles.contains(&"nvidia-current".to_string()),
        "{:?}",
        plan.profiles
    );
    assert!(has(&plan, "nvidia"));
    assert!(plan.repos.contains(&"nonfree".to_string()));
    assert!(plan.profiles.contains(&"intel-microcode".to_string()));
    assert!(plan.profiles.contains(&"notebook-power".to_string()));
    assert!(
        !plan.profiles.iter().any(|p| p == "intel-graphics"),
        "the HD Audio controller must not match graphics"
    );
    assert!(
        !plan.hybrid_graphics,
        "this notebook runs on the NVIDIA GPU alone"
    );
}

#[test]
fn audio_controllers_never_match_graphics_profiles() {
    // Regression: the class register was sliced one byte off, so 0x040300 became "0300".
    let root = machine(
        "audio",
        3,
        "GenuineIntel",
        &[
            ("0000:00:1f.3", 0x8086, 0xa348, AUDIO),
            ("0000:01:00.1", 0x10de, 0x1aeb, AUDIO),
        ],
        &[],
    );
    let plan = plan_for(&root);
    assert!(plan.devices.is_empty(), "{:?}", plan.devices);
    assert!(!has(&plan, "mesa-dri") && !has(&plan, "nvidia"));
}

#[test]
fn hybrid_laptop_gets_both_graphics_stacks_and_switcheroo() {
    let root = machine(
        "hybrid",
        10,
        "GenuineIntel",
        &[
            ("0000:00:02.0", 0x8086, 0x3e9b, VGA),
            ("0000:01:00.0", 0x10de, 0x2191, VGA),
        ],
        &[],
    );
    let plan = plan_for(&root);
    assert!(plan.hybrid_graphics);
    for profile in [
        "intel-graphics",
        "nvidia-current",
        "hybrid-graphics",
        "notebook-power",
    ] {
        assert!(
            plan.profiles.contains(&profile.to_string()),
            "missing {profile}: {:?}",
            plan.profiles
        );
    }
    assert!(plan.services.contains(&"switcheroo-control".to_string()));
}

#[test]
fn amd_desktop_needs_no_nonfree_and_no_laptop_extras() {
    let root = machine(
        "amd",
        3,
        "AuthenticAMD",
        &[("0000:0a:00.0", 0x1002, 0x73bf, VGA)],
        &[],
    );
    let plan = plan_for(&root);
    assert!(plan.profiles.contains(&"amd-graphics".to_string()));
    assert!(plan.profiles.contains(&"amd-microcode".to_string()));
    assert!(!plan.profiles.contains(&"notebook-power".to_string()));
    assert!(!plan.hybrid_graphics);
    assert!(plan.repos.is_empty(), "{:?}", plan.repos);
}

#[test]
fn old_nvidia_gpus_get_the_right_legacy_branch() {
    let cases = [
        ("1c03", "nvidia-580", "nvidia580"),
        ("1180", "nvidia-470", "nvidia470"),
        ("1080", "nvidia-390", "nvidia390"),
    ];
    for (device, profile, package) in cases {
        let id = u16::from_str_radix(device, 16).unwrap();
        let root = machine(
            &format!("nv{device}"),
            3,
            "AuthenticAMD",
            &[("0000:01:00.0", 0x10de, id, VGA)],
            &[],
        );
        let plan = plan_for(&root);
        assert!(
            plan.profiles.contains(&profile.to_string()),
            "{device}: {:?}",
            plan.profiles
        );
        assert!(has(&plan, package), "{device}: {:?}", plan.packages);
        assert!(
            !has(&plan, "nvidia"),
            "{device} must not get the current driver"
        );
    }
}

#[test]
fn nvidia_gpu_no_void_driver_supports_stays_on_nouveau() {
    // 0x0640 is a GeForce 9500 GT, only supported by the 340.xx legacy driver, which Void does not ship.
    let root = machine(
        "nouveau",
        3,
        "AuthenticAMD",
        &[("0000:01:00.0", 0x10de, 0x0640, VGA)],
        &[],
    );
    let plan = plan_for(&root);
    assert_eq!(
        plan.profiles.first().map(String::as_str),
        Some("nouveau"),
        "{:?}",
        plan.profiles
    );
    assert!(has(&plan, "mesa-nouveau-dri"));
    assert!(!plan.repos.contains(&"nonfree".to_string()));
}

#[test]
fn two_gpus_on_the_same_profile_are_installed_once() {
    let root = machine(
        "two",
        3,
        "AuthenticAMD",
        &[
            ("0000:01:00.0", 0x10de, 0x2191, VGA),
            ("0000:02:00.0", 0x10de, 0x2191, VGA),
        ],
        &[],
    );
    let plan = plan_for(&root);
    assert_eq!(
        plan.profiles
            .iter()
            .filter(|p| *p == "nvidia-current")
            .count(),
        1
    );
    assert_eq!(plan.packages.iter().filter(|p| *p == "nvidia").count(), 1);
    assert_eq!(plan.devices.len(), 2, "both GPUs are reported");
}

#[test]
fn usb_wifi_adapters_get_their_dkms_driver() {
    let root = machine(
        "usb",
        3,
        "AuthenticAMD",
        &[],
        &[(0x0bda, 0x8812), (0x1d6b, 0x0002)],
    );
    let plan = plan_for(&root);
    assert!(plan.profiles.contains(&"rtl8812au".to_string()));
    assert!(has(&plan, "rtl8812au-dkms") && has(&plan, "dkms"));
}

#[test]
fn broadcom_wl_needs_nonfree_but_the_free_brcmfmac_cards_do_not() {
    let wl = plan_for(&machine(
        "bcm-wl",
        3,
        "AuthenticAMD",
        &[("0000:03:00.0", 0x14e4, 0x43a0, WIFI)],
        &[],
    ));
    assert!(has(&wl, "broadcom-wl-dkms") && wl.repos.contains(&"nonfree".to_string()));
    let free = plan_for(&machine(
        "bcm-free",
        3,
        "AuthenticAMD",
        &[("0000:03:00.0", 0x14e4, 0x43ba, WIFI)],
        &[],
    ));
    assert!(has(&free, "linux-firmware-broadcom") && !has(&free, "broadcom-wl-dkms"));
}

#[test]
fn a_machine_with_no_sysfs_yields_an_empty_plan() {
    let root = std::env::temp_dir().join(format!("voidhw-it-empty-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let plan = plan_for(&root);
    assert!(plan.profiles.is_empty() && plan.packages.is_empty());
    assert_eq!(plan.chassis, "unknown");
}

#[test]
fn a_device_with_unreadable_registers_is_skipped_not_fatal() {
    let root = machine(
        "broken",
        3,
        "AuthenticAMD",
        &[("0000:01:00.0", 0x10de, 0x2191, VGA)],
        &[],
    );
    let broken = root.join("sys/bus/pci/devices/0000:02:00.0");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("vendor"), "0x10de\n").unwrap(); // no device/class files
    let plan = plan_for(&root);
    assert!(plan.profiles.contains(&"nvidia-current".to_string()));
}

#[test]
fn snapshots_round_trip_and_omit_identifying_files() {
    let source = machine(
        "snap-src",
        10,
        "GenuineIntel",
        &[("0000:01:00.0", 0x10de, 0x2191, VGA)],
        &[],
    );
    fs::write(
        source.join("sys/bus/pci/devices/0000:01:00.0/serial"),
        "SECRET",
    )
    .unwrap();
    fs::write(source.join("sys/class/dmi/id/product_serial"), "SECRET").unwrap();
    let out = std::env::temp_dir().join(format!("voidhw-it-snap-out-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    voidhw::snapshot::write(&source, &out).unwrap();
    assert_eq!(
        plan_for(&out),
        plan_for(&source),
        "a snapshot must produce the same plan"
    );
    let leaked = fs::read_dir(out.join("sys/bus/pci/devices/0000:01:00.0"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name() == "serial");
    assert!(!leaked && !out.join("sys/class/dmi/id/product_serial").exists());
}

/// A VM: like `machine`, plus DMI strings and the CPU hypervisor flag.
fn vm(
    name: &str,
    sys_vendor: &str,
    product: &str,
    cpu: &str,
    pci: &[(&str, u16, u16, u32)],
) -> PathBuf {
    let root = machine(name, 1, cpu, pci, &[]);
    let dmi = root.join("sys/class/dmi/id");
    fs::write(dmi.join("sys_vendor"), format!("{sys_vendor}\n")).unwrap();
    fs::write(dmi.join("product_name"), format!("{product}\n")).unwrap();
    fs::write(
        root.join("proc/cpuinfo"),
        format!("vendor_id\t: {cpu}\nflags\t\t: fpu hypervisor\n"),
    )
    .unwrap();
    root
}

#[test]
fn vmware_guest_gets_tools_and_mesa_but_no_bare_metal_extras() {
    let root = vm(
        "vmware",
        "VMware, Inc.",
        "VMware Virtual Platform",
        "GenuineIntel",
        &[("0000:00:0f.0", 0x15ad, 0x0405, VGA)],
    );
    let plan = plan_for(&root);
    assert_eq!(plan.hypervisor.as_deref(), Some("vmware"));
    for profile in ["vm-vmware", "virtual-graphics"] {
        assert!(
            plan.profiles.contains(&profile.to_string()),
            "{profile}: {:?}",
            plan.profiles
        );
    }
    assert!(has(&plan, "open-vm-tools") && has(&plan, "mesa-dri"));
    assert!(plan.services.contains(&"vmtoolsd".to_string()));
    assert!(
        !plan.profiles.contains(&"intel-microcode".to_string()),
        "no microcode inside a VM"
    );
    assert!(!plan.profiles.contains(&"notebook-power".to_string()));
    assert!(plan.repos.is_empty(), "{:?}", plan.repos);
}

#[test]
fn virtualbox_guest_gets_the_guest_additions() {
    let root = vm(
        "vbox",
        "innotek GmbH",
        "VirtualBox",
        "AuthenticAMD",
        &[("0000:00:02.0", 0x80ee, 0xbeef, VGA)],
    );
    let plan = plan_for(&root);
    assert_eq!(plan.hypervisor.as_deref(), Some("virtualbox"));
    assert!(has(&plan, "virtualbox-ose-guest"));
    assert!(plan.services.contains(&"vboxservice".to_string()));
    assert!(plan.profiles.contains(&"virtual-graphics".to_string()));
}

#[test]
fn qemu_guest_gets_the_agent_and_spice() {
    let root = vm(
        "qemu",
        "QEMU",
        "Standard PC (Q35 + ICH9, 2009)",
        "GenuineIntel",
        &[("0000:00:01.0", 0x1af4, 0x1050, VGA)],
    );
    let plan = plan_for(&root);
    assert_eq!(plan.hypervisor.as_deref(), Some("qemu"));
    assert!(has(&plan, "qemu-ga") && has(&plan, "spice-vdagent"));
    assert!(plan.services.contains(&"spice-vdagentd".to_string()));
}

#[test]
fn an_unidentified_hypervisor_installs_no_guest_tools() {
    let root = vm("other", "Acme", "Cloud VM", "GenuineIntel", &[]);
    let plan = plan_for(&root);
    assert_eq!(plan.hypervisor.as_deref(), Some("other"));
    assert!(plan.packages.is_empty(), "{:?}", plan.packages);
}

#[test]
fn physical_machines_report_no_hypervisor() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notebook-gtx1660ti-ax-wifi");
    assert_eq!(plan_for(&root).hypervisor, None);
}
