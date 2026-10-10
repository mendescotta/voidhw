//! The only place that changes the system: install packages, write config files, enable services.

use crate::plan::Plan;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Init {
    Runit,
    Dinit,
}

impl Init {
    pub fn name(self) -> &'static str {
        match self {
            Init::Runit => "runit",
            Init::Dinit => "dinit",
        }
    }

    /// Where the service definition lives and where enabling it puts a link, inside the target system.
    fn paths(self, service: &str) -> (String, String) {
        match self {
            Init::Runit => (
                format!("/etc/sv/{service}"),
                format!("/etc/runit/runsvdir/default/{service}"),
            ),
            Init::Dinit => (
                format!("/etc/dinit.d/{service}"),
                format!("/etc/dinit.d/boot.d/{service}"),
            ),
        }
    }
}

/// Which init system the system at `root` runs, from its service directories.
pub fn detect_init(root: &Path) -> Option<Init> {
    if root.join("etc/dinit.d").is_dir() {
        Some(Init::Dinit)
    } else if root.join("etc/runit").is_dir() {
        Some(Init::Runit)
    } else {
        None
    }
}

/// What to change, and where.
pub struct Options {
    pub root: PathBuf,
    /// Print the commands/files instead of running them.
    pub dry_run: bool,
    /// Override init detection.
    pub init: Option<Init>,
}

fn xbps_install(root: &Path, packages: &[&str]) -> Command {
    let mut cmd = Command::new("xbps-install");
    cmd.arg("-Sy");
    if root != Path::new("/") {
        cmd.arg("-r").arg(root);
    }
    cmd.args(packages);
    cmd
}

fn package_installed(root: &Path, package: &str) -> bool {
    Command::new("xbps-query")
        .arg("-r")
        .arg(root)
        .args(["-p", "state", package])
        .output()
        .is_ok_and(|output| output.status.success() && output.stdout == b"installed\n")
}

fn describe(cmd: &Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
    parts.join(" ")
}

fn run(cmd: &mut Command, log: &mut Vec<String>, dry_run: bool) -> Result<(), String> {
    log.push(format!("run: {}", describe(cmd)));
    if dry_run {
        return Ok(());
    }
    let status = cmd
        .status()
        .map_err(|e| format!("{}: {e}", describe(cmd)))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed ({status})", describe(cmd)))
    }
}

/// Rooted path for an absolute path inside the target system.
fn under(root: &Path, absolute: &str) -> PathBuf {
    root.join(absolute.trim_start_matches('/'))
}

/// Apply `plan` to the system at `options.root`. Returns a log of what was (or would be) done.
pub fn apply(plan: &Plan, options: &Options) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    let dry = options.dry_run;
    let root = options.root.as_path();

    if plan.repos.iter().any(|r| r == "nonfree")
        && (dry || !package_installed(root, "void-repo-nonfree"))
    {
        run(
            &mut xbps_install(root, &["void-repo-nonfree"]),
            &mut log,
            dry,
        )?;
    }
    if !plan.packages.is_empty() {
        let packages: Vec<&str> = plan
            .packages
            .iter()
            .map(String::as_str)
            .filter(|package| {
                if !dry && package_installed(root, package) {
                    log.push(format!("skip: {package} is already installed"));
                    false
                } else {
                    true
                }
            })
            .collect();
        if !packages.is_empty() {
            run(&mut xbps_install(root, &packages), &mut log, dry)?;
        }
    }

    for file in &plan.files {
        let path = under(root, &file.path);
        log.push(format!("write: {}", path.display()));
        if !dry {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            fs::write(&path, &file.content).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }

    if !plan.services.is_empty() {
        match options.init.or_else(|| detect_init(root)) {
            None => log.push(format!(
                "skip: cannot tell the init system under {}, not enabling: {}",
                root.display(),
                plan.services.join(" ")
            )),
            Some(init) => {
                for service in &plan.services {
                    let (definition, link_path) = init.paths(service);
                    let link = under(root, &link_path);
                    // A dry run happens before the packages exist, so only a real run can check the definition.
                    if !dry && !under(root, &definition).exists() {
                        log.push(format!(
                            "skip: {service} has no {} service ({definition})",
                            init.name()
                        ));
                        continue;
                    }
                    log.push(format!("enable: {} -> {definition}", link.display()));
                    if dry || link.exists() || link.is_symlink() {
                        continue;
                    }
                    if let Some(parent) = link.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| format!("{}: {e}", parent.display()))?;
                    }
                    symlink(&definition, &link).map_err(|e| format!("{}: {e}", link.display()))?;
                }
            }
        }
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::FileSpec;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("voidhw-apply-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn options(root: &Path, dry_run: bool, init: Option<Init>) -> Options {
        Options {
            root: root.to_path_buf(),
            dry_run,
            init,
        }
    }

    #[test]
    fn configured_guest_packages_are_skipped_but_services_are_enabled() {
        if Command::new("xbps-query")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // XBPS is available on Void, but not every CI host.
        }
        let root = temp_root("installed-guest");
        fs::create_dir_all(root.join("var/db/xbps")).unwrap();
        fs::write(root.join("var/db/xbps/pkgdb-0.38.plist"), r#"<?xml version="1.0"?>
<plist version="1.0"><dict>
<key>virtualbox-ose-guest</key><dict><key>pkgver</key><string>virtualbox-ose-guest-7.2.20_1</string><key>state</key><string>installed</string></dict>
<key>virtualbox-ose-guest-dkms</key><dict><key>pkgver</key><string>virtualbox-ose-guest-dkms-7.2.20_1</string><key>state</key><string>installed</string></dict>
<key>unconfigured</key><dict><key>pkgver</key><string>unconfigured-1_1</string><key>state</key><string>unpacked</string></dict>
</dict></plist>"#).unwrap();
        fs::create_dir_all(root.join("etc/dinit.d")).unwrap();
        fs::write(root.join("etc/dinit.d/vboxservice"), "type = process\n").unwrap();
        assert!(!package_installed(&root, "unconfigured"));
        assert!(!package_installed(&root, "missing"));
        let plan = Plan {
            packages: vec![
                "virtualbox-ose-guest".into(),
                "virtualbox-ose-guest-dkms".into(),
            ],
            services: vec!["vboxservice".into()],
            ..Plan::default()
        };
        let log = apply(&plan, &options(&root, false, None)).unwrap();
        assert_eq!(
            log.iter().filter(|line| line.starts_with("skip:")).count(),
            2
        );
        assert!(!log.iter().any(|line| line.starts_with("run:")));
        assert!(root.join("etc/dinit.d/boot.d/vboxservice").is_symlink());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dry_run_changes_nothing_but_reports_every_step() {
        let root = temp_root("dry");
        let plan = Plan {
            repos: vec!["nonfree".into()],
            packages: vec!["nvidia".into()],
            files: vec![FileSpec {
                path: "/etc/modprobe.d/x.conf".into(),
                content: "a\n".into(),
            }],
            services: vec!["switcheroo-control".into()],
            ..Plan::default()
        };
        let log = apply(&plan, &options(&root, true, Some(Init::Runit))).unwrap();
        assert!(log.iter().any(|l| l.contains("void-repo-nonfree")));
        assert!(log
            .iter()
            .any(|l| l.contains("xbps-install -Sy") && l.contains("nvidia")));
        assert!(log
            .iter()
            .any(|l| l.starts_with("write:") && l.contains("x.conf")));
        assert!(log
            .iter()
            .any(|l| l.starts_with("enable:") && l.contains("switcheroo-control")));
        assert!(
            !root.join("etc").exists(),
            "a dry run must not touch the target"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn files_and_runit_links_are_written_under_the_root() {
        let root = temp_root("write");
        fs::create_dir_all(root.join("etc/sv/switcheroo-control")).unwrap();
        let plan = Plan {
            files: vec![FileSpec {
                path: "/etc/modprobe.d/x.conf".into(),
                content: "options a=1\n".into(),
            }],
            services: vec!["switcheroo-control".into()],
            ..Plan::default()
        };
        let opts = options(&root, false, Some(Init::Runit));
        apply(&plan, &opts).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("etc/modprobe.d/x.conf")).unwrap(),
            "options a=1\n"
        );
        let link = root.join("etc/runit/runsvdir/default/switcheroo-control");
        assert_eq!(
            fs::read_link(&link).unwrap(),
            PathBuf::from("/etc/sv/switcheroo-control")
        );
        apply(&plan, &opts).unwrap(); // applying twice is fine
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn dinit_services_are_linked_into_boot_d() {
        let root = temp_root("dinit");
        fs::create_dir_all(root.join("etc/dinit.d/boot.d")).unwrap();
        fs::write(root.join("etc/dinit.d/vmtoolsd"), "type = process\n").unwrap();
        let plan = Plan {
            services: vec!["vmtoolsd".into()],
            ..Plan::default()
        };
        apply(&plan, &options(&root, false, None)).unwrap(); // init detected from /etc/dinit.d
        assert_eq!(
            fs::read_link(root.join("etc/dinit.d/boot.d/vmtoolsd")).unwrap(),
            PathBuf::from("/etc/dinit.d/vmtoolsd")
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn init_is_detected_from_the_service_directories() {
        let root = temp_root("detect");
        assert_eq!(detect_init(&root), None);
        fs::create_dir_all(root.join("etc/runit")).unwrap();
        assert_eq!(detect_init(&root), Some(Init::Runit));
        fs::create_dir_all(root.join("etc/dinit.d")).unwrap();
        assert_eq!(detect_init(&root), Some(Init::Dinit));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_service_the_init_has_no_definition_for_is_skipped_not_fatal() {
        let root = temp_root("missing");
        fs::create_dir_all(root.join("etc/dinit.d/boot.d")).unwrap();
        let plan = Plan {
            services: vec!["nope".into()],
            ..Plan::default()
        };
        let log = apply(&plan, &options(&root, false, None)).unwrap();
        assert!(
            log.iter()
                .any(|l| l.starts_with("skip:") && l.contains("nope")),
            "{log:?}"
        );
        assert!(!root.join("etc/dinit.d/boot.d/nope").exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unknown_init_system_skips_services_with_a_message() {
        let root = temp_root("noinit");
        let plan = Plan {
            services: vec!["x".into()],
            ..Plan::default()
        };
        let log = apply(&plan, &options(&root, false, None)).unwrap();
        assert!(
            log.iter()
                .any(|l| l.contains("cannot tell the init system")),
            "{log:?}"
        );
        fs::remove_dir_all(&root).ok();
    }
}
