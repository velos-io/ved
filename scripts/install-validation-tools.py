#!/usr/bin/env python3
"""Install pinned Compose and kubectl after upstream SHA256 verification."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import tempfile
import time
import urllib.request

MAX_BINARY_BYTES = 150 * 1024 * 1024
TIMEOUT_SECONDS = 30
TOTAL_DOWNLOAD_SECONDS = 180


def download(url, limit=MAX_BINARY_BYTES):
    deadline = time.monotonic() + TOTAL_DOWNLOAD_SECONDS
    request = urllib.request.Request(url, headers={"User-Agent": "vt-deployer-validation"})
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
        if int(response.headers.get("Content-Length", "0")) > limit:
            raise ValueError("download exceeds size limit")
        chunks, size = [], 0
        while True:
            if time.monotonic() > deadline:
                raise TimeoutError("download deadline exceeded")
            chunk = response.read(min(1024 * 1024, limit + 1 - size))
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
            size += len(chunk)
            if size > limit:
                raise ValueError("download exceeds size limit")


def urls(tool, version, system, machine):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"invalid {tool} version: {version!r}")
    if system not in ("linux", "darwin") or machine not in ("x86_64", "arm64"):
        raise ValueError(f"unsupported platform: {system}/{machine}")
    if tool == "compose":
        arch = "aarch64" if machine == "arm64" else machine
        return f"https://github.com/docker/compose/releases/download/v{version}/docker-compose-{system}-{arch}"
    if tool == "kubectl":
        arch = "amd64" if machine == "x86_64" else machine
        return f"https://dl.k8s.io/release/v{version}/bin/{system}/{arch}/kubectl"
    raise ValueError(f"unsupported tool: {tool}")


def install(destination, report_dir, system=None, machine=None):
    report_dir.mkdir(parents=True, exist_ok=True)
    destination.mkdir(parents=True, exist_ok=True)
    system = (system or platform.system()).lower()
    machine = machine or platform.machine()
    machine = {"AMD64": "x86_64", "aarch64": "arm64"}.get(machine, machine)
    report = {"platform": f"{system}/{machine}", "status": "failed", "tools": []}
    try:
        pins = json.loads(Path(__file__).with_name("validation-tools.json").read_text())
        for tool in ("compose", "kubectl"):
            version = pins[tool]
            url = urls(tool, version, system, machine)
            entry = {"tool": tool, "version": version, "url": url,
                     "checksum_url": url + ".sha256", "status": "failed"}
            report["tools"].append(entry)
            checksum = download(entry["checksum_url"], 4096).decode("ascii").strip().split()[0]
            if not re.fullmatch(r"[a-fA-F0-9]{64}", checksum):
                raise ValueError(f"invalid upstream SHA256 for {tool}")
            payload = download(url)
            actual = hashlib.sha256(payload).hexdigest()
            entry.update(expected_sha256=checksum.lower(), actual_sha256=actual, bytes=len(payload))
            if actual != checksum.lower():
                raise ValueError(f"SHA256 mismatch for {tool}")
            name = "docker-compose" if tool == "compose" else tool
            with tempfile.NamedTemporaryFile(dir=destination, delete=False) as output:
                temporary = Path(output.name)
                output.write(payload)
            try:
                temporary.chmod(0o755)
                os.replace(temporary, destination / name)
            finally:
                temporary.unlink(missing_ok=True)
            entry.update(status="verified", path=str(destination / name))
        report["status"] = "verified"
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        (report_dir / "tool-downloads.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--report-dir", required=True, type=Path)
    parser.add_argument("--system", choices=("linux", "darwin"), help="override platform for download verification")
    parser.add_argument("--machine", choices=("x86_64", "arm64"), help="override architecture for download verification")
    args = parser.parse_args()
    try:
        report = install(args.destination, args.report_dir, args.system, args.machine)
    except Exception as error:
        parser.exit(1, f"validation tool installation failed: {error}\n")
    print(f"Verified {len(report['tools'])} tools for {report['platform']}")


if __name__ == "__main__":
    main()
