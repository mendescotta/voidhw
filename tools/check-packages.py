#!/usr/bin/env python3
"""Check that every package named in data/profiles/*.toml exists in the Void repositories
and that every service a profile enables is shipped (as /etc/sv/<name>) by one of its packages.

Needs xbps-query with repository data (run `xbps-install -S` once). Exit status 1 on any problem.
"""
import glob
import subprocess
import sys
import tomllib


def query(*args):
    return subprocess.run(["xbps-query", "-R", *args], capture_output=True, text=True)


def main():
    problems = []
    checked = set()
    for path in sorted(glob.glob("data/profiles/*.toml")):
        data = tomllib.load(open(path, "rb"))
        for profile in data.get("profile", []):
            pid = profile["id"]
            for pkg in profile.get("packages", []):
                if pkg in checked:
                    continue
                checked.add(pkg)
                if not query("-p", "pkgver", pkg).stdout.strip():
                    problems.append(f"{pid}: package '{pkg}' is not in the Void repositories")
            for svc in profile.get("services", []):
                shipped = any(f"/etc/sv/{svc}/run" in query("-f", pkg).stdout for pkg in profile.get("packages", []))
                if not shipped:
                    problems.append(f"{pid}: no package of this profile ships /etc/sv/{svc}/run")
    for line in problems:
        print("FAIL", line)
    print(f"{len(checked)} packages checked, {len(problems)} problem(s)")
    sys.exit(1 if problems else 0)


main()
