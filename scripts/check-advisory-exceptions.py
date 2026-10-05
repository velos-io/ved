#!/usr/bin/env python3
"""Fail closed when the reviewed maintenance-advisory exceptions expire or drift."""
import argparse
from collections import Counter
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys
import tomllib

EXPIRES = datetime(2027, 1, 9, tzinfo=timezone.utc)
# ID -> (affected package, reviewed version, exact direct reverse dependencies).
EXCEPTIONS = {
    "RUSTSEC-2023-0089": ("atomic-polyfill", "1.0.3", {("heapless", "0.7.17")}),
    "RUSTSEC-2024-0388": ("derivative", "2.2.0", {("starlark", "0.14.2"), ("starlark_syntax", "0.14.2")}),
    "RUSTSEC-2025-0057": ("fxhash", "0.2.1", {("starlark_map", "0.14.2")}),
    "RUSTSEC-2024-0436": ("paste", "1.0.15", {("starlark", "0.14.2")}),
    "RUSTSEC-2024-0449": ("unsafe-libyaml", "0.2.11", {("serde_yaml", "0.9.34+deprecated")}),
}
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def check_policy(policy, now):
    if now >= EXPIRES:
        raise ValueError(f"advisory exceptions expired at {EXPIRES.isoformat()}; review and remove or renew explicitly")
    graph = policy["graph"]
    if graph.get("all-features") is not True or graph.get("exclude") or graph.get("targets"):
        raise ValueError("deny graph must include all features and targets without package exclusions")
    advisories = policy["advisories"]
    for category in ("unmaintained", "unsound"):
        if advisories.get(category) != "all":
            raise ValueError(f"advisories.{category} must remain 'all'")
    for key in ("yanked", "unused-ignored-advisory"):
        if advisories.get(key) != "deny":
            raise ValueError(f"advisories.{key} must remain 'deny'")
    ignores = advisories.get("ignore", [])
    ids = [entry["id"] for entry in ignores]
    if Counter(ids) != Counter(EXCEPTIONS.keys()):
        raise ValueError(f"ignored advisory IDs differ from reviewed policy: {ids}")
    if any(not entry.get("reason", "").strip() for entry in ignores):
        raise ValueError("every exception requires a nonempty reason")


def check_graph(metadata, label):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = metadata["resolve"]["nodes"]
    for name, version, expected_parents in EXCEPTIONS.values():
        matches = [package for package in packages.values() if package["name"] == name]
        if len(matches) != 1 or matches[0]["version"] != version or matches[0]["source"] != REGISTRY:
            actual = [(package["version"], package["source"]) for package in matches]
            raise ValueError(f"{label}: {name} must remain exactly registry version {version}; found {actual}")
        target = matches[0]["id"]
        parents = [packages[node["id"]] for node in nodes if any(dep["pkg"] == target for dep in node["deps"])]
        actual_parents = {(parent["name"], parent["version"]) for parent in parents}
        if actual_parents != expected_parents or len(parents) != len(expected_parents) or any(parent["source"] != REGISTRY for parent in parents):
            raise ValueError(f"{label}: {name} reverse parents changed: expected {sorted(expected_parents)}, found {sorted(actual_parents)}")


def load_graph(manifest):
    # No --filter-platform: retain target-specific edges (including atomic-polyfill).
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--all-features", "--format-version", "1", "--manifest-path", str(manifest)],
        check=True, capture_output=True, text=True, timeout=120,
    )
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    try:
        with (args.root / "deny.toml").open("rb") as source:
            check_policy(tomllib.load(source), datetime.now(timezone.utc))
        for manifest in (args.root / "Cargo.toml", args.root / "fuzz/Cargo.toml"):
            check_graph(load_graph(manifest), str(manifest))
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as error:
        print(f"advisory exception guard failed: {error}", file=sys.stderr)
        return 1
    print(f"advisory exceptions match both locked graphs; expire {EXPIRES.date()} UTC")
    return 0


if __name__ == "__main__":
    sys.exit(main())
