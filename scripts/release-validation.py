#!/usr/bin/env python3
"""Bounded offline parser and extracted release executable evidence (no deployment)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


class ValidationError(Exception):
    pass


class Harness:
    def __init__(self, report_dir, timeout=60, output_limit=4 * 1024 * 1024):
        self.report_dir = Path(report_dir).resolve()
        self.report_dir.mkdir(parents=True, exist_ok=True)
        self.timeout, self.output_limit = timeout, output_limit
        self.report = {"evidence": "offline parsing/rendering; no schema, admission or runtime claim", "commands": [], "versions": {}, "status": "failed"}

    def run(self, argv, cwd):
        record = {"argv": [str(x) for x in argv]}
        self.report["commands"].append(record)
        prefix = self.report_dir / f"command-{len(self.report['commands']):02}"
        record.update(stdout=str(prefix) + '.stdout', stderr=str(prefix) + '.stderr')
        env = {"PATH": "/usr/bin:/bin", "HOME": str(cwd), "TMPDIR": str(cwd), "KUBECONFIG": str(Path(cwd) / "absent-kubeconfig"), "DOCKER_CONFIG": str(Path(cwd) / "docker-config"), "LANG": "C", "LC_ALL": "C"}
        buffers = {"stdout": bytearray(), "stderr": bytearray()}
        process = None
        problem = None
        try:
            process = subprocess.Popen(record['argv'], cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            with selectors.DefaultSelector() as selector:
                for name in buffers:
                    stream = getattr(process, name)
                    os.set_blocking(stream.fileno(), False)
                    selector.register(stream, selectors.EVENT_READ, name)
                deadline = time.monotonic() + self.timeout
                while selector.get_map():
                    if time.monotonic() >= deadline:
                        raise ValidationError(f"command timed out after {self.timeout}s")
                    for key, _ in selector.select(min(.1, max(0, deadline - time.monotonic()))):
                        data = os.read(key.fileobj.fileno(), 65536)
                        if not data:
                            selector.unregister(key.fileobj)
                            continue
                        room = self.output_limit - sum(map(len, buffers.values()))
                        buffers[key.data].extend(data[:max(0, room)])
                        if len(data) > room:
                            raise ValidationError(f"command exceeded output limit {self.output_limit} bytes")
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ValidationError(f"command timed out after {self.timeout}s")
                record['returncode'] = process.wait(timeout=remaining)
                if record['returncode']:
                    raise ValidationError(f"command failed with exit {record['returncode']}: {record['argv']}")
        except (OSError, subprocess.TimeoutExpired, ValidationError) as exc:
            problem = str(exc)
            record['error'] = problem
        finally:
            if process:
                # Also kill descendants which outlive a successful parent.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                for name in buffers:
                    getattr(process, name).close()
            for name, data in buffers.items():
                Path(record[name]).write_bytes(data)
        if problem:
            raise ValidationError(problem)
        return bytes(buffers['stdout']).decode('utf-8', errors='replace') + bytes(buffers['stderr']).decode('utf-8', errors='replace')

    def finish(self, error=None):
        if error:
            self.report['error'] = str(error)
        else:
            self.report['status'] = 'passed'
        (self.report_dir / 'report.json').write_text(json.dumps(self.report, indent=2) + '\n')
        (self.report_dir / 'report.txt').write_text(json.dumps(self.report, indent=2) + '\n')


def executable(path):
    value = Path(path).expanduser().resolve()
    if not value.is_file() or not os.access(value, os.X_OK):
        raise ValidationError(f"missing or non-executable tool: {value}")
    return value


def compile_fixture(harness, ved, workspace):
    config = workspace / 'config'
    config.mkdir()
    fixture = workspace / 'ved.yaml'
    shutil.copyfile(ROOT / 'tests/fixtures/release-validation/ved.yaml', fixture)
    output = workspace / 'dist'
    harness.run([ved, '--dir', workspace, '--config-dir', config, '--work-dir', workspace / '.ved', '--output-dir', output, 'compile', '--file', fixture, '--target', 'all'], workspace)
    for required in ['docker-compose.yml', 'values.yaml', 'k8s/base/kustomization.yaml']:
        if not (output / required).is_file():
            raise ValidationError(f"compile missing artifact: {required}")
    harness.report['generated_artifacts'] = [
        {'path': str(path.relative_to(output)), 'size': path.stat().st_size, 'sha256': file_sha256(path)}
        for path in sorted(output.rglob('*')) if path.is_file()
    ]
    return output


def file_sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def archive_binary(archive, workspace, report=None):
    with tarfile.open(archive, 'r:gz') as handle:
        members = handle.getmembers()
        if len(members) != 1 or members[0].name != 'ved' or not members[0].isfile():
            raise ValidationError('release archive must contain exactly one regular file named ved')
        member = members[0]
        if not member.mode & 0o111:
            raise ValidationError('release archive ved must be executable')
        if member.size > 256 * 1024 * 1024:
            raise ValidationError('release archive executable exceeds 256 MiB limit')
        binary = workspace / 'ved'
        source = handle.extractfile(member)
        if source is None:
            raise ValidationError('release archive has no executable payload')
        with source, binary.open('wb') as destination:
            shutil.copyfileobj(source, destination)
        binary.chmod(0o755)
        if report is not None:
            report['archive_inspection'] = {
                'member': member.name,
                'size': member.size,
                'mode': oct(member.mode),
                'archive_sha256': file_sha256(archive),
                'extracted_binary_sha256': file_sha256(binary),
            }
        return binary


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ved')
    parser.add_argument('--compose')
    parser.add_argument('--kubectl')
    parser.add_argument('--archive')
    parser.add_argument('--smoke-only', action='store_true')
    parser.add_argument('--content-only', action='store_true')
    parser.add_argument('--report-dir', required=True)
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--output-limit', type=int, default=4 * 1024 * 1024)
    args = parser.parse_args(argv)
    harness = Harness(args.report_dir, args.timeout, args.output_limit)
    try:
        if args.timeout <= 0 or args.output_limit <= 0:
            raise ValidationError('timeout and output-limit must be positive')
        with tempfile.TemporaryDirectory(prefix='ved-release-validation-') as temporary:
            workspace = Path(temporary)
            if args.archive:
                ved = archive_binary(Path(args.archive).resolve(), workspace, harness.report)
                harness.report['archive'] = str(Path(args.archive).resolve())
                harness.report['archive_evidence'] = 'content only' if args.content_only else 'extracted executable version/help/compile'
                if args.content_only:
                    harness.finish()
                    return 0
            else:
                if args.content_only or not args.ved or (not args.smoke_only and not all([args.compose, args.kubectl])):
                    raise ValidationError('offline validation requires --ved, --compose and --kubectl; --content-only requires --archive')
                ved = executable(args.ved)
            version = harness.run([ved, '--version'], workspace)
            if not re.search(r'\bved\s+\d+\.\d+\.\d+', version):
                raise ValidationError('unexpected ved version output')
            harness.report['versions']['ved'] = version.strip()
            harness.run([ved, '--help'], workspace)
            output = compile_fixture(harness, ved, workspace)
            if not args.archive and not args.smoke_only:
                pins = json.loads((ROOT / 'scripts/validation-tools.json').read_text())
                compose, kubectl = executable(args.compose), executable(args.kubectl)
                version = harness.run([compose, 'version', '--short'], workspace).strip()
                harness.report['versions']['compose'] = version
                if version.removeprefix('v') != pins['compose'].removeprefix('v'):
                    raise ValidationError(f"Compose version mismatch: expected {pins['compose']}, got {version}")
                version = json.loads(harness.run([kubectl, 'version', '--client', '-o', 'json'], workspace))
                harness.report['versions']['kubectl'] = version
                if version['clientVersion']['gitVersion'].removeprefix('v') != pins['kubectl'].removeprefix('v') or version['kustomizeVersion'].removeprefix('v') != pins['kustomize'].removeprefix('v'):
                    raise ValidationError('kubectl/Kustomize version mismatch')
                harness.run([compose, '-f', output / 'docker-compose.yml', 'config'], workspace)
                trees = [output / 'k8s/base'] + sorted(p.parent for p in (output / 'k8s/overlays').rglob('kustomization.yaml'))
                if len(trees) < 2:
                    raise ValidationError('compile missing Kubernetes overlays')
                for tree in trees:
                    harness.run([kubectl, 'kustomize', tree], workspace)
            harness.finish()
            return 0
    except Exception as exc:
        harness.finish(exc)
        print(f'release validation failed: {exc}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
