#!/usr/bin/env python3
"""
generate.py — Ved deployment compiler.

Compiles one YAML spec into four outputs:
  1. docker-compose.yml
  2. installer/values.yaml      (Helm values)
  3. installer/k8s/             (Kustomize manifests)
  4. config/.generated.env

Entry point:
  generate.py <spec.yaml> <out-compose.yml> [out-values.yaml] [out-kustomize-dir]
              [--org ORG] [--env K=V ...]

Invoked via:
  installer/ved/bin/compile   (shell wrapper that bootstraps .venv)

Rules:
  - Zero application-specific knowledge. No if s_name == "my-service" logic.
  - All branching is driven by the spec.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import sys
from pathlib import Path
from typing import Any

import yaml


# ─────────────────────────────────────────────────────────────────────────────
# Secret detection
# ─────────────────────────────────────────────────────────────────────────────

_SECRET_SUFFIXES = (
    "_KEY", "_SECRET", "_PASSWORD", "_PASSWD", "_TOKEN",
    "_CREDENTIAL", "_CREDENTIALS", "_API_KEY", "_PRIVATE_KEY",
    "_AUTH", "_CERT", "_CERTIFICATE",
)
# Bare (no-underscore-prefix) exact matches for standalone keys like SECRET, TOKEN
_SECRET_BARE = frozenset({
    "KEY", "SECRET", "PASSWORD", "PASSWD", "TOKEN",
    "CREDENTIAL", "CREDENTIALS", "API_KEY", "PRIVATE_KEY",
    "AUTH", "CERT", "CERTIFICATE",
})


def _looks_like_secret(key: str) -> bool:
    """Return True if key matches a known secret suffix (case-insensitive, exact suffix)."""
    u = key.upper()
    # Suffix match: e.g. DB_PASSWORD ends with _PASSWORD
    for suffix in _SECRET_SUFFIXES:
        if u.endswith(suffix):
            return True
    # Bare exact match: key is literally SECRET, TOKEN, etc.
    if u in _SECRET_BARE:
        return True
    return False


# ─────────────────────────────────────────────────────────────────────────────
# Image expansion
# ─────────────────────────────────────────────────────────────────────────────

def _expand_image(image: str | None, sources: dict) -> str | None:
    """Expand ${var} tokens in image string using sources dict.

    Unknown ${var} tokens are left unchanged.
    None or empty string returned unchanged.
    """
    if not image:
        return image

    def _replace(m: re.Match) -> str:
        var = m.group(1)
        return sources.get(var, m.group(0))  # leave intact if not found

    res = re.sub(r"\$\{([^}]+)\}", _replace, image)
    while res.startswith("/"):
        res = res[1:]
    return res


# ─────────────────────────────────────────────────────────────────────────────
# Env file loader
# ─────────────────────────────────────────────────────────────────────────────

def _load_env_file(path: str, redact: bool = False) -> dict:
    """Load KEY=VALUE pairs from an env file.

    - Blank lines and comments (#) are ignored.
    - Surrounding quotes are stripped from values.
    - Inline comments after # are stripped (limitation: # inside values is removed).
    - If redact=True, secret keys are skipped and a warning is printed to stderr.
    - Missing file is silently skipped (returns empty dict).
    """
    result: dict[str, str] = {}
    try:
        with open(path, encoding="utf-8") as f:
            for raw_line in f:
                line = raw_line.strip()
                if not line or line.startswith("#"):
                    continue
                if "=" not in line:
                    continue
                key, _, val = line.partition("=")
                key = key.strip()
                # Strip inline comment
                val = val.split("#")[0].strip()
                # Strip surrounding quotes
                if (val.startswith('"') and val.endswith('"')) or \
                   (val.startswith("'") and val.endswith("'")):
                    val = val[1:-1]
                if not key:
                    continue
                if redact and _looks_like_secret(key):
                    print(f"WARN: redacting secret key '{key}' from {path}", file=sys.stderr)
                    continue
                result[key] = val
    except FileNotFoundError:
        pass  # silently skip missing files
    return result


# ─────────────────────────────────────────────────────────────────────────────
# Spec loading
# ─────────────────────────────────────────────────────────────────────────────

def _load_spec(path: str) -> dict:
    """Load and parse the YAML spec file. Exits on error."""
    if not os.path.exists(path):
        sys.exit(f"ERROR: spec file not found: {path}")
    try:
        with open(path, encoding="utf-8") as f:
            spec = yaml.safe_load(f)
        if not isinstance(spec, dict):
            sys.exit(f"ERROR: spec must be a YAML mapping, got {type(spec).__name__}: {path}")
        return spec
    except yaml.YAMLError as e:
        sys.exit(f"ERROR: invalid YAML in {path}: {e}")


# ─────────────────────────────────────────────────────────────────────────────
# Org profile merge
# ─────────────────────────────────────────────────────────────────────────────

# Mapping from org YAML keys → spec global paths
_ORG_REGISTRY_MAP = {
    "ubi":      ("sources", "ubi_registry_mirror"),
    "internal": ("sources", "internal_registry"),
    "external": ("sources", "external_registry_mirror"),
    "payment":  ("sources", "payment_registry_mirror"),
    "npm":      ("registries", "npm"),
    "pip":      ("registries", "pip"),
}

_ORG_K8S_MAP = {
    "ingress_suffix":  ("kubernetes", "ingress_suffix"),
    "image_registry":  ("sources", "k8s_registry"),
    "image_prefix":    ("sources", "k8s_image_prefix"),
    "pull_secret":     ("kubernetes", "imagePullSecret"),
}


def _merge_org_profile(spec: dict, org_path: str) -> None:
    """Merge an org YAML file into spec.global in place.

    Only sets values when the org value is a non-empty string.
    Org YAML keys map to spec global paths via _ORG_REGISTRY_MAP and _ORG_K8S_MAP.
    """
    if not os.path.exists(org_path):
        sys.exit(f"ERROR: org file not found: {org_path}")
    try:
        with open(org_path, encoding="utf-8") as f:
            org = yaml.safe_load(f) or {}
    except yaml.YAMLError as e:
        sys.exit(f"ERROR: invalid YAML in org file {org_path}: {e}")

    g = spec.setdefault("global", {})

    # Registry keys
    org_regs = org.get("registries", {}) or {}
    for org_key, (g_section, g_key) in _ORG_REGISTRY_MAP.items():
        if org_key in org_regs:
            val = org_regs[org_key]
            if val is not None:
                g.setdefault(g_section, {})[g_key] = val

    # Image keys — merge all
    org_images = org.get("images", {}) or {}
    if org_images:
        g_images = g.setdefault("images", {})
        for k, v in org_images.items():
            if v is not None:
                g_images[k] = v

    # Kubernetes keys
    org_k8s = org.get("kubernetes", {}) or {}
    for org_key, (g_section, g_key) in _ORG_K8S_MAP.items():
        if org_key in org_k8s:
            val = org_k8s[org_key]
            if val is not None:
                g.setdefault(g_section, {})[g_key] = val


# ─────────────────────────────────────────────────────────────────────────────
# Generated env writer
# ─────────────────────────────────────────────────────────────────────────────

def _write_generated_env(spec: dict, env_vars: dict, out_path: str) -> None:
    """Write config/.generated.env from spec global values and runtime env_vars.

    Fallback chain for namespace groups:
      K8S_INFRA_NS: env K8S_INFRA_NAMESPACE → secretsGroup → "infra"
      K8S_TOOLS_NS: env K8S_TOOLS_NAMESPACE → extraBaseResourcesGroup → "tools"
      K8S_AGENT_NS: env K8S_AGENT_NAMESPACE → "agent"
    """
    g = spec.get("global", {})
    sources = g.get("sources", {})
    registries = g.get("registries", {})
    images = g.get("images", {})
    k8s = g.get("kubernetes", {})

    mq_lib_image = images.get("mq_lib_image", "")
    mq_lib_version = images.get("mq_lib_version", "")
    payments_mq_lib_image = f"{mq_lib_image}:{mq_lib_version}" if mq_lib_image and mq_lib_version else ""

    k8s_registry = sources.get("k8s_registry", "")
    k8s_overlay = "installer/k8s/overlays/dev" if k8s_registry else "installer/k8s/overlays/local"

    secrets_group = k8s.get("secretsGroup", "infra")
    extra_group = k8s.get("extraBaseResourcesGroup", "tools")

    infra_ns = env_vars.get("K8S_INFRA_NAMESPACE") or secrets_group or "infra"
    tools_ns = env_vars.get("K8S_TOOLS_NAMESPACE") or extra_group or "tools"
    agent_ns = env_vars.get("K8S_AGENT_NAMESPACE") or "agent"

    lines = [
        f"REDHAT_BASE_REGISTRY={sources.get('ubi_registry_mirror', '')}",
        f"INTERNAL_REGISTRY={sources.get('internal_registry', '')}",
        f"EXTERNAL_REGISTRY_MIRROR={sources.get('external_registry_mirror', '')}",
        f"PAYMENT_BASE_REGISTRY={sources.get('payment_registry_mirror', '')}",
        f"NPM_REGISTRY={registries.get('npm', '')}",
        f"PIP_REGISTRY={registries.get('pip', '')}",
        f"MQLIB_VERSION={mq_lib_version}",
        f"PAYMENTS_MQ_LIB_IMAGE={payments_mq_lib_image}",
        f"K8S_NAMESPACE={k8s.get('namespace', '')}",
        f"INGRESS_SUFFIX={k8s.get('ingress_suffix', '')}",
        f"APP_VERSION={g.get('appVersion', '')}",
        f"K8S_OVERLAY={k8s_overlay}",
        f"K8S_INFRA_NS={infra_ns}",
        f"K8S_AGENT_NS={agent_ns}",
        f"K8S_LOCAL_OVERLAY=installer/k8s/overlays/local",
    ]

    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")

    print(f"compile: wrote {out_path}")


# ─────────────────────────────────────────────────────────────────────────────
# Group namespace token resolver
# ─────────────────────────────────────────────────────────────────────────────

def _build_ns_map(spec: dict, env_vars: dict) -> dict[str, str]:
    """Build {token: resolved_namespace} map for {group_ns} token replacement in k8s_env."""
    g = spec.get("global", {})
    k8s = g.get("kubernetes", {})
    sources = g.get("sources", {})

    slot = env_vars.get("K8S_APP_SLOT", "1")
    secrets_group = k8s.get("secretsGroup", "infra")
    extra_group = k8s.get("extraBaseResourcesGroup", "tools")

    infra_ns = env_vars.get("K8S_INFRA_NAMESPACE") or secrets_group or "infra"
    tools_ns = env_vars.get("K8S_TOOLS_NAMESPACE") or extra_group or "tools"
    agent_ns = f"ns-{slot}"

    ns_map = {
        "agent_ns": agent_ns,
        "infra_ns": infra_ns,
        "tools_ns": tools_ns,
    }

    # Custom groups: any namespaceGroup not covered above
    for svc_name, svc in spec.get("services", {}).items():
        ng = (svc.get("targets", {}) or {}).get("helm", {}).get("namespaceGroup", "agent")
        if ng not in ns_map:
            env_key = f"K8S_{ng.upper()}_NAMESPACE"
            ns_map[f"{ng}_ns"] = env_vars.get(env_key, ng)

    return ns_map


def _resolve_ns_tokens(value: str, ns_map: dict[str, str]) -> str:
    """Replace {group_ns} tokens in a string with resolved namespace values.

    Used by the kustomize base generator (T4) to resolve {infra_ns}, {tools_ns},
    {agent_ns} tokens in k8s_env values at compile time.
    """
    for token, ns in ns_map.items():
        value = value.replace(f"{{{token}}}", ns)
    return value


# ─────────────────────────────────────────────────────────────────────────────
# Docker Compose generator
# ─────────────────────────────────────────────────────────────────────────────

# Build arg name → (global section, global key)
_BUILD_ARG_KEYS: dict[str, tuple[str, str]] = {
    "NPM_REGISTRY":             ("registries", "npm"),
    "PIP_REGISTRY":             ("registries", "pip"),
    "REDHAT_BASE_REGISTRY":     ("sources",    "ubi_registry_mirror"),
    "INTERNAL_REGISTRY":        ("sources",    "internal_registry"),
    "EXTERNAL_REGISTRY_MIRROR": ("sources",    "external_registry_mirror"),
    "PAYMENT_BASE_REGISTRY":    ("sources",    "payment_registry_mirror"),
    "PAYMENTS_MQ_LIB_IMAGE":    ("images",     "mq_lib_image"),
}


def _build_profile_index(spec: dict) -> tuple[dict[str, str], set[str]]:
    """Return (profile_of, always_on_profiles).

    profile_of: {service_name: profile_name} — inverted from global.profiles
    always_on_profiles: set of profile names whose profileActivation has no 'when'
    """
    g = spec.get("global", {})
    profiles: dict[str, list[str]] = g.get("profiles", {}) or {}
    activation: dict[str, dict] = g.get("profileActivation", {}) or {}

    profile_of: dict[str, str] = {}
    for profile_name, members in profiles.items():
        for svc in (members or []):
            profile_of[svc] = profile_name

    always_on: set[str] = set()
    for profile_name, cfg in activation.items():
        if not (cfg or {}).get("when"):
            always_on.add(profile_name)

    return profile_of, always_on


def _should_keep_dep(
    src_name: str,
    tgt_name: str,
    profile_of: dict[str, str],
    always_on: set[str],
) -> bool:
    """Return True if the waitsFor dependency should be kept in depends_on."""
    tgt_profile = profile_of.get(tgt_name)
    src_profile = profile_of.get(src_name)

    if tgt_profile is None:                           # Case 1: target has no profile (always present)
        return True
    if tgt_profile in always_on:                      # Case 2: target's profile is always-on
        return True
    if src_profile and src_profile == tgt_profile:   # Case 3: same conditional profile
        return True
    return False                                      # Drop: cross-profile or profile-free source → conditional target


def _build_args_for(spec: dict) -> dict[str, str]:
    """Build the auto-injected build args dict from non-empty global values."""
    g = spec.get("global", {})
    sources = g.get("sources", {})
    registries = g.get("registries", {})
    images = g.get("images", {})

    # Compute the combined MQ lib image string (mirrors _write_generated_env logic)
    mq_image = images.get("mq_lib_image", "")
    mq_version = images.get("mq_lib_version", "")
    payments_mq = f"{mq_image}:{mq_version}" if mq_image and mq_version else ""

    candidates = {
        "NPM_REGISTRY":             registries.get("npm", ""),
        "PIP_REGISTRY":             registries.get("pip", ""),
        "REDHAT_BASE_REGISTRY":     sources.get("ubi_registry_mirror", ""),
        "INTERNAL_REGISTRY":        sources.get("internal_registry", ""),
        "EXTERNAL_REGISTRY_MIRROR": sources.get("external_registry_mirror", ""),
        "PAYMENT_BASE_REGISTRY":    sources.get("payment_registry_mirror", ""),
        "PAYMENTS_MQ_LIB_IMAGE":    payments_mq,
    }
    return {k: v for k, v in candidates.items() if v}


def _build_healthcheck(hc_spec: dict) -> dict:
    """Convert spec healthcheck dict to Compose format (rename startPeriod → start_period)."""
    hc = dict(hc_spec)  # shallow copy — don't mutate spec
    if "startPeriod" in hc:
        hc["start_period"] = hc.pop("startPeriod")
    return hc


def _expand_build_env(val: str, env_vars: dict) -> str:
    """Expand ${VAR:-default} and ${VAR} in build string using env_vars."""
    if not val:
        return val

    def _replace(m: re.Match) -> str:
        var_expr = m.group(1)
        if ":-" in var_expr:
            var_name, default_val = var_expr.split(":-", 1)
            v = env_vars.get(var_name)
            return v if v is not None and v != "" else default_val
        return env_vars.get(var_expr, m.group(0))

    return re.sub(r"\$\{([^}]+)\}", _replace, val)


def _generate_compose(
    spec: dict, sources: dict, env_vars: dict, out_path: str, no_profiles: bool = False
) -> None:
    """Generate docker-compose.yml from the spec.

    Key rules:
    - envFile → env_file: list (NOT inlined into environment:)
    - env: block → environment: map
    - startPeriod renamed to start_period in healthcheck
    - waitsFor filtered by three-case profile rule
    - restart: no emitted as 'no' (quoted) via post-processing
    - All services get container_name and networks when compose.network is set
    """
    g = spec.get("global", {})
    compose_cfg = spec.get("compose", {}) or {}
    services_spec: dict[str, dict] = spec.get("services", {}) or {}

    network_name: str | None = compose_cfg.get("network")
    global_volumes = compose_cfg.get("volumes")

    profile_of, always_on = _build_profile_index(spec)
    build_args = _build_args_for(spec)

    compose_services: dict[str, dict] = {}

    for svc_name, svc in services_spec.items():
        svc = svc or {}
        targets = svc.get("targets", {}) or {}
        dev = targets.get("dev", {}) or {}
        entry: dict[str, Any] = {}

        # image — expand ${var}; targets.dev.image overrides in compose only
        image = _expand_image(svc.get("image"), sources)
        if dev.get("image"):
            image = _expand_image(dev["image"], sources)
        if image:
            entry["image"] = image

        # container_name — always set
        entry["container_name"] = svc_name

        # ports
        if svc.get("ports"):
            entry["ports"] = list(svc["ports"])

        # env_file — list, NOT inlined. Docker reads at compose up.
        if svc.get("envFile"):
            entry["env_file"] = list(svc["envFile"])

        # environment — from env: block only; never from envFile
        env_block = svc.get("env") or {}
        if env_block:
            entry["environment"] = dict(env_block)

        # volumes
        if svc.get("volumes"):
            entry["volumes"] = list(svc["volumes"])

        # healthcheck
        if svc.get("healthcheck"):
            entry["healthcheck"] = _build_healthcheck(svc["healthcheck"])

        # depends_on — filter waitsFor by three-case rule
        waits = svc.get("waitsFor") or {}
        if waits:
            kept = {
                tgt: cond
                for tgt, cond in waits.items()
                if _should_keep_dep(svc_name, tgt, profile_of, always_on)
            }
            if kept:
                entry["depends_on"] = {
                    tgt: {"condition": cond if cond.startswith("service_") else f"service_{cond}"}
                    for tgt, cond in kept.items()
                }

        # command
        if svc.get("command"):
            entry["command"] = list(svc["command"])

        # restart — 'no' requires special quoting (post-processed below)
        if "restart" in svc:
            entry["restart"] = svc["restart"]

        # platform
        if svc.get("platform"):
            entry["platform"] = svc["platform"]

        # ulimits
        if svc.get("ulimits"):
            entry["ulimits"] = dict(svc["ulimits"])

        # build — from targets.dev.build; inject non-empty build args
        if dev.get("build"):
            build_block = dict(dev["build"])
            if "dockerfile" in build_block:
                build_block["dockerfile"] = _expand_build_env(build_block["dockerfile"], env_vars)
            if "context" in build_block:
                build_block["context"] = _expand_build_env(build_block["context"], env_vars)
            if build_args:
                build_block.setdefault("args", {}).update(build_args)
            entry["build"] = build_block

        # profiles — from global.profiles membership (omitted if no_profiles=True)
        if not no_profiles:
            profile = profile_of.get(svc_name)
            if profile:
                entry["profiles"] = [profile]

        # networks — all services join compose.network when set
        if network_name:
            entry["networks"] = [network_name]

        compose_services[svc_name] = entry

    # ── Top-level compose document ──────────────────────────────────────────
    compose_doc: dict[str, Any] = {"services": compose_services}

    if network_name:
        compose_doc["networks"] = {network_name: None}

    if global_volumes:
        compose_doc["volumes"] = global_volumes

    # ── Serialise ────────────────────────────────────────────────────────────
    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    raw = yaml.dump(compose_doc, sort_keys=False, allow_unicode=True, default_flow_style=False)

    # Post-process: quote 'no' for restart to prevent YAML boolean misparse
    raw = raw.replace("restart: no\n", "restart: 'no'\n")

    with open(out_path, "w", encoding="utf-8") as f:
        f.write(raw)

    print(f"compile: wrote {out_path}")


def _normalize_memory(mem: Any) -> str:
    """Normalize memory strings like 512m -> 512Mi, 1g -> 1Gi for Kubernetes manifests."""
    if not mem:
        return ""
    m = str(mem).strip()
    if m.endswith("m"):
        return m[:-1] + "Mi"
    if m.endswith("g"):
        return m[:-1] + "Gi"
    return m


def _parse_service_ports(ports: list[Any]) -> list[dict]:
    """Parse port specs (e.g. ['8080:8080', 5432]) into Kubernetes Service port mappings."""
    s_ports = []
    for p in ports:
        parts = str(p).split(":")
        if len(parts) == 2:
            p_ext = int(parts[0])
            p_int = int(parts[1])
        else:
            p_ext = p_int = int(parts[0])
        s_ports.append({
            "name": f"port-{p_int}",
            "port": p_ext,
            "targetPort": p_int,
        })
    return s_ports


def _build_k8s_volumes(svc: dict, helm: dict) -> tuple[list[dict], list[dict]]:
    """Build Kubernetes pod volumes and container volumeMounts from service and helm targets."""
    volumes: list[dict] = []
    volume_mounts: list[dict] = []
    seen_names: set[str] = set()

    # 1. k8sVolumes (explicit ConfigMap or Secret mounts)
    for kv in (helm.get("k8sVolumes") or []):
        vname = kv.get("name")
        if not vname or vname in seen_names:
            continue
        seen_names.add(vname)
        v_entry: dict[str, Any] = {"name": vname}
        if "secret" in kv:
            v_entry["secret"] = kv["secret"]
        elif "configMap" in kv:
            v_entry["configMap"] = kv["configMap"]
        else:
            v_entry["emptyDir"] = {}
        volumes.append(v_entry)
        volume_mounts.append({
            "name": vname,
            "mountPath": kv.get("mountPath"),
            "readOnly": bool(kv.get("readOnly", False)),
        })

    # 2. hostPathVolumes
    for idx, hv in enumerate(helm.get("hostPathVolumes") or []):
        hp = hv.get("hostPath", "")
        clean_name = re.sub(r"[^a-zA-Z0-9-]", "-", hp).strip("-") or f"hostpath-{idx}"
        vname = f"hp-{clean_name}"
        if vname in seen_names:
            vname = f"{vname}-{idx}"
        seen_names.add(vname)
        volumes.append({"name": vname, "hostPath": {"path": hp}})
        volume_mounts.append({
            "name": vname,
            "mountPath": hv.get("mountPath"),
            "readOnly": bool(hv.get("readOnly", False)),
        })

    # 3. svc.volumes (if not already handled by hostPathVolumes)
    has_hpv = bool(helm.get("hostPathVolumes"))
    for idx, vol_str in enumerate(svc.get("volumes") or []):
        if has_hpv:
            continue
        parts = str(vol_str).split(":")
        source = parts[0]
        mount_path = parts[1] if len(parts) > 1 else source
        ro = len(parts) > 2 and parts[2] == "ro"

        if source.startswith(".") or "/" in source:
            vname = re.sub(r"[^a-zA-Z0-9-]", "-", os.path.basename(source)).strip("-") or f"vol-{idx}"
            vname = f"vol-{vname}"
            if vname in seen_names:
                vname = f"{vname}-{idx}"
            seen_names.add(vname)
            volumes.append({"name": vname, "hostPath": {"path": source}})
        else:
            vname = source
            if vname in seen_names:
                vname = f"{vname}-{idx}"
            seen_names.add(vname)
            volumes.append({"name": vname, "emptyDir": {}})

        volume_mounts.append({
            "name": vname,
            "mountPath": mount_path,
            "readOnly": ro,
        })

    return volumes, volume_mounts


def _generate_service_k8s_docs(
    svc_name: str,
    svc: dict,
    helm: dict,
    sources: dict,
    env_vars: dict,
    ns_map: dict,
    all_secrets: set[str],
) -> list[dict]:
    """Generate ordered Kubernetes documents for a single service.

    Order: ConfigMap (<svc>-env) -> Workload (Deployment/Job/CronJob) -> Service -> Ingress.
    """
    docs: list[dict] = []

    has_env = bool(svc.get("env"))
    has_env_file = bool(svc.get("envFile"))
    has_cm = has_env or has_env_file or bool(helm.get("k8s_env"))

    # 1. ConfigMap (<svc>-env)
    if has_cm:
        cm_data: dict[str, str] = {}
        merged_env = dict(svc.get("env") or {})
        merged_env.update(helm.get("k8s_env") or {})

        for k, v in merged_env.items():
            if _looks_like_secret(k):
                continue  # redacted into secrets.yaml
            resolved_val = _resolve_ns_tokens(str(v), ns_map)
            cm_data[k] = resolved_val

        docs.append({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {"name": f"{svc_name}-env"},
            "data": cm_data,
        })

    # 2. Workload (Deployment / Job / CronJob)
    annotations = dict(helm.get("annotations") or {})
    is_hook = bool(annotations.get("helm.sh/hook"))
    is_oneshot = (svc.get("restart") in ("no", "'no'", False)) and not is_hook

    cron_schedule = helm.get("schedule")
    if not cron_schedule and helm.get("scheduleFromEnv"):
        cron_schedule = env_vars.get(helm["scheduleFromEnv"], "0 0 * * *")
    is_cron = bool(cron_schedule) and not is_hook

    volumes, volume_mounts = _build_k8s_volumes(svc, helm)

    image = _expand_image(svc.get("image"), sources)
    container: dict[str, Any] = {
        "name": svc_name,
        "image": image,
    }

    kargs = helm.get("k8s_args") or helm.get("kargs") or helm.get("kustomize_args") or svc.get("command")
    if kargs:
        container["command"] = list(kargs)

    if svc.get("ports") and not is_hook:
        container["ports"] = [
            {"containerPort": int(str(p).split(":")[-1]), "name": f"port-{str(p).split(':')[-1]}"}
            for p in svc["ports"]
        ]

    env_from: list[dict] = []
    if has_cm:
        env_from.append({"configMapRef": {"name": f"{svc_name}-env"}})
    if has_env_file or any(_looks_like_secret(k) for k in (svc.get("env") or {})):
        env_from.append({"secretRef": {"name": "app-secrets"}})
    if env_from:
        container["envFrom"] = env_from

    if svc.get("memory"):
        norm_m = _normalize_memory(svc["memory"])
        container["resources"] = {
            "limits": {"memory": norm_m},
            "requests": {"memory": norm_m},
        }

    if volume_mounts:
        container["volumeMounts"] = volume_mounts

    c_sec: dict[str, Any] = {}
    if "allowPrivilegeEscalation" in helm:
        c_sec["allowPrivilegeEscalation"] = bool(helm["allowPrivilegeEscalation"])
    if "readOnlyRootFilesystem" in helm:
        c_sec["readOnlyRootFilesystem"] = bool(helm["readOnlyRootFilesystem"])
    if helm.get("addCapabilities"):
        c_sec["capabilities"] = {"add": list(helm["addCapabilities"])}
    if c_sec:
        container["securityContext"] = c_sec

    pod_spec: dict[str, Any] = {"containers": [container]}
    if volumes:
        pod_spec["volumes"] = volumes

    p_sec: dict[str, Any] = {}
    for fld in ("runAsUser", "runAsGroup", "fsGroup", "runAsNonRoot"):
        if fld in helm:
            p_sec[fld] = helm[fld]
    if "seccompProfileType" in helm:
        p_sec["seccompProfile"] = {"type": helm["seccompProfileType"]}
    if p_sec:
        pod_spec["securityContext"] = p_sec

    if helm.get("initContainers"):
        inits = []
        for ic in helm["initContainers"]:
            ic_c = dict(ic)
            if ic_c.get("image"):
                ic_c["image"] = _expand_image(ic_c["image"], sources)
            inits.append(ic_c)
        pod_spec["initContainers"] = inits

    if is_hook:
        pod_spec["restartPolicy"] = "Never"
        workload = {
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": {
                "name": svc_name,
                "annotations": annotations,
            },
            "spec": {
                "template": {
                    "metadata": {"labels": {"app.kubernetes.io/name": svc_name}},
                    "spec": pod_spec,
                }
            },
        }
    elif is_cron:
        pod_spec["restartPolicy"] = "OnFailure"
        workload = {
            "apiVersion": "batch/v1",
            "kind": "CronJob",
            "metadata": {"name": svc_name},
            "spec": {
                "schedule": cron_schedule,
                "concurrencyPolicy": "Forbid",
                "jobTemplate": {
                    "spec": {
                        "template": {
                            "metadata": {"labels": {"app.kubernetes.io/name": svc_name}},
                            "spec": pod_spec,
                        }
                    }
                },
            },
        }
    elif is_oneshot:
        pod_spec["restartPolicy"] = "OnFailure"
        workload = {
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": {"name": svc_name},
            "spec": {
                "template": {
                    "metadata": {"labels": {"app.kubernetes.io/name": svc_name}},
                    "spec": pod_spec,
                }
            },
        }
    else:
        workload = {
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": svc_name,
                "labels": {"app.kubernetes.io/name": svc_name},
            },
            "spec": {
                "replicas": helm.get("replicas", 1),
                "selector": {"matchLabels": {"app.kubernetes.io/name": svc_name}},
                "template": {
                    "metadata": {"labels": {"app.kubernetes.io/name": svc_name}},
                    "spec": pod_spec,
                },
            },
        }
    docs.append(workload)

    # 3. Service (only when service has ports and is not a hook)
    if svc.get("ports") and not is_hook:
        s_ports = _parse_service_ports(svc["ports"])
        docs.append({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {"name": svc_name},
            "spec": {
                "selector": {"app.kubernetes.io/name": svc_name},
                "ports": s_ports,
            },
        })

    # 4. Ingress (only when targets.helm.ingress.hosts set and not a hook)
    ingress_cfg = helm.get("ingress") or {}
    if ingress_cfg.get("hosts") and not is_hook:
        svc_port = 80
        if svc.get("ports"):
            svc_port = int(str(svc["ports"][0]).split(":")[0])
        rules = []
        for h in ingress_cfg["hosts"]:
            rules.append({
                "host": _resolve_ns_tokens(str(h), ns_map),
                "http": {
                    "paths": [{
                        "path": "/",
                        "pathType": "Prefix",
                        "backend": {
                            "service": {
                                "name": svc_name,
                                "port": {"number": svc_port},
                            }
                        },
                    }]
                },
            })
        docs.append({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "Ingress",
            "metadata": {"name": svc_name},
            "spec": {
                "ingressClassName": "nginx",
                "rules": rules,
            },
        })

    return docs


def _generate_kustomize_base(
    spec: dict,
    sources: dict,
    env_vars: dict,
    ns_map: dict,
    base_dir: str,
) -> None:
    """Generate k8s/base/ directory containing manifests per namespace group."""
    g = spec.get("global", {})
    k8s_global = g.get("kubernetes", {}) or {}
    secrets_group = k8s_global.get("secretsGroup", "infra")
    extra_group = k8s_global.get("extraBaseResourcesGroup", "tools")
    extra_resources = spec.get("extraBaseResources") or g.get("extraBaseResources") or []

    # 1. Collect secrets across all enabled services
    all_secrets: set[str] = set()
    services_spec: dict[str, dict] = spec.get("services", {}) or {}

    for svc_name, svc in services_spec.items():
        svc = svc or {}
        helm = (svc.get("targets", {}) or {}).get("helm", {}) or {}
        if helm.get("enabled") is False:
            continue
        ef_env = helm.get("enabledFromEnv")
        if ef_env and str(env_vars.get(ef_env, "")).lower() == "false":
            continue

        for k in (svc.get("env") or {}).keys():
            if _looks_like_secret(k):
                all_secrets.add(k)
        for ef in (svc.get("envFile") or []):
            loaded = _load_env_file(ef, redact=False)
            for k in loaded.keys():
                if _looks_like_secret(k):
                    all_secrets.add(k)

    # 2. Group services and generate documents
    groups: dict[str, list[tuple[str, list[dict]]]] = {}
    for svc_name, svc in services_spec.items():
        svc = svc or {}
        helm = (svc.get("targets", {}) or {}).get("helm", {}) or {}
        if helm.get("enabled") is False:
            continue
        ef_env = helm.get("enabledFromEnv")
        if ef_env and str(env_vars.get(ef_env, "")).lower() == "false":
            continue

        grp = helm.get("namespaceGroup") or "agent"
        docs = _generate_service_k8s_docs(
            svc_name, svc, helm, sources, env_vars, ns_map, all_secrets
        )
        groups.setdefault(grp, []).append((svc_name, docs))

    if secrets_group not in groups:
        groups[secrets_group] = []
    if extra_group not in groups and extra_resources:
        groups[extra_group] = []

    # 3. Write manifests to base/<group>/
    os.makedirs(base_dir, exist_ok=True)
    active_groups: list[str] = []

    for grp in sorted(groups.keys()):
        grp_dir = os.path.join(base_dir, grp)
        os.makedirs(grp_dir, exist_ok=True)
        grp_resources: list[str] = []

        # Write per-service multi-doc YAML
        for svc_name, docs in sorted(groups[grp], key=lambda x: x[0]):
            fname = f"{svc_name}.yaml"
            fpath = os.path.join(grp_dir, fname)
            with open(fpath, "w", encoding="utf-8") as f:
                f.write(yaml.dump_all(docs, sort_keys=False, default_flow_style=False))
            grp_resources.append(fname)

        # Write secrets.yaml in secretsGroup
        if grp == secrets_group and all_secrets:
            sec_doc = {
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {"name": "app-secrets"},
                "type": "Opaque",
                "stringData": {k: "placeholder" for k in sorted(all_secrets)},
            }
            sec_fname = "secrets.yaml"
            sec_fpath = os.path.join(grp_dir, sec_fname)
            with open(sec_fpath, "w", encoding="utf-8") as f:
                f.write(yaml.dump(sec_doc, sort_keys=False, default_flow_style=False))
            grp_resources.append(sec_fname)

        # Copy extra base resources into extraBaseResourcesGroup
        if grp == extra_group:
            for extra_res in extra_resources:
                target_path = os.path.join(grp_dir, extra_res)
                src_template = os.path.join("installer", "k8s-templates", extra_res)
                if not os.path.exists(src_template):
                    bundled_tpl = os.path.join(os.path.dirname(__file__), "..", "k8s-templates", extra_res)
                    if os.path.exists(bundled_tpl):
                        src_template = bundled_tpl
                if os.path.exists(src_template):
                    shutil.copyfile(src_template, target_path)
                elif not os.path.exists(target_path):
                    with open(target_path, "w", encoding="utf-8") as f:
                        f.write(f"# Placeholder for {extra_res}\n")
                if extra_res not in grp_resources:
                    grp_resources.append(extra_res)

        if grp_resources:
            # Write group kustomization.yaml
            kust_doc = {"resources": sorted(grp_resources)}
            kust_fpath = os.path.join(grp_dir, "kustomization.yaml")
            with open(kust_fpath, "w", encoding="utf-8") as f:
                f.write(yaml.dump(kust_doc, sort_keys=False, default_flow_style=False))
            active_groups.append(grp)

    # 4. Write root base/kustomization.yaml
    root_kust = {"resources": active_groups}
    with open(os.path.join(base_dir, "kustomization.yaml"), "w", encoding="utf-8") as f:
        f.write(yaml.dump(root_kust, sort_keys=False, default_flow_style=False))

    print(f"compile: wrote {base_dir}")


def _generate_kustomize_overlays(
    spec: dict,
    sources: dict,
    env_vars: dict,
    ns_map: dict,
    overlays_dir: str,
) -> None:
    """Generate installer/k8s/overlays/ directory (local + dev)."""
    g = spec.get("global", {})
    k8s_global = g.get("kubernetes", {}) or {}
    app_source = sources.get("app_source", "demo")
    ingress_suffix = k8s_global.get("ingress_suffix", "app.localapp")
    slot = env_vars.get("K8S_APP_SLOT", "1")

    # Determine whether pod security is enabled
    pod_sec_str = str(env_vars.get("K8S_POD_SECURITY_ENABLED", "")).lower()
    if pod_sec_str in ("false", "0", "no"):
        pod_sec_enabled = False
    elif pod_sec_str in ("true", "1", "yes"):
        pod_sec_enabled = True
    else:
        pod_sec_enabled = bool(k8s_global.get("podSecurityEnabled", False))

    services_spec: dict[str, dict] = spec.get("services", {}) or {}
    groups: dict[str, list[tuple[str, dict]]] = {}

    for svc_name, svc in services_spec.items():
        svc = svc or {}
        helm = (svc.get("targets", {}) or {}).get("helm", {}) or {}
        if helm.get("enabled") is False:
            continue
        ef_env = helm.get("enabledFromEnv")
        if ef_env and str(env_vars.get(ef_env, "")).lower() == "false":
            continue
        grp = helm.get("namespaceGroup") or "agent"
        groups.setdefault(grp, []).append((svc_name, svc))

    secrets_group = k8s_global.get("secretsGroup", "infra")
    extra_group = k8s_global.get("extraBaseResourcesGroup", "tools")
    if secrets_group not in groups:
        groups[secrets_group] = []
    if extra_group not in groups:
        groups[extra_group] = []

    # ─────────────────────────────────────────────────────────────────────────
    # 1. overlays/local/
    # ─────────────────────────────────────────────────────────────────────────
    local_dir = os.path.join(overlays_dir, "local")
    os.makedirs(local_dir, exist_ok=True)

    for grp in sorted(groups.keys()):
        if grp == "agent":
            subdir_name = f"ns-{slot}"
            target_ns = f"ns-{slot}"
        elif grp == secrets_group:
            target_ns = ns_map.get("infra_ns", grp)
            subdir_name = target_ns
        elif grp == extra_group:
            target_ns = ns_map.get("tools_ns", grp)
            subdir_name = target_ns
        else:
            target_ns = ns_map.get(f"{grp}_ns", grp)
            subdir_name = target_ns

        grp_overlay_dir = os.path.join(local_dir, subdir_name)
        os.makedirs(grp_overlay_dir, exist_ok=True)

        kust: dict[str, Any] = {
            "namespace": target_ns,
            "resources": [f"../../../base/{grp}"],
        }
        patches: list[dict] = []

        # Ingress patch
        ingress_patches: list[dict] = []
        for svc_name, svc in sorted(groups[grp], key=lambda x: x[0]):
            helm = (svc.get("targets", {}) or {}).get("helm", {}) or {}
            ing = helm.get("ingress") or {}
            prefix = ing.get("localSlotPrefix")
            if prefix:
                host_str = f"{prefix}{slot}.{ingress_suffix}"
                svc_port = 80
                if svc.get("ports"):
                    svc_port = int(str(svc["ports"][0]).split(":")[0])
                ingress_patches.append({
                    "apiVersion": "networking.k8s.io/v1",
                    "kind": "Ingress",
                    "metadata": {"name": svc_name},
                    "spec": {
                        "rules": [{
                            "host": host_str,
                            "http": {
                                "paths": [{
                                    "path": "/",
                                    "pathType": "Prefix",
                                    "backend": {
                                        "service": {
                                            "name": svc_name,
                                            "port": {"number": svc_port},
                                        }
                                    },
                                }]
                            },
                        }]
                    },
                })

        if ingress_patches:
            ing_fpath = os.path.join(grp_overlay_dir, "ingress-patch.yaml")
            with open(ing_fpath, "w", encoding="utf-8") as f:
                f.write(yaml.dump_all(ingress_patches, sort_keys=False, default_flow_style=False))
            patches.append({"path": "ingress-patch.yaml"})

        # Local security patch
        if not pod_sec_enabled:
            sec_patches: list[dict] = []
            for svc_name, svc in sorted(groups[grp], key=lambda x: x[0]):
                helm = (svc.get("targets", {}) or {}).get("helm", {}) or {}
                should_patch = (grp == secrets_group) or bool(helm.get("localSecurityPatch"))
                if not should_patch:
                    continue

                annotations = dict(helm.get("annotations") or {})
                is_hook = bool(annotations.get("helm.sh/hook"))
                is_oneshot = (svc.get("restart") in ("no", "'no'", False)) and not is_hook
                cron_schedule = helm.get("schedule")
                if not cron_schedule and helm.get("scheduleFromEnv"):
                    cron_schedule = env_vars.get(helm["scheduleFromEnv"], "0 0 * * *")
                is_cron = bool(cron_schedule) and not is_hook

                if is_hook or is_oneshot:
                    workload_kind = "Job"
                    api_version = "batch/v1"
                elif is_cron:
                    workload_kind = "CronJob"
                    api_version = "batch/v1"
                else:
                    workload_kind = "Deployment"
                    api_version = "apps/v1"

                p_sec = helm.get("localSecurityContext")
                if p_sec is None:
                    p_sec = {
                        "runAsNonRoot": False,
                        "runAsUser": 0,
                        "runAsGroup": 0,
                        "fsGroup": 0,
                    }
                else:
                    p_sec = dict(p_sec)

                c_sec = helm.get("localContainerSecurityContext")
                if c_sec is None:
                    c_sec = {
                        "allowPrivilegeEscalation": True,
                        "readOnlyRootFilesystem": False,
                    }
                else:
                    c_sec = dict(c_sec)

                container_patch = {
                    "name": svc_name,
                    "securityContext": c_sec,
                }

                if workload_kind == "CronJob":
                    patch_doc = {
                        "apiVersion": api_version,
                        "kind": workload_kind,
                        "metadata": {"name": svc_name},
                        "spec": {
                            "jobTemplate": {
                                "spec": {
                                    "template": {
                                        "spec": {
                                            "securityContext": p_sec,
                                            "containers": [container_patch],
                                        }
                                    }
                                }
                            }
                        },
                    }
                else:
                    patch_doc = {
                        "apiVersion": api_version,
                        "kind": workload_kind,
                        "metadata": {"name": svc_name},
                        "spec": {
                            "template": {
                                "spec": {
                                    "securityContext": p_sec,
                                    "containers": [container_patch],
                                }
                            }
                        },
                    }
                sec_patches.append(patch_doc)

            if sec_patches:
                sec_fpath = os.path.join(grp_overlay_dir, "local-security-patch.yaml")
                with open(sec_fpath, "w", encoding="utf-8") as f:
                    f.write(yaml.dump_all(sec_patches, sort_keys=False, default_flow_style=False))
                patches.append({"path": "local-security-patch.yaml"})

        if patches:
            kust["patches"] = patches

        # Track images built locally
        if grp == "agent":
            agent_images = []
            for svc_name, svc in sorted(groups[grp], key=lambda x: x[0]):
                dev = (svc.get("targets", {}) or {}).get("dev", {}) or {}
                if dev.get("build") or (svc.get("image") or "").startswith(f"${{app_source}}"):
                    orig_name = f"{app_source}/{svc_name}"
                    agent_images.append({
                        "name": orig_name,
                        "newName": orig_name,
                    })
            if agent_images:
                kust["images"] = agent_images

        with open(os.path.join(grp_overlay_dir, "kustomization.yaml"), "w", encoding="utf-8") as f:
            f.write(yaml.dump(kust, sort_keys=False, default_flow_style=False))

    print(f"compile: wrote {local_dir}")

    # ─────────────────────────────────────────────────────────────────────────
    # 2. overlays/dev/
    # ─────────────────────────────────────────────────────────────────────────
    dev_dir = os.path.join(overlays_dir, "dev")
    os.makedirs(dev_dir, exist_ok=True)

    k8s_ns = k8s_global.get("namespace", "default")
    k8s_reg = sources.get("k8s_registry", "")
    k8s_prefix = sources.get("k8s_image_prefix", "")
    tag_placeholder = sources.get("k8s_tag_placeholder", "<REPLACEMENT_APP_VERSION>")

    dev_kust: dict[str, Any] = {
        "namespace": k8s_ns,
        "resources": ["../../base"],
        "patches": [{"path": "secrets-patch.yaml"}],
    }

    dev_images = []
    for svc_name, svc in sorted(services_spec.items(), key=lambda x: x[0]):
        svc = svc or {}
        helm = (svc.get("targets", {}) or {}).get("helm", {}) or {}
        if helm.get("enabled") is False:
            continue
        ef_env = helm.get("enabledFromEnv")
        if ef_env and str(env_vars.get(ef_env, "")).lower() == "false":
            continue

        dev = (svc.get("targets", {}) or {}).get("dev", {}) or {}
        if dev.get("build") or (svc.get("image") or "").startswith(f"${{app_source}}"):
            orig_name = f"{app_source}/{svc_name}"
            new_name = f"{k8s_reg}/{k8s_prefix}{svc_name}" if k8s_reg else orig_name
            dev_images.append({
                "name": orig_name,
                "newName": new_name,
                "newTag": tag_placeholder,
            })

    if dev_images:
        dev_kust["images"] = dev_images

    with open(os.path.join(dev_dir, "kustomization.yaml"), "w", encoding="utf-8") as f:
        f.write(yaml.dump(dev_kust, sort_keys=False, default_flow_style=False))

    print(f"compile: wrote {dev_dir}")


def _generate_kustomize(spec: dict, sources: dict, env_vars: dict, ns_map: dict, out_dir: str) -> None:
    """Generate installer/k8s/ Kustomize manifests (base + overlays)."""
    base_dir = os.path.join(out_dir, "base")
    overlays_dir = os.path.join(out_dir, "overlays")
    _generate_kustomize_base(spec, sources, env_vars, ns_map, base_dir)
    _generate_kustomize_overlays(spec, sources, env_vars, ns_map, overlays_dir)


def _normalize_volumes_for_values(svc: dict, helm: dict) -> list[dict]:
    """Normalize volumes into {mountPath, readOnly, hostPath?, type?, name?}."""
    res: list[dict] = []
    # 1. k8sVolumes
    for kv in (helm.get("k8sVolumes") or []):
        entry: dict[str, Any] = {
            "name": kv.get("name"),
            "mountPath": kv.get("mountPath"),
            "readOnly": bool(kv.get("readOnly", False)),
        }
        if "secret" in kv:
            entry["type"] = "secret"
        elif "configMap" in kv:
            entry["type"] = "configMap"
        else:
            entry["type"] = "volume"
        res.append(entry)

    # 2. hostPathVolumes
    for hv in (helm.get("hostPathVolumes") or []):
        res.append({
            "mountPath": hv.get("mountPath"),
            "hostPath": hv.get("hostPath"),
            "readOnly": bool(hv.get("readOnly", False)),
            "type": "hostPath",
        })

    # 3. svc.volumes (if not already handled by hostPathVolumes)
    if not helm.get("hostPathVolumes"):
        for vol_str in (svc.get("volumes") or []):
            parts = str(vol_str).split(":")
            source = parts[0]
            mpath = parts[1] if len(parts) > 1 else source
            ro = len(parts) > 2 and parts[2] == "ro"
            if source.startswith(".") or "/" in source:
                res.append({
                    "mountPath": mpath,
                    "hostPath": source,
                    "readOnly": ro,
                    "type": "hostPath",
                })
            else:
                res.append({
                    "name": source,
                    "mountPath": mpath,
                    "readOnly": ro,
                    "type": "named",
                })
    return res


def _generate_helm_values(spec: dict, sources: dict, env_vars: dict, out_path: str) -> None:
    """Generate installer/values.yaml.

    Key rules:
    - Secret keys stripped from env block
    - Memory normalised: 512m -> 512Mi, 1g -> 1Gi
    - ${var} in image expanded via sources
    - Volumes normalised to {mountPath, readOnly, hostPath?, type?}
    - Each service entry includes: image, replicas, memory, env (non-secret only), volumes, ports, healthcheck
    """
    g = spec.get("global", {})
    services_spec: dict[str, dict] = spec.get("services", {}) or {}

    services_values: dict[str, dict] = {}

    for svc_name, svc in services_spec.items():
        svc = svc or {}
        helm = (svc.get("targets", {}) or {}).get("helm", {}) or {}

        # 1. Image (${var} expanded via sources)
        image = _expand_image(svc.get("image"), sources)

        # 2. Replicas
        replicas = int(helm.get("replicas", 1))

        # 3. Memory normalised (512m -> 512Mi, 1g -> 1Gi)
        memory = _normalize_memory(svc.get("memory"))

        # 4. Env (secret keys stripped)
        env_block: dict[str, str] = {}
        for k, v in (svc.get("env") or {}).items():
            if not _looks_like_secret(k):
                env_block[k] = str(v)

        # 5. Volumes normalised
        volumes = _normalize_volumes_for_values(svc, helm)

        # 6. Ports
        ports: list[dict] = []
        if svc.get("ports"):
            for p in svc["ports"]:
                parts = str(p).split(":")
                p_int = int(parts[-1])
                p_ext = int(parts[0]) if len(parts) == 2 else p_int
                ports.append({"port": p_ext, "targetPort": p_int})

        # 7. Healthcheck
        healthcheck = None
        if svc.get("healthcheck"):
            healthcheck = _build_healthcheck(svc["healthcheck"])

        entry: dict[str, Any] = {
            "image": image,
            "replicas": replicas,
            "memory": memory,
            "env": env_block,
            "volumes": volumes,
            "ports": ports,
        }
        if healthcheck:
            entry["healthcheck"] = healthcheck

        services_values[svc_name] = entry

    values_doc: dict[str, Any] = {
        "global": dict(g),
        "services": services_values,
    }
    # Provide top-level keys for each service as well for template flexibility
    for s_name, s_val in services_values.items():
        values_doc[s_name] = s_val

    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write(yaml.dump(values_doc, sort_keys=False, default_flow_style=False))

    print(f"compile: wrote {out_path}")


# ─────────────────────────────────────────────────────────────────────────────
# CLI
# ─────────────────────────────────────────────────────────────────────────────

def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Ved — compile a YAML spec into compose, k8s, and helm outputs."
    )
    parser.add_argument("spec", help="Path to the spec YAML file")
    parser.add_argument("out_compose", help="Output path for docker-compose.yml")
    parser.add_argument("out_values", nargs="?", default=None,
                        help="Output path for installer/values.yaml (optional)")
    parser.add_argument("out_k8s", nargs="?", default=None,
                        help="Output directory for installer/k8s/ (optional)")
    parser.add_argument("--org", default="local",
                        help="Org profile name (looks up installer/orgs/{ORG}.yaml)")
    parser.add_argument("--no-profiles", action="store_true",
                        help="Omit profiles from docker-compose.yml for runtimes without profile support (e.g. pullrun)")
    parser.add_argument("--env", action="append", default=[],
                        metavar="K=V",
                        help="Extra env var to inject into the spec (can repeat)")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = _parse_args(argv)

    # 1. Load spec
    spec = _load_spec(args.spec)

    # 2. Merge org profile
    org_path = None
    if os.path.isfile(args.org):
        org_path = args.org
    else:
        candidates = [
            os.path.join("installer", "orgs", f"{args.org}.yaml"),
            os.path.join(os.path.dirname(__file__), "..", "orgs", f"{args.org}.yaml"),
        ]
        for c in candidates:
            if os.path.isfile(c):
                org_path = os.path.abspath(c)
                break

    if not org_path or not os.path.isfile(org_path):
        sys.exit(f"ERROR: org file not found for '{args.org}'. Searched ./installer/orgs/{args.org}.yaml and bundled orgs.")

    _merge_org_profile(spec, org_path)

    # 3. Apply extra --env K=V overrides
    extra_env: dict[str, str] = {}
    for kv in args.env:
        key, _, val = kv.partition("=")
        extra_env[key.strip()] = val  # split on first = only

    # 4. Load runtime env vars (for token resolution and generated.env)
    env_vars: dict[str, str] = {}
    for env_file in ["config/.generated.env", "config/.env", "config/.local.env"]:
        env_vars.update(_load_env_file(env_file))
    env_vars.update(extra_env)

    # 5. Extract sources after org merge
    g = spec.get("global", {})
    sources: dict[str, str] = g.get("sources", {})

    # 6. Build namespace map for {group_ns} token resolution
    ns_map = _build_ns_map(spec, env_vars)

    # 7. Write config/.generated.env
    _write_generated_env(spec, env_vars, "config/.generated.env")

    # 8. Generate compose
    try:
        _generate_compose(spec, sources, env_vars, args.out_compose, no_profiles=args.no_profiles)
    except NotImplementedError as e:
        print(f"INFO: {e}", file=sys.stderr)

    # 9. Generate kustomize (optional)
    if args.out_k8s:
        try:
            _generate_kustomize(spec, sources, env_vars, ns_map, args.out_k8s)
        except NotImplementedError as e:
            print(f"INFO: {e}", file=sys.stderr)

    # 10. Generate helm values (optional)
    if args.out_values:
        try:
            _generate_helm_values(spec, sources, env_vars, args.out_values)
        except NotImplementedError as e:
            print(f"INFO: {e}", file=sys.stderr)


if __name__ == "__main__":
    main()
