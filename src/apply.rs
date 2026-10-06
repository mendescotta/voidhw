//! The only place that changes the system: install packages, write config files, enable services.

use crate::plan::Plan;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Print the commands/files instead of running them.
pub struct Options {
    pub root: PathBuf,
    pub dry_run: bool,
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

    if plan.repos.iter().any(|r| r == "nonfree") {
        run(
            &mut xbps_install(root, &["void-repo-nonfree"]),
            &mut log,
            dry,
        )?;
    }
    if !plan.packages.is_empty() {
        let packages: Vec<&str> = plan.packages.iter().map(String::as_str).collect();
        run(&mut xbps_install(root, &packages), &mut log, dry)?;
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

    for service in &plan.services {
        let link = under(root, &format!("/etc/runit/runsvdir/default/{service}"));
        let target = format!("/etc/sv/{service}");
        log.push(format!("enable: {} -> {target}", link.display()));
        if dry {
            continue;
        }
        if !under(root, &target).exists() {
            return Err(format!(
                "service {service} not found at {target}; is its package installed?"
            ));
        }
        if link.exists() || link.is_symlink() {
            continue;
        }
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        symlink(&target, &link).map_err(|e| format!("{}: {e}", link.display()))?;
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::FileSpec;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("voidhw-apply-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
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
        let log = apply(
            &plan,
            &Options {
                root: root.clone(),
                dry_run: true,
            },
        )
        .unwrap();
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
    fn files_and_service_links_are_written_under_the_root() {
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
        apply(
            &plan,
            &Options {
                root: root.clone(),
                dry_run: false,
            },
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(root.join("etc/modprobe.d/x.conf")).unwrap(),
            "options a=1\n"
        );
        let link = root.join("etc/runit/runsvdir/default/switcheroo-control");
        assert_eq!(
            fs::read_link(&link).unwrap(),
            PathBuf::from("/etc/sv/switcheroo-control")
        );
        // Applying twice is fine.
        apply(
            &plan,
            &Options {
                root: root.clone(),
                dry_run: false,
            },
        )
        .unwrap();
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn enabling_a_missing_service_is_an_error() {
        let root = temp_root("missing");
        let plan = Plan {
            services: vec!["nope".into()],
            ..Plan::default()
        };
        let err = apply(
            &plan,
            &Options {
                root: root.clone(),
                dry_run: false,
            },
        )
        .unwrap_err();
        assert!(err.contains("nope"));
        fs::remove_dir_all(&root).ok();
    }
}
