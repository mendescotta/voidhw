@AGENTS.md

# voidhw

Hardware-aware driver, firmware, configuration and service planner for Void.
By default it reports only; mutation requires `--apply`.

- Keep hardware discovery separate from the package and service plan.
- Preserve `--json` as the machine-readable interface used by installers.
- Test with synthetic sysfs trees or `tests/fixtures/`, not the host's devices.
- `--root` directs target writes; check every path stays under that root.
- Verify with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test`. Run `tools/check-packages.py` when profiles change.
