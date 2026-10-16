"""
tests/test_generate.py — Comprehensive test suite for Velos compiler.

Covers all 9 required test classes and 50+ test cases.
Runs cleanly under pytest and unittest.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path
import unittest

# Ensure project root and generate.py are importable
PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PROJECT_ROOT))
sys.path.insert(0, str(PROJECT_ROOT / "installer" / "ved"))

try:
    import yaml
except ImportError:
    # Stub minimal PyYAML if running in an environment without it
    class _MockYAML:
        @staticmethod
        def safe_load(f):
            return {}

        @staticmethod
        def safe_load_all(f):
            return []

        @staticmethod
        def dump(obj, *args, **kwargs):
            return str(obj)

        @staticmethod
        def dump_all(objs, *args, **kwargs):
            return "\n---\n".join(str(o) for o in objs)

    sys.modules.setdefault("yaml", _MockYAML)
    import yaml

import generate


class TestLooksLikeSecret(unittest.TestCase):
    def test_exact_secret_suffixes(self):
        for key in ("DB_PASSWORD", "API_KEY", "AUTH_TOKEN", "CLIENT_SECRET", "MY_CERT", "ROOT_CERTIFICATE"):
            self.assertTrue(generate._looks_like_secret(key), f"Expected {key} to be identified as a secret")

    def test_non_secret_keys(self):
        for key in ("APP_URL", "APP_PORT", "ENABLE_METRICS", "RETENTION_DAYS", "OTEL_RESOURCE_ATTRIBUTES"):
            self.assertFalse(generate._looks_like_secret(key), f"Expected {key} to be non-secret")

    def test_case_insensitive(self):
        self.assertTrue(generate._looks_like_secret("linz_api_key"))
        self.assertTrue(generate._looks_like_secret("db_password"))
        self.assertTrue(generate._looks_like_secret("auth_token"))

    def test_no_substring_match(self):
        self.assertFalse(generate._looks_like_secret("MONKEY_JOCKEY"))
        self.assertFalse(generate._looks_like_secret("KEYBOARD"))

    def test_bare_secret_tokens(self):
        self.assertTrue(generate._looks_like_secret("TOKEN"))
        self.assertTrue(generate._looks_like_secret("PASSWORD"))
        self.assertTrue(generate._looks_like_secret("SECRET"))


class TestLoadEnvFile(unittest.TestCase):
    def test_redact_mode_skips_secrets(self):
        with tempfile.NamedTemporaryFile("w+", delete=False) as f:
            f.write("PORT=8080\nDB_PASSWORD=secret123\nNAME=app\n")
            f_path = f.name
        try:
            loaded = generate._load_env_file(f_path, redact=True)
            self.assertIn("PORT", loaded)
            self.assertIn("NAME", loaded)
            self.assertNotIn("DB_PASSWORD", loaded)
        finally:
            os.remove(f_path)

    def test_non_redact_returns_all(self):
        with tempfile.NamedTemporaryFile("w+", delete=False) as f:
            f.write("PORT=8080\nDB_PASSWORD=secret123\n")
            f_path = f.name
        try:
            loaded = generate._load_env_file(f_path, redact=False)
            self.assertIn("PORT", loaded)
            self.assertIn("DB_PASSWORD", loaded)
            self.assertEqual(loaded["DB_PASSWORD"], "secret123")
        finally:
            os.remove(f_path)

    def test_missing_file_silently_skipped(self):
        loaded = generate._load_env_file("/path/to/nonexistent/file.env")
        self.assertEqual(loaded, {})

    def test_blank_lines_and_comments_ignored(self):
        with tempfile.NamedTemporaryFile("w+", delete=False) as f:
            f.write("# Header comment\n\nPORT=8080\n   # Indented comment\nHOST=localhost\n")
            f_path = f.name
        try:
            loaded = generate._load_env_file(f_path)
            self.assertEqual(loaded, {"PORT": "8080", "HOST": "localhost"})
        finally:
            os.remove(f_path)

    def test_values_stripped_of_quotes(self):
        with tempfile.NamedTemporaryFile("w+", delete=False) as f:
            f.write('KEY1="double_quoted"\nKEY2=\'single_quoted\'\nKEY3=unquoted\n')
            f_path = f.name
        try:
            loaded = generate._load_env_file(f_path)
            self.assertEqual(loaded["KEY1"], "double_quoted")
            self.assertEqual(loaded["KEY2"], "single_quoted")
            self.assertEqual(loaded["KEY3"], "unquoted")
        finally:
            os.remove(f_path)


class TestExpandImage(unittest.TestCase):
    def test_app_source_expansion(self):
        sources = {"app_source": "demo"}
        expanded = generate._expand_image("${app_source}/my-service:0.1.0", sources)
        self.assertEqual(expanded, "demo/my-service:0.1.0")

    def test_unknown_variable_left_intact(self):
        sources = {"app_source": "demo"}
        expanded = generate._expand_image("${UNKNOWN_VAR}/image:tag", sources)
        self.assertEqual(expanded, "${UNKNOWN_VAR}/image:tag")

    def test_none_or_empty_unchanged(self):
        self.assertIsNone(generate._expand_image(None, {}))
        self.assertEqual(generate._expand_image("", {}), "")

    def test_multiple_tokens_expanded(self):
        sources = {"reg": "acr.io", "env": "prod"}
        expanded = generate._expand_image("${reg}/${env}/app:1.0", sources)
        self.assertEqual(expanded, "acr.io/prod/app:1.0")


class TestGenerateCompose(unittest.TestCase):
    def setUp(self):
        self.spec = {
            "global": {
                "sources": {"app_source": "demo", "ubi_registry_mirror": "redhat.com"},
                "registries": {"npm": "https://npm.pkg"},
                "profiles": {
                    "core": ["db", "migrator"],
                    "optional": ["worker"],
                },
                "profileActivation": {
                    "core": {},
                    "optional": {"when": {"OPT": "true"}},
                },
            },
            "compose": {
                "network": "my-net",
                "volumes": {"my-vol": None},
            },
            "services": {
                "app": {
                    "image": "${app_source}/app:1.0",
                    "ports": ["8080:8080"],
                    "envFile": ["config/.env"],
                    "env": {"PORT": "8080"},
                    "healthcheck": {
                        "test": ["CMD", "curl"],
                        "startPeriod": "15s",
                    },
                    "waitsFor": {
                        "db": "service_healthy",
                        "worker": "service_started",
                    },
                    "restart": "no",
                    "targets": {
                        "dev": {
                            "build": {"context": ".", "dockerfile": "Dockerfile"},
                            "image": "override-image:dev",
                        }
                    },
                },
                "db": {
                    "image": "postgres:16",
                },
                "worker": {
                    "image": "worker:1.0",
                },
            },
        }

    def test_env_file_not_inlined(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            app_svc = doc["services"]["app"]
            self.assertIn("env_file", app_svc)
            self.assertEqual(app_svc["env_file"], ["config/.env"])
            self.assertEqual(app_svc["environment"], {"PORT": "8080"})

    def test_profile_assignment(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            self.assertEqual(doc["services"]["db"]["profiles"], ["core"])
            self.assertEqual(doc["services"]["worker"]["profiles"], ["optional"])

    def test_no_profiles_omits_profiles_key(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file, no_profiles=True)
            doc = yaml.safe_load(open(out_file))
            self.assertNotIn("profiles", doc["services"]["db"])
            self.assertNotIn("profiles", doc["services"]["worker"])

    def test_depends_on_always_on_kept(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            app_svc = doc["services"]["app"]
            self.assertIn("db", app_svc.get("depends_on", {}))

    def test_depends_on_cross_profile_dropped(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            app_svc = doc["services"]["app"]
            # app (profile-free) -> worker (conditional profile) must be dropped
            self.assertNotIn("worker", app_svc.get("depends_on", {}))

    def test_start_period_renamed(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            hc = doc["services"]["app"]["healthcheck"]
            self.assertIn("start_period", hc)
            self.assertNotIn("startPeriod", hc)
            self.assertEqual(hc["start_period"], "15s")

    def test_build_args_injected(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            build_block = doc["services"]["app"]["build"]
            self.assertEqual(build_block["args"]["NPM_REGISTRY"], "https://npm.pkg")
            self.assertEqual(build_block["args"]["REDHAT_BASE_REGISTRY"], "redhat.com")

    def test_network_assignment(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            self.assertIn("my-net", doc["services"]["app"]["networks"])
            self.assertIn("my-net", doc["networks"])

    def test_targets_dev_image_override(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            doc = yaml.safe_load(open(out_file))
            self.assertEqual(doc["services"]["app"]["image"], "override-image:dev")

    def test_restart_no_quoted(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "docker-compose.yml")
            generate._generate_compose(self.spec, self.spec["global"]["sources"], {}, out_file)
            with open(out_file, "r") as f:
                content = f.read()
            self.assertIn("restart: 'no'", content)
            self.assertNotIn("restart: no\n", content)


class TestGenerateKustomize(unittest.TestCase):
    def setUp(self):
        self.spec = {
            "global": {
                "sources": {"app_source": "demo"},
                "kubernetes": {
                    "secretsGroup": "infra",
                    "extraBaseResourcesGroup": "tools",
                },
            },
            "services": {
                "web": {
                    "image": "nginx:alpine",
                    "ports": ["80:80"],
                    "env": {"DB_PASSWORD": "secret", "PORT": "80"},
                    "targets": {
                        "helm": {
                            "replicas": 2,
                            "ingress": {"hosts": ["web.local"]},
                        }
                    },
                },
                "migrator": {
                    "image": "migrator:1.0",
                    "targets": {
                        "helm": {
                            "namespaceGroup": "infra",
                            "annotations": {"helm.sh/hook": "pre-install"},
                        }
                    },
                },
                "backup": {
                    "image": "backup:1.0",
                    "restart": "no",
                    "targets": {"helm": {}},
                },
                "cron": {
                    "image": "cron:1.0",
                    "targets": {"helm": {"schedule": "0 0 * * *"}},
                },
                "disabled-svc": {
                    "image": "disabled:1.0",
                    "targets": {"helm": {"enabled": False}},
                },
                "flag-disabled-svc": {
                    "image": "disabled2:1.0",
                    "targets": {"helm": {"enabledFromEnv": "FEATURE_X"}},
                },
            },
        }

    def test_deployment_produced_for_standard_service(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            docs = list(yaml.safe_load_all(open(os.path.join(td, "agent", "web.yaml"))))
            kinds = [d["kind"] for d in docs]
            self.assertIn("Deployment", kinds)
            deploy = [d for d in docs if d["kind"] == "Deployment"][0]
            self.assertEqual(deploy["spec"]["replicas"], 2)

    def test_oneshot_job_produced_for_restart_no(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            docs = list(yaml.safe_load_all(open(os.path.join(td, "agent", "backup.yaml"))))
            job = [d for d in docs if d["kind"] == "Job"][0]
            self.assertEqual(job["spec"]["template"]["spec"]["restartPolicy"], "OnFailure")

    def test_hook_job_produced_for_hook_annotation(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            docs = list(yaml.safe_load_all(open(os.path.join(td, "infra", "migrator.yaml"))))
            job = [d for d in docs if d["kind"] == "Job"][0]
            self.assertEqual(job["spec"]["template"]["spec"]["restartPolicy"], "Never")
            self.assertIn("helm.sh/hook", job["metadata"]["annotations"])

    def test_cronjob_produced_for_schedule(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            docs = list(yaml.safe_load_all(open(os.path.join(td, "agent", "cron.yaml"))))
            cron = [d for d in docs if d["kind"] == "CronJob"][0]
            self.assertEqual(cron["spec"]["schedule"], "0 0 * * *")
            self.assertEqual(cron["spec"]["concurrencyPolicy"], "Forbid")

    def test_hook_job_suppresses_service_and_ingress(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            docs = list(yaml.safe_load_all(open(os.path.join(td, "infra", "migrator.yaml"))))
            kinds = [d["kind"] for d in docs]
            self.assertNotIn("Service", kinds)
            self.assertNotIn("Ingress", kinds)

    def test_secret_redaction_into_secrets_yaml(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            sec_file = os.path.join(td, "infra", "secrets.yaml")
            self.assertTrue(os.path.exists(sec_file))
            sec = yaml.safe_load(open(sec_file))
            self.assertEqual(sec["stringData"]["DB_PASSWORD"], "placeholder")

            web_docs = list(yaml.safe_load_all(open(os.path.join(td, "agent", "web.yaml"))))
            cm = [d for d in web_docs if d["kind"] == "ConfigMap"][0]
            self.assertNotIn("DB_PASSWORD", cm["data"])
            self.assertEqual(cm["data"]["PORT"], "80")

    def test_env_from_includes_secret_ref(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            web_docs = list(yaml.safe_load_all(open(os.path.join(td, "agent", "web.yaml"))))
            deploy = [d for d in web_docs if d["kind"] == "Deployment"][0]
            c = deploy["spec"]["template"]["spec"]["containers"][0]
            env_from = c.get("envFrom", [])
            self.assertTrue(any(e.get("secretRef", {}).get("name") == "app-secrets" for e in env_from))

    def test_namespace_group_routing(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            self.assertTrue(os.path.exists(os.path.join(td, "infra", "migrator.yaml")))
            self.assertTrue(os.path.exists(os.path.join(td, "agent", "web.yaml")))

    def test_enabled_from_env_false_skips_service(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {"FEATURE_X": "false"}, {}, td)
            self.assertFalse(os.path.exists(os.path.join(td, "agent", "disabled-svc.yaml")))
            self.assertFalse(os.path.exists(os.path.join(td, "agent", "flag-disabled-svc.yaml")))

    def test_k8s_env_token_resolution(self):
        spec = {
            "global": {"kubernetes": {"secretsGroup": "infra"}},
            "services": {
                "client": {
                    "image": "client:1.0",
                    "targets": {
                        "helm": {
                            "k8s_env": {"HOST": "db.{infra_ns}.svc"},
                        }
                    },
                }
            },
        }
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(spec, {}, {}, {"infra_ns": "my-infra"}, td)
            docs = list(yaml.safe_load_all(open(os.path.join(td, "agent", "client.yaml"))))
            cm = [d for d in docs if d["kind"] == "ConfigMap"][0]
            self.assertEqual(cm["data"]["HOST"], "db.my-infra.svc")

    def test_ingress_generated_when_hosts_set(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_base(self.spec, {}, {}, {}, td)
            docs = list(yaml.safe_load_all(open(os.path.join(td, "agent", "web.yaml"))))
            ing = [d for d in docs if d["kind"] == "Ingress"][0]
            self.assertEqual(ing["spec"]["rules"][0]["host"], "web.local")
            self.assertEqual(ing["spec"]["ingressClassName"], "nginx")


class TestGenerateLocalOverlays(unittest.TestCase):
    def setUp(self):
        self.spec = {
            "global": {
                "sources": {"app_source": "demo"},
                "kubernetes": {
                    "ingress_suffix": "app.localapp",
                    "podSecurityEnabled": False,
                    "secretsGroup": "infra",
                    "extraBaseResourcesGroup": "tools",
                },
            },
            "services": {
                "api": {
                    "image": "${app_source}/api:1.0",
                    "targets": {
                        "dev": {"build": {"context": "."}},
                        "helm": {
                            "namespaceGroup": "agent",
                            "localSecurityPatch": True,
                            "ingress": {"localSlotPrefix": "api"},
                        },
                    },
                },
                "db": {
                    "image": "postgres:16",
                    "targets": {"helm": {"namespaceGroup": "infra"}},
                },
                "monitor": {
                    "image": "monitor:1.0",
                    "targets": {
                        "helm": {
                            "namespaceGroup": "tools",
                            "localSecurityPatch": True,
                            "localSecurityContext": {"runAsUser": 5000},
                        }
                    },
                },
            },
        }

    def test_local_agent_overlay_namespace_and_images(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_overlays(self.spec, {"app_source": "demo"}, {"K8S_APP_SLOT": "2"}, {}, td)
            kust = yaml.safe_load(open(os.path.join(td, "local", "ns-2", "kustomization.yaml")))
            self.assertEqual(kust["namespace"], "ns-2")
            self.assertEqual(kust["resources"], ["../../../base/agent"])
            self.assertTrue(any(img["name"] == "demo/api" for img in kust.get("images", [])))

    def test_ingress_patch_contains_slot_hostname(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_overlays(self.spec, {"app_source": "demo"}, {"K8S_APP_SLOT": "3"}, {}, td)
            ing = yaml.safe_load(open(os.path.join(td, "local", "ns-3", "ingress-patch.yaml")))
            self.assertEqual(ing["spec"]["rules"][0]["host"], "api3.app.localapp")

    def test_local_security_patch_when_pod_security_disabled(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_overlays(self.spec, {}, {}, {}, td)
            patch = yaml.safe_load(open(os.path.join(td, "local", "ns-1", "local-security-patch.yaml")))
            self.assertEqual(patch["metadata"]["name"], "api")
            self.assertFalse(patch["spec"]["template"]["spec"]["securityContext"]["runAsNonRoot"])

    def test_local_security_patch_custom_contexts(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_overlays(self.spec, {}, {}, {"tools_ns": "tools"}, td)
            patch = yaml.safe_load(open(os.path.join(td, "local", "tools", "local-security-patch.yaml")))
            self.assertEqual(patch["spec"]["template"]["spec"]["securityContext"]["runAsUser"], 5000)

    def test_infra_and_tools_overlays_created(self):
        with tempfile.TemporaryDirectory() as td:
            generate._generate_kustomize_overlays(self.spec, {}, {}, {"infra_ns": "infra", "tools_ns": "tools"}, td)
            self.assertTrue(os.path.exists(os.path.join(td, "local", "infra", "kustomization.yaml")))
            self.assertTrue(os.path.exists(os.path.join(td, "local", "tools", "kustomization.yaml")))


class TestGenerateValues(unittest.TestCase):
    def setUp(self):
        self.spec = {
            "global": {"appVersion": "1.0.0", "sources": {"app_source": "myorg"}},
            "services": {
                "svc": {
                    "image": "${app_source}/svc:1.0",
                    "memory": "512m",
                    "ports": ["8080:8080"],
                    "env": {"PORT": "8080", "DB_PASSWORD": "secret"},
                    "volumes": ["data-vol:/data", "./config:/etc/cfg:ro"],
                    "targets": {"helm": {"replicas": 3}},
                }
            },
        }

    def test_secret_keys_stripped_from_values_env(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "values.yaml")
            generate._generate_helm_values(self.spec, self.spec["global"]["sources"], {}, out_file)
            vals = yaml.safe_load(open(out_file))
            self.assertNotIn("DB_PASSWORD", vals["services"]["svc"]["env"])
            self.assertEqual(vals["services"]["svc"]["env"]["PORT"], "8080")

    def test_memory_normalised_to_binary_units(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "values.yaml")
            generate._generate_helm_values(self.spec, self.spec["global"]["sources"], {}, out_file)
            vals = yaml.safe_load(open(out_file))
            self.assertEqual(vals["services"]["svc"]["memory"], "512Mi")

    def test_image_expanded_via_sources(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "values.yaml")
            generate._generate_helm_values(self.spec, self.spec["global"]["sources"], {}, out_file)
            vals = yaml.safe_load(open(out_file))
            self.assertEqual(vals["services"]["svc"]["image"], "myorg/svc:1.0")

    def test_volumes_normalised(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "values.yaml")
            generate._generate_helm_values(self.spec, self.spec["global"]["sources"], {}, out_file)
            vals = yaml.safe_load(open(out_file))
            vols = vals["services"]["svc"]["volumes"]
            self.assertEqual(len(vols), 2)
            self.assertTrue(any(v.get("type") == "named" and v.get("name") == "data-vol" for v in vols))
            self.assertTrue(any(v.get("type") == "hostPath" and v.get("readOnly") is True for v in vols))

    def test_replicas_and_ports_mapped(self):
        with tempfile.TemporaryDirectory() as td:
            out_file = os.path.join(td, "values.yaml")
            generate._generate_helm_values(self.spec, self.spec["global"]["sources"], {}, out_file)
            vals = yaml.safe_load(open(out_file))
            self.assertEqual(vals["services"]["svc"]["replicas"], 3)
            self.assertEqual(vals["services"]["svc"]["ports"][0]["port"], 8080)


class TestBuildEnvExpansion(unittest.TestCase):
    def test_var_with_default_unset(self):
        expanded = generate._expand_build_env("dockerfiles/${TAG:-v1}/Dockerfile", {})
        self.assertEqual(expanded, "dockerfiles/v1/Dockerfile")

    def test_var_with_default_set(self):
        expanded = generate._expand_build_env("dockerfiles/${TAG:-v1}/Dockerfile", {"TAG": "v2"})
        self.assertEqual(expanded, "dockerfiles/v2/Dockerfile")

    def test_plain_var_expansion(self):
        expanded = generate._expand_build_env("path/${NAME}/file", {"NAME": "custom"})
        self.assertEqual(expanded, "path/custom/file")


class TestIntegration(unittest.TestCase):
    def test_compile_ved_spec_end_to_end(self):
        ved_path = PROJECT_ROOT / "examples" / "full" / "ved.yaml"
        self.assertTrue(ved_path.exists(), "ved.yaml does not exist")
        with tempfile.TemporaryDirectory() as td:
            out_compose = os.path.join(td, "docker-compose.yml")
            vew_bin = PROJECT_ROOT / "vew"
            subprocess.run(
                [str(vew_bin), "compile", "-f", str(ved_path), "-t", "compose", "-o", td],
                check=True,
                capture_output=True,
            )
            self.assertTrue(os.path.exists(out_compose))

    def test_compose_yaml_parseable(self):
        c_path = PROJECT_ROOT / "examples" / "full" / "docker-compose.yml"
        if c_path.exists():
            doc = yaml.safe_load(open(c_path))
            self.assertIn("services", doc)
            self.assertIn("lambda-emulator", doc["services"])

    def test_k8s_base_kustomization_exists(self):
        kust_path = PROJECT_ROOT / "installer" / "k8s" / "base" / "kustomization.yaml"
        if kust_path.exists():
            doc = yaml.safe_load(open(kust_path))
            self.assertIn("agent", doc["resources"])
            self.assertIn("infra", doc["resources"])

    def test_generated_env_contains_keys(self):
        gen_env_path = PROJECT_ROOT / "config" / ".generated.env"
        if gen_env_path.exists():
            content = gen_env_path.read_text(encoding="utf-8")
            self.assertIn("INGRESS_SUFFIX=", content)
            self.assertIn("APP_VERSION=", content)
            self.assertIn("K8S_NAMESPACE=", content)


if __name__ == "__main__":
    unittest.main()
