import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

MODULE_PATH = Path(__file__).resolve().parents[1] / "scripts/install-validation-tools.py"
spec = importlib.util.spec_from_file_location("validation_tools", MODULE_PATH)
tools = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tools)


class ValidationToolsTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def run_install(self):
        return tools.install(self.root / "bin", self.root / "report", "linux", "x86_64")

    def report(self):
        return json.loads((self.root / "report/tool-downloads.json").read_text())

    def test_verified_payload_installed_executable_with_evidence(self):
        payload = b"test binary"
        checksum = hashlib.sha256(payload).hexdigest().encode()
        with patch.object(tools, "download", side_effect=lambda url, *args: checksum if url.endswith(".sha256") else payload):
            self.run_install()
        self.assertEqual((self.root / "bin/docker-compose").read_bytes(), payload)
        self.assertTrue((self.root / "bin/kubectl").stat().st_mode & 0o111)
        self.assertEqual(self.report()["status"], "verified")

    def test_checksum_mismatch_never_installs(self):
        with patch.object(tools, "download", side_effect=[b"0" * 64, b"bad"]):
            with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
                self.run_install()
        self.assertFalse((self.root / "bin/docker-compose").exists())
        self.assertEqual(self.report()["status"], "failed")
        self.assertIn("actual_sha256", self.report()["tools"][0])

    def test_network_failure_records_failure(self):
        with patch.object(tools, "download", side_effect=TimeoutError("network timeout")):
            with self.assertRaises(TimeoutError):
                self.run_install()
        self.assertIn("network timeout", self.report()["error"])

    def test_missing_version_fails_before_download(self):
        with patch.object(tools.json, "loads", return_value={}), patch.object(tools, "download") as download:
            with self.assertRaises(KeyError):
                self.run_install()
            download.assert_not_called()
        self.assertEqual(self.report()["status"], "failed")

    def test_urls_map_arm_and_reject_invalid_versions(self):
        self.assertIn("darwin-aarch64", tools.urls("compose", "5.3.1", "darwin", "arm64"))
        self.assertIn("linux/amd64", tools.urls("kubectl", "1.36.1", "linux", "x86_64"))
        with self.assertRaises(ValueError):
            tools.urls("compose", "", "linux", "x86_64")

    def test_download_bounds(self):
        class Response:
            headers = {"Content-Length": "10"}
            def __enter__(self):
                return self
            def __exit__(self, *args):
                pass
        with patch.object(tools.urllib.request, "urlopen", return_value=Response()):
            with self.assertRaisesRegex(ValueError, "size limit"):
                tools.download("https://example.test/tool", 5)


if __name__ == "__main__":
    unittest.main()
