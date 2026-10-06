//! Which Void NVIDIA driver branch serves a GPU, from NVIDIA's own supported-chips lists
//! (generated into data/nvidia-branches.toml by tools/gen-nvidia-table.py).

use std::collections::HashMap;
use std::sync::OnceLock;

/// Newest first: the first branch listing the device wins.
pub const BRANCHES: [&str; 4] = ["current", "580", "470", "390"];

static TABLE_TOML: &str = include_str!("../data/nvidia-branches.toml");

fn table() -> &'static HashMap<String, Vec<String>> {
    static TABLE: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    TABLE.get_or_init(|| toml::from_str(TABLE_TOML).expect("data/nvidia-branches.toml is valid"))
}

/// The driver branch for an NVIDIA PCI device id (four lowercase hex digits), or `None` for GPUs
/// no Void driver supports (they stay on nouveau).
pub fn branch_for(device_hex: &str) -> Option<&'static str> {
    let device = device_hex.to_ascii_lowercase();
    let table = table();
    BRANCHES.into_iter().find(|branch| {
        table
            .get(*branch)
            .is_some_and(|ids| ids.iter().any(|id| *id == device))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turing_gets_the_current_driver() {
        assert_eq!(branch_for("2191"), Some("current"), "GTX 1660 Ti Mobile");
        assert_eq!(branch_for("2684"), Some("current"), "RTX 4090");
    }

    #[test]
    fn pascal_and_maxwell_are_on_the_580_branch() {
        assert_eq!(branch_for("1c03"), Some("580"), "GTX 1060 6GB");
        assert_eq!(branch_for("13c2"), Some("580"), "GTX 970");
    }

    #[test]
    fn kepler_is_470_and_fermi_is_390() {
        assert_eq!(branch_for("1180"), Some("470"), "GTX 680");
        assert_eq!(branch_for("1080"), Some("390"), "GTX 580");
    }

    #[test]
    fn unsupported_and_unknown_ids_have_no_branch() {
        assert_eq!(branch_for("0000"), None);
        assert_eq!(branch_for("zzzz"), None);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(branch_for("2BB1"), branch_for("2bb1"));
    }
}
