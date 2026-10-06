# voidhw

Hardware-aware driver and firmware selection for Void Linux.

`voidhw` reads the machine's PCI and USB devices, CPU vendor and chassis type from sysfs, and works
out which packages, config files and runit services it needs: Mesa for Intel and AMD GPUs, the
right NVIDIA driver branch for each NVIDIA GPU, Wi-Fi firmware and DKMS drivers, CPU microcode,
laptop power management, hybrid-graphics switching.

By default it only reports. Nothing changes unless you pass `--apply`.

```
$ voidhw
Machine: notebook (CPU GenuineIntel)

Devices:
  0000:00:14.3 8086:a370 -> wifi-intel-firmware
  0000:01:00.0 GPU 10de:2191 -> nvidia-current

Repositories: nonfree
Packages:     linux-firmware-intel nvidia intel-ucode power-profiles-daemon
File:         /etc/modprobe.d/voidhw-nvidia-drm.conf
Services:     power-profiles-daemon
...
```

## Usage

```
voidhw                       report what this machine needs
voidhw --json                the same plan as JSON (for installers)
voidhw --apply --dry-run     preview the install steps
sudo voidhw --apply          install packages, write files, enable services
voidhw --root /mnt/target    inspect or (with --apply) configure a system mounted elsewhere
voidhw --profiles DIR        add or override profiles from *.toml files
voidhw --snapshot DIR        save the hardware description (no serial numbers) for a bug report
```

## NVIDIA branches

Void ships four NVIDIA driver branches (`nvidia`, `nvidia580`, `nvidia470`, `nvidia390`). The branch
for each GPU comes from NVIDIA's own supported-chips lists, compiled into
`data/nvidia-branches.toml` by `tools/gen-nvidia-table.py`: the newest branch that lists the PCI
device id wins. GPUs no Void driver supports stay on nouveau. Regenerate the table when Void moves
to a new branch.

## Profiles

Profiles are TOML in `data/profiles/` (compiled in) or a directory given with `--profiles`. See
`src/profile.rs` for the schema. `tools/check-packages.py` verifies that every package exists in
the Void repositories and that every service is shipped by one of the profile's packages.

## Testing

`cargo test` runs the unit tests and integration tests against sysfs trees: real snapshots in
`tests/fixtures/` and synthetic machines built in the tests. To add your machine:
`voidhw --snapshot tests/fixtures/<name>` and write a test for the expected plan.

## Lineage

The profile idea comes from CachyOS's `chwd`, via the `chwd_port` prototype in Aetheris OS (GPL-3.0).
This is a rewrite for Void: the PCI class parsing is fixed, package names are Void's, and changes are
opt-in.

Licensed under GPL-3.0-or-later.
