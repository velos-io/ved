"""Tests verifying that the native ved compiler processes ved.yaml accurately."""

import os
from pathlib import Path
import subprocess
import pytest
import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent
VEW_BIN = REPO_ROOT / "vew"
VED_YAML = REPO_ROOT / "examples" / "full" / "ved.yaml"
VEDFILE = REPO_ROOT / "examples" / "full" / "Vedfile"


@pytest.fixture(scope="module")
def compiled_output(tmp_path_factory):
    """Compile ved.yaml into a temporary directory using the native ved engine."""
    out_dir = tmp_path_factory.mktemp("ved_compiled")
    env = os.environ.copy()
    env["REDIS_ENABLED"] = "true"
    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(VED_YAML),
            "-t",
            "compose",
            "-o",
            str(out_dir),
        ],
        check=True,
        capture_output=True,
        env=env,
    )
    return out_dir


def test_compose_generation(compiled_output):
    """Verify that ved compile emits valid docker-compose.yml with all services and jobs."""
    compose_file = compiled_output / "docker-compose.yml"
    assert compose_file.exists(), "docker-compose.yml was not generated"

    doc = yaml.safe_load(compose_file.read_text(encoding="utf-8"))
    services = doc.get("services", {})

    # Check that services exist
    assert "postgres" in services
    assert "redis" in services
    assert "lambda-emulator" in services
    assert "api-gateway" in services

    # Check that jobs exist
    assert "flyway-demo" in services
    assert "smoke-test" in services
    assert "cleanup-job" in services
    assert "report-export" in services

    # Check networks and volumes
    assert "demo-net" in doc.get("networks", {})
    assert "postgres-data" in doc.get("volumes", {})


def test_generated_env():
    """Verify that config/.generated.env contains required runtime parameters."""
    env_file = REPO_ROOT / "config" / ".generated.env"
    assert env_file.exists(), "config/.generated.env was not found"
    content = env_file.read_text(encoding="utf-8")
    assert "INGRESS_SUFFIX=" in content
    assert "APP_VERSION=" in content


def test_k8s_generation(tmp_path):
    """Verify that ved compile -t k8s emits valid Kustomize manifests."""
    out_k8s = tmp_path / "k8s"
    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(VED_YAML),
            "-t",
            "k8s",
            "-o",
            str(out_k8s),
        ],
        check=True,
        capture_output=True,
    )

    assert (out_k8s / "base" / "kustomization.yaml").exists()
    assert (out_k8s / "base" / "infra" / "kustomization.yaml").exists()
    assert (out_k8s / "base" / "agent" / "kustomization.yaml").exists()
    assert (out_k8s / "base" / "tools" / "kustomization.yaml").exists()


def test_vedfile_starlark_compile(tmp_path):
    """Verify that ved compile -f Vedfile evaluates Starlark and emits valid compose manifest."""
    vedfile = VEDFILE
    out_dir = tmp_path / "starlark_out"
    env = os.environ.copy()
    env["REDIS_ENABLED"] = "true"
    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(vedfile),
            "-t",
            "compose",
            "-o",
            str(out_dir),
        ],
        check=True,
        capture_output=True,
        env=env,
    )

    compose_file = out_dir / "docker-compose.yml"
    assert compose_file.exists()
    doc = yaml.safe_load(compose_file.read_text(encoding="utf-8"))
    services = doc.get("services", {})
    assert "postgres" in services
    assert "redis" in services
    assert "lambda-emulator" in services
    assert "api-gateway" in services
    assert "flyway-demo" in services
    assert "smoke-test" in services


def test_conditional_enablement_and_pruning(tmp_path):
    """Verify that setting <WORKLOAD>_ENABLED=false prunes it and its downstream dependencies."""
    vedfile = VEDFILE
    out_dir = tmp_path / "pruned_out"

    env = os.environ.copy()
    env["REPORT_EXPORT_ENABLED"] = "false"
    env["OTEL_COLLECTOR_ENABLED"] = "false"

    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(vedfile),
            "-t",
            "compose",
            "-o",
            str(out_dir),
        ],
        check=True,
        capture_output=True,
        env=env,
    )

    compose_file = out_dir / "docker-compose.yml"
    assert compose_file.exists()
    doc = yaml.safe_load(compose_file.read_text(encoding="utf-8"))
    services = doc.get("services", {})

    # Pruned workloads must not appear
    assert "report-export" not in services
    assert "otel-collector" not in services

    # Active workloads must still have their valid dependencies
    api_gw = services.get("api-gateway", {})
    deps = api_gw.get("depends_on", {})
    assert "lambda-emulator" in deps


def test_group_mapping_to_profiles(tmp_path):
    """Verify that workloads in non-core groups receive docker-compose profiles."""
    vedfile = VEDFILE
    out_dir = tmp_path / "group_out"

    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(vedfile),
            "-t",
            "compose",
            "-o",
            str(out_dir),
        ],
        check=True,
        capture_output=True,
    )

    doc = yaml.safe_load((out_dir / "docker-compose.yml").read_text(encoding="utf-8"))
    services = doc.get("services", {})
    otel = services.get("otel-collector", {})
    profiles = otel.get("profiles", [])
    assert "tools" in profiles


def test_centralised_env_files_and_cascade(tmp_path):
    """Verify that App(env_files=...) drives docker-compose env_file and .generated.env."""
    vedfile = VEDFILE
    out_dir = tmp_path / "env_out"

    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(vedfile),
            "-t",
            "compose",
            "-o",
            str(out_dir),
        ],
        check=True,
        capture_output=True,
    )

    doc = yaml.safe_load((out_dir / "docker-compose.yml").read_text(encoding="utf-8"))
    services = doc.get("services", {})
    pg = services.get("postgres", {})
    env_file = pg.get("env_file", [])
    assert env_file == ["config/.env", "config/.local.env"]

    # Verify config/.generated.env has VED_ENV_FILES
    gen_env = (REPO_ROOT / "config" / ".generated.env").read_text(encoding="utf-8")
    assert 'VED_ENV_FILES="config/.env config/.local.env"' in gen_env


def test_redis_flag_toggle_and_dependency_pruning(tmp_path):
    """Verify that REDIS_ENABLED=true includes Redis and REDIS_ENABLED=false prunes it and its dependencies."""
    vedfile = VEDFILE

    # 1. Test flag ON
    out_on = tmp_path / "redis_on"
    env_on = os.environ.copy()
    env_on["REDIS_ENABLED"] = "true"
    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(vedfile),
            "-t",
            "compose",
            "-o",
            str(out_on),
        ],
        check=True,
        capture_output=True,
        env=env_on,
    )
    doc_on = yaml.safe_load((out_on / "docker-compose.yml").read_text(encoding="utf-8"))
    svcs_on = doc_on.get("services", {})
    assert "redis" in svcs_on, "Redis should be present when REDIS_ENABLED=true"
    api_gw_deps_on = svcs_on.get("api-gateway", {}).get("depends_on", {})
    assert "redis" in api_gw_deps_on, "api-gateway should depend on redis when active"
    assert "lambda-emulator" in api_gw_deps_on

    # 2. Test flag OFF
    out_off = tmp_path / "redis_off"
    env_off = os.environ.copy()
    env_off["REDIS_ENABLED"] = "false"
    subprocess.run(
        [
            str(VEW_BIN),
            "compile",
            "-f",
            str(vedfile),
            "-t",
            "compose",
            "-o",
            str(out_off),
        ],
        check=True,
        capture_output=True,
        env=env_off,
    )
    doc_off = yaml.safe_load((out_off / "docker-compose.yml").read_text(encoding="utf-8"))
    svcs_off = doc_off.get("services", {})
    assert "redis" not in svcs_off, "Redis should be pruned when REDIS_ENABLED=false"
    api_gw_deps_off = svcs_off.get("api-gateway", {}).get("depends_on", {})
    assert "redis" not in api_gw_deps_off, "api-gateway must prune dependency on absent redis"
    assert "lambda-emulator" in api_gw_deps_off, "api-gateway must retain valid dependency on lambda-emulator"




