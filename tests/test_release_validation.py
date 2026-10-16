"""Portable release gate failure and archive safety regressions."""
import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('release_validation', ROOT / 'scripts/release-validation.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReleaseValidationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def tool(self, body):
        path = self.root / 'tool'
        path.write_text('#!' + sys.executable + '\n' + body)
        path.chmod(0o755)
        return path

    def harness(self, **kwargs):
        return module.Harness(self.root / 'report', **kwargs)

    def test_missing_tool_retains_report(self):
        result = module.main(['--ved', str(self.root / 'missing'), '--smoke-only', '--report-dir', str(self.root / 'report')])
        self.assertEqual(result, 1)
        report = json.loads((self.root / 'report/report.json').read_text())
        self.assertIn('missing', report['error'])

    def test_failing_tool_retains_diagnostics(self):
        harness = self.harness()
        with self.assertRaisesRegex(module.ValidationError, 'exit 7'):
            harness.run([self.tool("import sys\nprint('failure details', file=sys.stderr)\nsys.exit(7)\n")], self.root)
        self.assertIn('failure details', Path(harness.report['commands'][0]['stderr']).read_text())

    def test_timeout(self):
        harness = self.harness(timeout=.1)
        with self.assertRaisesRegex(module.ValidationError, 'timed out'):
            harness.run([self.tool('import time\ntime.sleep(10)\n')], self.root)

    def test_output_limit(self):
        harness = self.harness(output_limit=100)
        with self.assertRaisesRegex(module.ValidationError, 'output limit'):
            harness.run([self.tool("print('x' * 1000)\n")], self.root)
        self.assertEqual(Path(harness.report['commands'][0]['stdout']).stat().st_size, 100)

    def test_environment_isolated(self):
        import os
        os.environ['VED_RELEASE_TEST_SECRET'] = 'do-not-inherit'
        try:
            output = self.harness().run([self.tool("import os\nassert 'VED_RELEASE_TEST_SECRET' not in os.environ\nassert os.environ['KUBECONFIG'].endswith('absent-kubeconfig')\n")], self.root)
            self.assertEqual(output, '')
        finally:
            del os.environ['VED_RELEASE_TEST_SECRET']

    def archive(self, name='ved', mode=0o755, kind=tarfile.REGTYPE):
        archive = self.root / 'release.tar.gz'
        with tarfile.open(archive, 'w:gz') as handle:
            info = tarfile.TarInfo(name)
            info.mode, info.type = mode, kind
            info.size = 1 if kind == tarfile.REGTYPE else 0
            info.linkname = '/bin/sh'
            handle.addfile(info, io.BytesIO(b'x') if info.size else None)
        return archive

    def test_unsafe_archive_entries_rejected(self):
        for name, kind in [('../ved', tarfile.REGTYPE), ('/ved', tarfile.REGTYPE), ('ved', tarfile.SYMTYPE), ('ved', tarfile.LNKTYPE), ('ved', tarfile.DIRTYPE)]:
            with self.subTest(name=name, kind=kind):
                with self.assertRaisesRegex(module.ValidationError, 'exactly one regular'):
                    module.archive_binary(self.archive(name=name, kind=kind), self.root)

    def test_archive_extra_entry_rejected(self):
        archive = self.root / 'extra.tar.gz'
        with tarfile.open(archive, 'w:gz') as handle:
            for name in ['ved', '._ved']:
                info = tarfile.TarInfo(name)
                info.mode, info.size = 0o755, 1
                handle.addfile(info, io.BytesIO(b'x'))
        with self.assertRaisesRegex(module.ValidationError, 'exactly one regular'):
            module.archive_binary(archive, self.root)

    def test_archive_requires_executable(self):
        with self.assertRaisesRegex(module.ValidationError, 'executable'):
            module.archive_binary(self.archive(mode=0o644), self.root)

    def test_content_only_report(self):
        self.assertEqual(module.main(['--archive', str(self.archive()), '--content-only', '--report-dir', str(self.root / 'report')]), 0)
        report = json.loads((self.root / 'report/report.json').read_text())
        self.assertEqual(report['archive_evidence'], 'content only')
        self.assertEqual(report['archive_inspection']['member'], 'ved')
        self.assertEqual(report['archive_inspection']['size'], 1)
        self.assertEqual(report['archive_inspection']['mode'], '0o755')
        import hashlib
        self.assertEqual(report['archive_inspection']['extracted_binary_sha256'], hashlib.sha256(b'x').hexdigest())
        self.assertEqual(report['archive_inspection']['archive_sha256'], module.file_sha256(self.root / 'release.tar.gz'))

    def fake_ved(self):
        path = self.root / 'ved-tool'
        path.write_text('#!' + sys.executable + '''
import pathlib, sys
if '--version' in sys.argv: print('ved 0.1.0')
elif 'compile' in sys.argv:
    out = pathlib.Path(sys.argv[sys.argv.index('--output-dir') + 1])
    for name in ['docker-compose.yml', 'values.yaml', 'k8s/base/kustomization.yaml', 'k8s/overlays/dev/kustomization.yaml']:
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('invalid: [')
''')
        path.chmod(0o755)
        return path

    def test_version_mismatch_fails_gate(self):
        compose = self.tool("print('0.0.0')\n")
        self.assertEqual(module.main(['--ved', str(self.fake_ved()), '--compose', str(compose), '--kubectl', str(compose), '--report-dir', str(self.root / 'report')]), 1)
        self.assertIn('version mismatch', json.loads((self.root / 'report/report.json').read_text())['error'])

    def test_malformed_manifest_fails_gate(self):
        pins = json.loads((ROOT / 'scripts/validation-tools.json').read_text())
        compose = self.tool(f"import sys\nif 'version' in sys.argv: print({pins['compose']!r})\nelse:\n print('malformed manifest', file=sys.stderr)\n sys.exit(1)\n")
        kubectl = self.root / 'kubectl'
        kubectl.write_text('#!' + sys.executable + '\nprint(' + repr(json.dumps({'clientVersion': {'gitVersion': pins['kubectl']}, 'kustomizeVersion': pins['kustomize']})) + ')\n')
        kubectl.chmod(0o755)
        self.assertEqual(module.main(['--ved', str(self.fake_ved()), '--compose', str(compose), '--kubectl', str(kubectl), '--report-dir', str(self.root / 'report')]), 1)
        self.assertIn('malformed manifest', (self.root / 'report/command-06.stderr').read_text())


if __name__ == '__main__':
    unittest.main()
