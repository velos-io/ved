#!/usr/bin/env python3
"""Canonical Runtime Diff Verification Tool.

Normalizes Compose manifests using the platform's native resolver
(`docker compose config --format json --no-env-resolution`) and verifies
semantic parity across services, healthchecks, networks, and environment variables.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any, Dict, List, Optional, Tuple


def find_docker() -> Optional[str]:
    """Locate the docker binary across common installation paths."""
    candidates = [
        shutil.which("docker"),
        "/usr/local/bin/docker",
        "/opt/homebrew/bin/docker",
        "/usr/bin/docker",
    ]
    for path in candidates:
        if path and os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    return None


def resolve_canonical_compose(file_path: Path, docker_bin: str) -> Dict[str, Any]:
    """Run docker compose config and return normalized JSON output."""
    if not file_path.exists():
        raise FileNotFoundError(f"Compose file not found: {file_path}")

    cmd = [
        docker_bin,
        "compose",
        "-f",
        str(file_path.resolve()),
        "config",
        "--format",
        "json",
        "--no-env-resolution",
    ]

    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        check=False,
    )

    if result.returncode != 0:
        raise RuntimeError(
            f"Failed to resolve canonical config for {file_path}:\n{result.stderr.strip()}"
        )

    return json.loads(result.stdout)


def _normalize_values(obj: Any) -> Any:
    """Normalize macOS /private symlink paths and recursive structures."""
    if isinstance(obj, str):
        if obj.startswith("/private/"):
            return obj[len("/private"):]
        return obj
    elif isinstance(obj, dict):
        return {k: _normalize_values(v) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [_normalize_values(item) for item in obj]
    return obj


def sanitize_canonical_tree(tree: Dict[str, Any], ignore_keys: List[str]) -> Dict[str, Any]:
    """Sanitize directory-dependent fields like project name and absolute build contexts."""
    sanitized = json.loads(json.dumps(tree))
    sanitized = _normalize_values(sanitized)

    # Project names depend on folder location, not service semantics
    sanitized.pop("name", None)

    for key in ignore_keys:
        sanitized.pop(key, None)

    services = sanitized.get("services", {})
    if isinstance(services, dict):
        for svc in services.values():
            if isinstance(svc, dict) and "build" in svc:
                build = svc["build"]
                if isinstance(build, dict) and "context" in build:
                    # Strip absolute context directory to allow comparing across temp dirs
                    build["context"] = Path(build["context"]).name

    return sanitized


def diff_canonical_trees(
    source: Any,
    compiled: Any,
    path: str = "",
) -> List[str]:
    """Recursively compare two JSON trees and return all semantic deviations."""
    differences: List[str] = []

    if type(source) != type(compiled):
        differences.append(
            f"TYPE MISMATCH at '{path}': expected {type(source).__name__}, got {type(compiled).__name__}"
        )
        return differences

    if isinstance(source, dict):
        source_keys = set(source.keys())
        compiled_keys = set(compiled.keys())

        for key in sorted(source_keys - compiled_keys):
            subpath = f"{path}.{key}" if path else key
            differences.append(f"MISSING FIELD at '{subpath}': expected {json.dumps(source[key])}")

        for key in sorted(compiled_keys - source_keys):
            subpath = f"{path}.{key}" if path else key
            differences.append(f"UNEXPECTED FIELD at '{subpath}': found {json.dumps(compiled[key])}")

        for key in sorted(source_keys & compiled_keys):
            subpath = f"{path}.{key}" if path else key
            differences.extend(diff_canonical_trees(source[key], compiled[key], subpath))

    elif isinstance(source, list):
        if len(source) != len(compiled):
            differences.append(
                f"ARRAY LENGTH MISMATCH at '{path}': expected {len(source)} items, got {len(compiled)}"
            )
        else:
            for idx, (s_item, c_item) in enumerate(zip(source, compiled)):
                subpath = f"{path}[{idx}]"
                differences.extend(diff_canonical_trees(s_item, c_item, subpath))

    else:
        if source != compiled:
            differences.append(
                f"VALUE MISMATCH at '{path}': expected {json.dumps(source)}, got {json.dumps(compiled)}"
            )

    return differences


def main() -> int:
    parser = argparse.ArgumentParser(description="Canonical Runtime Diff Verification")
    parser.add_argument("--source", "-s", required=True, type=Path, help="Path to source Compose manifest")
    parser.add_argument("--compiled", "-c", required=True, type=Path, help="Path to compiled Compose manifest")
    parser.add_argument("--ignore", "-i", action="append", default=[], help="Keys to ignore from root")
    parser.add_argument("--verbose", "-v", action="store_true", help="Print debug information")

    args = parser.parse_args()

    docker_bin = find_docker()
    if not docker_bin:
        sys.stderr.write("ERROR: Docker binary not found in PATH or standard directories.\n")
        return 2

    try:
        source_raw = resolve_canonical_compose(args.source, docker_bin)
        compiled_raw = resolve_canonical_compose(args.compiled, docker_bin)
    except Exception as err:
        sys.stderr.write(f"ERROR: {err}\n")
        return 2

    source_tree = sanitize_canonical_tree(source_raw, args.ignore)
    compiled_tree = sanitize_canonical_tree(compiled_raw, args.ignore)

    differences = diff_canonical_trees(source_tree, compiled_tree)

    if not differences:
        print(f"PASS: Canonical equivalence verified between {args.source} and {args.compiled}.")
        return 0

    print(f"FAIL: Semantic drift detected ({len(differences)} differences):")
    for diff in differences:
        print(f"  - {diff}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
