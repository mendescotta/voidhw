use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// A file a profile wants on the target system (modprobe/dracut snippets).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileSpec {
    pub path: String,
    pub content: String,
}

/// What a profile matches. Every field that is set must hold; list fields match any element.
/// Device-level criteria (`class`, `vendor`, `device`, `usb`, `nvidia_branch`) select devices; a profile
/// with only system-level criteria (`cpu_vendor`, `chassis`, `hybrid_graphics`) applies to the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Match {
    /// PCI base class + subclass, four hex digits ("0300").
    pub class: Vec<String>,
    /// PCI vendor ids, four hex digits ("10de").
    pub vendor: Vec<String>,
    /// PCI device ids, four hex digits.
    pub device: Vec<String>,
    /// USB ids as "vendor:product".
    pub usb: Vec<String>,
    /// Only NVIDIA GPUs served by this driver branch ("current", "580", "470", "390").
    pub nvidia_branch: Option<String>,
    /// NVIDIA GPUs no Void driver supports.
    pub nvidia_unsupported: bool,
    pub cpu_vendor: Vec<String>,
    /// "notebook" or "desktop".
    pub chassis: Vec<String>,
    pub hybrid_graphics: Option<bool>,
    /// Only inside these hypervisors ("vmware", "virtualbox", "qemu", "hyperv", "xen", "parallels", "other").
    pub vm: Vec<String>,
    /// `true`: only on physical hardware; `false`: only inside a VM.
    pub bare_metal: Option<bool>,
}

impl Match {
    pub fn is_device_level(&self) -> bool {
        !(self.class.is_empty()
            && self.vendor.is_empty()
            && self.device.is_empty()
            && self.usb.is_empty()
            && self.nvidia_branch.is_none()
            && !self.nvidia_unsupported)
    }

    pub fn is_usb(&self) -> bool {
        !self.usb.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub desc: String,
    /// Among the profiles matching one device, the highest priority wins.
    pub priority: i32,
    #[serde(rename = "match")]
    pub matches: Match,
    pub packages: Vec<String>,
    /// Extra repositories the packages come from ("nonfree").
    pub repos: Vec<String>,
    /// runit services to enable (must exist as /etc/sv/<name> once the packages are installed).
    pub services: Vec<String>,
    pub files: Vec<FileSpec>,
    /// Advice shown to the user, never executed.
    pub notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProfileFile {
    #[serde(default)]
    profile: Vec<Profile>,
}

const EMBEDDED: [(&str, &str); 5] = [
    ("gpu.toml", include_str!("../data/profiles/gpu.toml")),
    ("nvidia.toml", include_str!("../data/profiles/nvidia.toml")),
    (
        "network.toml",
        include_str!("../data/profiles/network.toml"),
    ),
    ("system.toml", include_str!("../data/profiles/system.toml")),
    ("vm.toml", include_str!("../data/profiles/vm.toml")),
];

pub fn parse(text: &str) -> Result<Vec<Profile>, String> {
    toml::from_str::<ProfileFile>(text)
        .map(|file| file.profile)
        .map_err(|e| e.to_string())
}

/// The profiles shipped inside the binary.
pub fn embedded() -> Vec<Profile> {
    let mut all = Vec::new();
    for (name, text) in EMBEDDED {
        all.extend(
            parse(text).unwrap_or_else(|e| panic!("embedded profile {name} is invalid: {e}")),
        );
    }
    all
}

/// Profiles from `*.toml` files in `dir`, sorted by file name. A profile whose `id` matches an
/// embedded one replaces it, so local files can override or extend the defaults.
pub fn with_overrides(mut base: Vec<Profile>, dir: &Path) -> Result<Vec<Profile>, String> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();
    for path in files {
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        for profile in parse(&text).map_err(|e| format!("{}: {e}", path.display()))? {
            base.retain(|existing| existing.id != profile.id);
            base.push(profile);
        }
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_profiles_parse_and_have_unique_ids() {
        let profiles = embedded();
        assert!(!profiles.is_empty());
        let mut ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate profile ids");
        assert!(profiles
            .iter()
            .all(|p| !p.id.is_empty() && !p.desc.is_empty()));
    }

    #[test]
    fn every_embedded_profile_has_a_criterion_and_an_effect() {
        for p in embedded() {
            let m = &p.matches;
            let has_criterion = m.is_device_level()
                || !m.cpu_vendor.is_empty()
                || !m.chassis.is_empty()
                || m.hybrid_graphics.is_some()
                || !m.vm.is_empty()
                || m.bare_metal.is_some();
            assert!(has_criterion, "{} matches everything", p.id);
            assert!(
                !p.packages.is_empty() || !p.files.is_empty() || !p.notes.is_empty(),
                "{} does nothing",
                p.id
            );
        }
    }

    #[test]
    fn ids_in_profiles_are_lowercase_four_digit_hex() {
        let hex4 = |s: &String| {
            s.len() == 4
                && s.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        };
        for p in embedded() {
            for id in p
                .matches
                .class
                .iter()
                .chain(&p.matches.vendor)
                .chain(&p.matches.device)
            {
                assert!(
                    hex4(id),
                    "{}: '{id}' is not four lowercase hex digits",
                    p.id
                );
            }
            for usb in &p.matches.usb {
                let (v, d) = usb
                    .split_once(':')
                    .unwrap_or_else(|| panic!("{}: bad usb id {usb}", p.id));
                assert!(
                    hex4(&v.to_string()) && hex4(&d.to_string()),
                    "{}: bad usb id {usb}",
                    p.id
                );
            }
        }
    }

    #[test]
    fn local_profiles_override_by_id() {
        let dir = std::env::temp_dir().join(format!("voidhw-profiles-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("local.toml"),
            "[[profile]]\nid = \"intel-graphics\"\ndesc = \"mine\"\npriority = 99\npackages = [\"mesa-dri\"]\n[profile.match]\nvendor = [\"8086\"]\nclass = [\"0300\"]\n",
        )
        .unwrap();
        let merged = with_overrides(embedded(), &dir).unwrap();
        let mine: Vec<_> = merged.iter().filter(|p| p.id == "intel-graphics").collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].desc, "mine");
        fs::remove_dir_all(&dir).ok();
    }
}
