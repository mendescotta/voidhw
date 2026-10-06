use std::path::PathBuf;
use std::process::ExitCode;

use voidhw::apply::{self, Init, Options};
use voidhw::plan::{self, Plan};
use voidhw::{profile, snapshot, sysfs};

const USAGE: &str = "\
voidhw - hardware-aware driver selection for Void Linux

Usage: voidhw [options]

Without options it only reports what this machine needs; nothing is changed.

  --apply            install the packages, write the config files and enable the services
  --dry-run          with --apply: print the steps instead of running them
  --json             print the plan as JSON
  --init dinit|runit force the init system for enabling services (default: detected under --root)
  --root DIR         the system to inspect and, with --apply, change (default /)
  --hardware-from DIR  read the hardware from DIR instead of --root; use it to configure an
                     install target (--root /mnt) for the machine that is running (--hardware-from /)
  --profiles DIR     extra profile files (*.toml); a profile with the same id replaces the built-in one
  --snapshot DIR     save the hardware description to DIR (for bug reports and tests) and exit
  -h, --help         show this help
  -V, --version      show the version
";

struct Args {
    apply: bool,
    dry_run: bool,
    json: bool,
    root: PathBuf,
    hardware_from: Option<PathBuf>,
    init: Option<Init>,
    profiles: Option<PathBuf>,
    snapshot: Option<PathBuf>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        apply: false,
        dry_run: false,
        json: false,
        root: PathBuf::from("/"),
        hardware_from: None,
        init: None,
        profiles: None,
        snapshot: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--apply" => args.apply = true,
            "--dry-run" => args.dry_run = true,
            "--json" => args.json = true,
            "--root" => args.root = value("--root")?,
            "--hardware-from" => args.hardware_from = Some(value("--hardware-from")?),
            "--init" => {
                args.init = Some(match value("--init")?.to_string_lossy().as_ref() {
                    "dinit" => Init::Dinit,
                    "runit" => Init::Runit,
                    other => return Err(format!("--init must be dinit or runit, not {other}")),
                })
            }
            "--profiles" => args.profiles = Some(value("--profiles")?),
            "--snapshot" => args.snapshot = Some(value("--snapshot")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("voidhw {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    if args.dry_run && !args.apply {
        return Err("--dry-run only makes sense with --apply".to_string());
    }
    Ok(Some(args))
}

fn print_report(hw: &sysfs::Hardware, plan: &Plan) {
    println!(
        "Machine: {} (CPU {})",
        plan.chassis,
        if plan.cpu_vendor.is_empty() {
            "unknown"
        } else {
            &plan.cpu_vendor
        }
    );
    if let Some(hypervisor) = &plan.hypervisor {
        println!("Virtual machine: {hypervisor}");
    }
    if plan.hybrid_graphics {
        println!("Graphics: hybrid (integrated + NVIDIA)");
    }
    println!("\nDevices:");
    if plan.devices.is_empty() {
        println!("  (none needing a profile)");
    }
    for dev in &plan.devices {
        let what = hw
            .pci
            .iter()
            .find(|d| d.address == dev.address)
            .map_or("", |d| if d.is_gpu() { "GPU " } else { "" });
        match &dev.profile {
            Some(profile) => println!("  {} {}{} -> {profile}", dev.address, what, dev.id),
            None => println!("  {} {}{} -> no profile", dev.address, what, dev.id),
        }
    }
    if plan.packages.is_empty() && plan.files.is_empty() && plan.services.is_empty() {
        println!("\nNothing to install.");
        return;
    }
    if !plan.repos.is_empty() {
        println!("\nRepositories: {}", plan.repos.join(", "));
    }
    if !plan.packages.is_empty() {
        println!("Packages:     {}", plan.packages.join(" "));
    }
    for file in &plan.files {
        println!("File:         {}", file.path);
    }
    if !plan.services.is_empty() {
        println!("Services:     {}", plan.services.join(" "));
    }
    for note in &plan.notes {
        println!("Note:         {note}");
    }
    println!("\nRun with --apply to make these changes (add --dry-run to preview).");
}

fn run() -> Result<(), String> {
    let Some(args) = parse_args()? else {
        return Ok(());
    };

    if let Some(out) = &args.snapshot {
        snapshot::write(&args.root, out).map_err(|e| format!("snapshot failed: {e}"))?;
        println!("hardware description saved to {}", out.display());
        return Ok(());
    }

    let hw = sysfs::scan(args.hardware_from.as_ref().unwrap_or(&args.root)).map_err(|e| format!("cannot read hardware: {e}"))?;
    let mut profiles = profile::embedded();
    if let Some(dir) = &args.profiles {
        profiles = profile::with_overrides(profiles, dir)?;
    }
    let plan = plan::build(&hw, &profiles);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())?
        );
    } else {
        print_report(&hw, &plan);
    }

    if args.apply {
        if !args.dry_run && not_root() {
            return Err("--apply needs root (use --dry-run to preview)".to_string());
        }
        let log = apply::apply(
            &plan,
            &Options {
                root: args.root.clone(),
                dry_run: args.dry_run,
                init: args.init,
            },
        )?;
        if !args.json {
            println!("\n{}:", if args.dry_run { "Would do" } else { "Done" });
            for line in log {
                println!("  {line}");
            }
        }
    }
    Ok(())
}

/// True when the effective uid is not 0 (reads /proc/self/status; no libc dependency).
fn not_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .map(|l| l.split_whitespace().nth(2).unwrap_or("1") != "0")
        })
        .unwrap_or(true)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("voidhw: {message}");
            ExitCode::FAILURE
        }
    }
}
