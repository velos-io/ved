"""Regression coverage for the locked, expiring advisory exception boundary."""
import copy
from datetime import datetime, timezone
import importlib.util
from pathlib import Path
import unittest

MODULE = Path(__file__).resolve().parents[1] / "scripts/check-advisory-exceptions.py"
spec = importlib.util.spec_from_file_location("advisory_exceptions", MODULE)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def policy():
    return {"graph": {"all-features": True}, "advisories": {"unmaintained": "all", "unsound": "all", "yanked": "deny", "unused-ignored-advisory": "deny", "ignore": [
        {"id": advisory, "reason": "Reviewed maintenance notice"} for advisory in guard.EXCEPTIONS
    ]}}


def graph():
    packages = {}
    nodes = {}
    def package(name, version):
        key = f"{name}@{version}"
        packages[key] = {"id": key, "name": name, "version": version, "source": guard.REGISTRY}
        nodes.setdefault(key, {"id": key, "deps": []})
        return key
    for name, version, parents in guard.EXCEPTIONS.values():
        target = package(name, version)
        for parent, parent_version in parents:
            nodes[package(parent, parent_version)]["deps"].append({"pkg": target})
    return {"packages": list(packages.values()), "resolve": {"nodes": list(nodes.values())}}


class AdvisoryExceptionTests(unittest.TestCase):
    def test_reviewed_policy_and_all_target_graph_pass(self):
        guard.check_policy(policy(), datetime(2026, 10, 11, tzinfo=timezone.utc))
        guard.check_graph(graph(), "fixture")

    def test_expiry_fails_at_utc_boundary(self):
        guard.check_policy(policy(), datetime(2027, 1, 8, 23, 59, 59, tzinfo=timezone.utc))
        with self.assertRaisesRegex(ValueError, "expired"):
            guard.check_policy(policy(), guard.EXPIRES)

    def test_added_removed_and_duplicate_ignore_fail(self):
        baseline = policy()
        for entries in [baseline["advisories"]["ignore"] + [{"id": "RUSTSEC-2099-0001", "reason": "extra"}], baseline["advisories"]["ignore"][1:], baseline["advisories"]["ignore"] + [baseline["advisories"]["ignore"][0]]]:
            changed = copy.deepcopy(baseline)
            changed["advisories"]["ignore"] = entries
            with self.assertRaisesRegex(ValueError, "IDs differ"):
                guard.check_policy(changed, datetime(2026, 10, 11, tzinfo=timezone.utc))

    def test_global_advisory_exclusion_and_empty_reason_fail(self):
        for key, value in [("unmaintained", "none"), ("unsound", "workspace")]:
            changed = policy()
            changed["advisories"][key] = value
            with self.assertRaises(ValueError):
                guard.check_policy(changed, datetime(2026, 10, 11, tzinfo=timezone.utc))
        changed = policy()
        changed["advisories"]["ignore"][0]["reason"] = " "
        with self.assertRaises(ValueError):
            guard.check_policy(changed, datetime(2026, 10, 11, tzinfo=timezone.utc))

    def test_filtered_target_features_and_package_exclusions_fail(self):
        for key, value in [("all-features", False), ("targets", ["x86_64-unknown-linux-gnu"]), ("exclude", ["starlark"])]:
            changed = policy()
            changed["graph"][key] = value
            with self.assertRaises(ValueError):
                guard.check_policy(changed, datetime(2026, 10, 11, tzinfo=timezone.utc))

    def test_package_version_removal_duplicate_and_source_drift_fail(self):
        baseline = graph()
        for kind in ("version", "remove", "duplicate", "source"):
            changed = copy.deepcopy(baseline)
            target = next(p for p in changed["packages"] if p["name"] == "atomic-polyfill")
            if kind == "version":
                target["version"] = "1.0.4"
            elif kind == "remove":
                changed["packages"].remove(target)
            elif kind == "duplicate":
                duplicate = dict(target, id="atomic-polyfill@2.0.0", version="2.0.0")
                changed["packages"].append(duplicate)
            else:
                target["source"] = "git+https://example.invalid/replacement"
            with self.assertRaises(ValueError):
                guard.check_graph(changed, "fixture")

    def test_reverse_parent_add_remove_and_version_change_fail(self):
        baseline = graph()
        for kind in ("add", "remove", "version"):
            changed = copy.deepcopy(baseline)
            parent = next(p for p in changed["packages"] if p["name"] == "heapless")
            node = next(n for n in changed["resolve"]["nodes"] if n["id"] == parent["id"])
            if kind == "add":
                changed["packages"].append({"id": "new-parent", "name": "new-parent", "version": "1", "source": guard.REGISTRY})
                changed["resolve"]["nodes"].append({"id": "new-parent", "deps": [{"pkg": "atomic-polyfill@1.0.3"}]})
            elif kind == "remove":
                node["deps"] = []
            else:
                parent["version"] = "0.7.18"
            with self.assertRaisesRegex(ValueError, "parents changed"):
                guard.check_graph(changed, "fixture")


if __name__ == "__main__":
    unittest.main()
