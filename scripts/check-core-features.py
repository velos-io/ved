#!/usr/bin/env python3
"""Check the independent rendering consumer's dependency boundary and byte parity."""
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "tests/fixtures/rendering-consumer/Cargo.toml"
ENV = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "target/rendering-consumer"))


def cargo(*args):
    command = ["cargo", *args, "--manifest-path", str(MANIFEST), "--locked"]
    try:
        result = subprocess.run(command, cwd=ROOT, env=ENV, capture_output=True,
                                text=True, timeout=600, check=False)
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(f"Command exceeded 600 seconds: {' '.join(command)}") from error
    if result.returncode:
        raise RuntimeError(f"Command failed: {' '.join(command)}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def main():
    tree = cargo("tree", "--edges", "normal", "--prefix", "none", "--no-default-features")
    packages = {line.split()[0] for line in tree.splitlines() if line.strip()}
    forbidden = packages & {"starlark", "ved-process", "rustix", "signal-hook"}
    if forbidden:
        raise RuntimeError(f"Host dependencies in rendering consumer: {sorted(forbidden)}")
    pure = cargo("run", "--quiet", "--no-default-features")
    host = cargo("run", "--quiet", "--no-default-features", "--features", "host")
    if not pure or pure != host:
        raise RuntimeError("Pure and host-enabled consumer artifact paths/bytes differ")
    paths = []
    for line in pure.splitlines():
        path, separator, encoded = line.partition("\t")
        if not separator or not re.fullmatch(r"(?:[0-9a-f]{2})*", encoded):
            raise RuntimeError("Invalid artifact record from consumer")
        if Path(path).is_absolute() or ".." in Path(path).parts:
            raise RuntimeError(f"Non-relative artifact path: {path}")
        paths.append(path)
    if not {"docker-compose.yml", "helm/values.yaml"}.issubset(paths) or not any(p.startswith("k8s/") for p in paths):
        raise RuntimeError("Consumer did not exercise all rendering targets")
    print(f"Rendering boundary passed: no host dependencies; {len(paths)} artifact paths and bytes identical")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as error:
        print(error, file=sys.stderr)
        sys.exit(1)
