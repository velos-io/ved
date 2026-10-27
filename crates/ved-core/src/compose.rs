use crate::error::Result;
use crate::validated::{DependencyCondition, DependencySpec, ValidatedSpec, ValidatedWorkload};
use indexmap::IndexMap;
use serde::Serialize;
use serde_yaml::{Mapping, Value};

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use crate::model::{
    AppSpec, CommandSpec, HealthCheckSpec, JobSpec, MountSpec, PortSpec, ServiceSpec,
};

#[derive(Debug, Clone, Serialize)]
pub struct ComposeFile {
    pub name: String,
    pub services: IndexMap<String, ComposeService>,
    pub networks: IndexMap<String, Option<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<IndexMap<String, Option<Value>>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ComposeService {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_file: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<IndexMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<ComposeHealthcheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<IndexMap<String, ComposeDependency>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub networks: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profiles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<IndexMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploy: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shm_size: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ulimits: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComposeDependency {
    pub condition: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComposeHealthcheck {
    pub test: Vec<String>,
    pub interval: String,
    pub timeout: String,
    pub retries: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_period: Option<String>,
}

#[cfg(feature = "host")]
pub fn generate_compose(spec: &AppSpec) -> Result<String> {
    generate_compose_validated(&ValidatedSpec::try_from(spec)?)
}

#[cfg(feature = "host")]
pub fn generate_compose_validated(validated: &ValidatedSpec) -> Result<String> {
    generate_compose_validated_with_context(
        validated,
        &crate::workspace::WorkspaceContext::new("."),
    )
}

/// Compile using a resolved configuration snapshot, without ambient lookups.
pub fn generate_compose_validated_with_context(
    validated: &ValidatedSpec,
    context: &crate::workspace::WorkspaceContext,
) -> Result<String> {
    let prepared = validated.with_context(context);
    let validated = &prepared;
    let spec = validated.as_spec();
    let ingress_suffix = context
        .config()
        .resolve_ingress_suffix(spec.ingress.as_deref());

    let (active_workloads, healthy_workloads) = collect_active_workloads(spec);
    let mut services = IndexMap::new();

    let effective_env_files: Vec<String> = spec
        .env_files
        .iter()
        .map(|f| rebase_env_file(f, context.root(), context.root()))
        .collect();

    for (name, svc) in &spec.services {
        if svc.enabled.unwrap_or(true) {
            let service = build_compose_service_typed(
                name,
                svc,
                spec,
                &active_workloads,
                &healthy_workloads,
                None,
                validated
                    .workload(None, name)
                    .expect("validated service cache"),
                Some(&ingress_suffix),
                &effective_env_files,
                context.root(),
            );
            services.insert(name.clone(), service);
        }
    }

    for (name, job) in &spec.jobs {
        if job.enabled.unwrap_or(true) {
            let job_svc = build_compose_job_typed(
                name,
                job,
                spec,
                &active_workloads,
                &healthy_workloads,
                None,
                validated.workload(None, name).expect("validated job cache"),
                &effective_env_files,
            );
            services.insert(name.clone(), job_svc);
        }
    }

    for (prof_name, prof) in &spec.profiles {
        for (name, svc) in &prof.services {
            if svc.enabled.unwrap_or(true) {
                let service = build_compose_service_typed(
                    name,
                    svc,
                    spec,
                    &active_workloads,
                    &healthy_workloads,
                    Some(prof_name),
                    validated
                        .workload(Some(prof_name), name)
                        .expect("validated service cache"),
                    Some(&ingress_suffix),
                    &effective_env_files,
                    context.root(),
                );
                services.insert(name.clone(), service);
            }
        }
        for (name, job) in &prof.jobs {
            if job.enabled.unwrap_or(true) {
                let job_svc = build_compose_job_typed(
                    name,
                    job,
                    spec,
                    &active_workloads,
                    &healthy_workloads,
                    Some(prof_name),
                    validated
                        .workload(Some(prof_name), name)
                        .expect("validated job cache"),
                    &effective_env_files,
                );
                services.insert(name.clone(), job_svc);
            }
        }
    }

    let mut networks = IndexMap::new();
    if !spec.networks.is_empty() {
        for (net_name, net_val) in &spec.networks {
            let val = if net_val.is_null() {
                None
            } else {
                Some(json_to_yaml_value(net_val))
            };
            networks.insert(net_name.clone(), val);
        }
    } else {
        networks.insert(spec.network.clone(), None);
    }

    let volumes = build_compose_volumes_validated(validated).map(|vol_map| {
        vol_map
            .into_iter()
            .filter_map(|(k, _)| k.as_str().map(|s| (s.to_string(), None)))
            .collect::<IndexMap<String, Option<Value>>>()
    });

    let compose_file = ComposeFile {
        name: spec.app.clone(),
        services,
        networks,
        volumes,
    };

    let yaml_str = serde_yaml::to_string(&compose_file)?;
    Ok(yaml_str)
}

fn collect_active_workloads(spec: &AppSpec) -> (HashSet<String>, HashSet<String>) {
    let mut active = HashSet::new();
    let mut healthy = HashSet::new();

    for (name, svc) in &spec.services {
        if svc.enabled.unwrap_or(true) {
            active.insert(name.clone());
            if svc.has_health() {
                healthy.insert(name.clone());
            }
        }
    }
    for (name, job) in &spec.jobs {
        if job.enabled.unwrap_or(true) {
            active.insert(name.clone());
        }
    }
    for (_prof_name, prof) in &spec.profiles {
        for (name, svc) in &prof.services {
            if svc.enabled.unwrap_or(true) {
                active.insert(name.clone());
                if svc.has_health() {
                    healthy.insert(name.clone());
                }
            }
        }
        for (name, job) in &prof.jobs {
            if job.enabled.unwrap_or(true) {
                active.insert(name.clone());
            }
        }
    }

    (active, healthy)
}

pub fn build_compose_service(
    name: &str,
    value: &ServiceSpec,
    spec: &AppSpec,
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
    profile_name: Option<&str>,
) -> Result<Mapping> {
    let mut snapshot = spec.clone();
    let entries = if let Some(profile) = profile_name {
        &mut snapshot
            .profiles
            .get_mut(profile)
            .ok_or_else(|| {
                crate::error::VedError::spec_parse("", format!("Unknown profile '{profile}'"))
            })?
            .services
    } else {
        &mut snapshot.services
    };
    entries.insert(name.to_string(), value.clone());
    let validated = ValidatedSpec::try_from(&snapshot)?;
    let workload = validated
        .workload(profile_name, name)
        .expect("validated workload cache");
    let empty_root = Path::new(".");
    let svc = build_compose_service_typed(
        name,
        value,
        validated.as_spec(),
        active_workloads,
        healthy_workloads,
        profile_name,
        workload,
        None,
        &validated.as_spec().env_files,
        empty_root,
    );
    match serde_yaml::to_value(&svc)? {
        Value::Mapping(m) => Ok(m),
        _ => Err(crate::error::VedError::spec_parse("", "Expected Mapping")),
    }
}

pub fn build_compose_job(
    name: &str,
    value: &JobSpec,
    spec: &AppSpec,
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
    profile_name: Option<&str>,
) -> Result<Mapping> {
    let mut snapshot = spec.clone();
    let entries = if let Some(profile) = profile_name {
        &mut snapshot
            .profiles
            .get_mut(profile)
            .ok_or_else(|| {
                crate::error::VedError::spec_parse("", format!("Unknown profile '{profile}'"))
            })?
            .jobs
    } else {
        &mut snapshot.jobs
    };
    entries.insert(name.to_string(), value.clone());
    let validated = ValidatedSpec::try_from(&snapshot)?;
    let workload = validated
        .workload(profile_name, name)
        .expect("validated workload cache");
    let job = build_compose_job_typed(
        name,
        value,
        validated.as_spec(),
        active_workloads,
        healthy_workloads,
        profile_name,
        workload,
        &validated.as_spec().env_files,
    );
    match serde_yaml::to_value(&job)? {
        Value::Mapping(m) => Ok(m),
        _ => Err(crate::error::VedError::spec_parse("", "Expected Mapping")),
    }
}

pub fn json_to_yaml_value(val: &serde_json::Value) -> serde_yaml::Value {
    match val {
        serde_json::Value::Null => serde_yaml::Value::Null,
        serde_json::Value::Bool(b) => serde_yaml::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_yaml::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_yaml::Value::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_yaml::Value::Number(f.into())
            } else {
                serde_yaml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => serde_yaml::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            serde_yaml::Value::Sequence(arr.iter().map(json_to_yaml_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut yaml_map = serde_yaml::Mapping::new();
            for (k, v) in map {
                yaml_map.insert(serde_yaml::Value::String(k.clone()), json_to_yaml_value(v));
            }
            serde_yaml::Value::Mapping(yaml_map)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_compose_service_typed(
    _name: &str,
    svc: &ServiceSpec,
    spec: &AppSpec,
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
    profile_name: Option<&str>,
    workload: &ValidatedWorkload,
    ingress_suffix: Option<&str>,
    effective_env_files: &[String],
    root: &Path,
) -> ComposeService {
    let entrypoint = workload.entrypoint().and_then(|cmd| {
        let v = format_command_value(cmd);
        (v != Value::Null).then_some(v)
    });
    let command = workload.command().and_then(|cmd| {
        let v = format_command_value(cmd);
        (v != Value::Null).then_some(v)
    });
    let ports = build_ports_value(workload.ports(), &svc.ports);
    let svc_env_files = if let Some(ref ef) = svc.env_file {
        vec![rebase_env_file(ef, root, root)]
    } else {
        effective_env_files.to_vec()
    };
    let env_file = build_env_file_value(&svc_env_files);
    let environment = build_env_map(&svc.env);
    let volumes = build_mounts_value(workload.mounts());
    let healthcheck = build_compose_healthcheck_typed(svc, workload.ports());
    let depends_on = build_depends_on(workload.dependencies(), active_workloads, healthy_workloads);
    let restart = svc.restart.clone();
    let build = svc
        .build
        .as_ref()
        .map(|b| Value::Mapping(resolve_build_mapping(b)));
    let networks = if let Some(ref svc_nets) = svc.networks {
        Some(json_to_yaml_value(svc_nets))
    } else {
        Some(Value::Sequence(vec![Value::String(spec.network.clone())]))
    };
    let profiles = build_profiles_value(&svc.group, profile_name, &svc.profiles);
    let labels = build_ingress_labels(svc.ingress.as_deref(), ingress_suffix);
    let deploy = svc.deploy.as_ref().map(json_to_yaml_value);
    let shm_size = svc.shm_size.as_ref().map(json_to_yaml_value);
    let ulimits = svc.ulimits.as_ref().map(json_to_yaml_value);

    ComposeService {
        image: svc.image.clone(),
        container_name: svc.container_name.clone(),
        ports,
        env_file,
        environment,
        volumes,
        healthcheck,
        depends_on,
        restart,
        build,
        entrypoint,
        command,
        networks,
        profiles,
        labels,
        deploy,
        shm_size,
        ulimits,
    }
}

fn build_ingress_labels(
    ingress: Option<&str>,
    ingress_suffix: Option<&str>,
) -> Option<IndexMap<String, String>> {
    let ingress = ingress?.trim();
    if ingress.is_empty() {
        return None;
    }
    let suffix = ingress_suffix?.trim();
    if suffix.is_empty() {
        return None;
    }
    let mut labels = IndexMap::new();
    labels.insert("ingress.host".to_string(), format!("{ingress}.{suffix}"));
    Some(labels)
}

#[allow(clippy::too_many_arguments)]
fn build_compose_job_typed(
    name: &str,
    job: &JobSpec,
    spec: &AppSpec,
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
    profile_name: Option<&str>,
    workload: &ValidatedWorkload,
    effective_env_files: &[String],
) -> ComposeService {
    let _ = name;
    let entrypoint = workload.entrypoint().and_then(|cmd| {
        let v = format_command_value(cmd);
        (v != Value::Null).then_some(v)
    });
    let command = workload.command().and_then(|cmd| {
        let v = format_command_value(cmd);
        (v != Value::Null).then_some(v)
    });
    let volumes = build_mounts_value(workload.mounts());
    let env_file = build_env_file_value(effective_env_files);
    let environment = build_env_map(&job.env);
    let depends_on = build_depends_on(workload.dependencies(), active_workloads, healthy_workloads);
    let restart = job.restart.clone().or_else(|| Some("no".to_string()));
    let networks = if let Some(ref job_nets) = job.networks {
        Some(json_to_yaml_value(job_nets))
    } else {
        Some(Value::Sequence(vec![Value::String(spec.network.clone())]))
    };
    let profiles = build_profiles_value(&job.group, profile_name, &job.profiles);
    let deploy = job.deploy.as_ref().map(json_to_yaml_value);
    let shm_size = job.shm_size.as_ref().map(json_to_yaml_value);
    let ulimits = job.ulimits.as_ref().map(json_to_yaml_value);

    ComposeService {
        image: Some(job.image.clone()),
        container_name: job.container_name.clone(),
        ports: None,
        env_file,
        environment,
        volumes,
        healthcheck: None,
        depends_on,
        restart,
        build: None,
        entrypoint,
        command,
        networks,
        profiles,
        labels: None,
        deploy,
        shm_size,
        ulimits,
    }
}

pub fn build_compose_healthcheck(svc: &ServiceSpec) -> Option<Mapping> {
    let hc = build_compose_healthcheck_typed(svc, &svc.typed_ports())?;
    match serde_yaml::to_value(&hc).ok()? {
        Value::Mapping(m) => Some(m),
        _ => None,
    }
}

fn build_compose_healthcheck_typed(
    svc: &ServiceSpec,
    ports: &[PortSpec],
) -> Option<ComposeHealthcheck> {
    let health_val = svc.health_value()?;
    let hc_spec = HealthCheckSpec::from_value(health_val);

    let mut test_seq = Vec::new();
    if let Some(test_val) = &hc_spec.test {
        if let Some(cmd) = test_val.as_str() {
            test_seq = build_health_test_seq(cmd, ports);
        } else if let Some(arr) = test_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    test_seq.push(s.to_string());
                }
            }
            if !test_seq.is_empty()
                && test_seq[0] != "CMD"
                && test_seq[0] != "CMD-SHELL"
                && test_seq[0] != "NONE"
            {
                test_seq.insert(0, "CMD".to_string());
            }
        }
    }

    if test_seq.is_empty() {
        return None;
    }

    Some(ComposeHealthcheck {
        test: test_seq,
        interval: hc_spec.interval,
        timeout: hc_spec.timeout,
        retries: hc_spec.retries,
        start_period: hc_spec.start_period,
    })
}

fn build_health_test_seq(cmd: &str, ports: &[PortSpec]) -> Vec<String> {
    let mut test_seq = Vec::new();
    if cmd.starts_with("http://") || cmd.starts_with("https://") || cmd.starts_with('/') {
        let mut url = if cmd.starts_with('/') {
            let cp = ports.first().map(|port| port.container).unwrap_or(80);
            format!("http://localhost:{cp}{cmd}")
        } else {
            cmd.to_string()
        };

        for port in ports {
            let (host_p, container_p) = (port.host, port.container);
            let host_prefix_localhost = format!("http://localhost:{host_p}");
            let host_prefix_ip = format!("http://127.0.0.1:{host_p}");
            let container_prefix = format!("http://localhost:{container_p}");
            if url.starts_with(&host_prefix_localhost) {
                url = url.replacen(&host_prefix_localhost, &container_prefix, 1);
                break;
            } else if url.starts_with(&host_prefix_ip) {
                url = url.replacen(&host_prefix_ip, &container_prefix, 1);
                break;
            }
        }

        test_seq.push("CMD-SHELL".to_string());
        test_seq.push(format!(
            "wget -qO- {url} > /dev/null 2>&1 || curl -fs {url} > /dev/null 2>&1 || exit 1"
        ));
    } else if cmd.starts_with("tcp:") {
        let mut port_str = cmd.trim_start_matches("tcp:").to_string();
        if let Ok(parsed_port) = port_str.parse::<u16>() {
            for port in ports {
                let (host_p, container_p) = (port.host, port.container);
                if host_p == parsed_port {
                    port_str = container_p.to_string();
                    break;
                }
            }
        }
        test_seq.push("CMD-SHELL".to_string());
        test_seq.push(format!(
            "nc -z localhost {port_str} || (echo > /dev/tcp/localhost/{port_str}) 2>/dev/null || exit 1"
        ));
    } else {
        test_seq.push("CMD-SHELL".to_string());
        test_seq.push(cmd.to_string());
    }
    test_seq
}

fn build_ports_value(ports: &[PortSpec], raw_ports: &[serde_json::Value]) -> Option<Vec<String>> {
    if !raw_ports.is_empty() {
        let mut result = Vec::new();
        for p in raw_ports {
            if let Some(s) = p.as_str() {
                result.push(s.to_string());
            } else if let Some(n) = p.as_u64() {
                result.push(format!("{n}:{n}"));
            } else if let Some(map) = p.as_object() {
                let target = map.get("target").and_then(|v| v.as_u64());
                let published = map.get("published").and_then(|v| v.as_u64());
                let host_ip = map.get("host_ip").and_then(|v| v.as_str());
                let proto = map.get("protocol").and_then(|v| v.as_str());
                let proto_suffix = match proto {
                    Some(pr) if pr != "tcp" => format!("/{pr}"),
                    _ => String::new(),
                };
                match (host_ip, published, target) {
                    (Some(ip), Some(pub_p), Some(tgt)) => {
                        result.push(format!("{ip}:{pub_p}:{tgt}{proto_suffix}"));
                    }
                    (None, Some(pub_p), Some(tgt)) => {
                        result.push(format!("{pub_p}:{tgt}{proto_suffix}"));
                    }
                    (Some(ip), None, Some(tgt)) => {
                        result.push(format!("{ip}:{tgt}:{tgt}{proto_suffix}"));
                    }
                    (None, None, Some(tgt)) => {
                        result.push(format!("{tgt}:{tgt}{proto_suffix}"));
                    }
                    _ => {}
                }
            }
        }
        if !result.is_empty() {
            return Some(result);
        }
    }
    (!ports.is_empty()).then(|| {
        ports
            .iter()
            .map(|port| format!("{}:{}", port.host, port.container))
            .collect()
    })
}

fn build_env_file_value(env_files: &[String]) -> Option<Vec<String>> {
    (!env_files.is_empty()).then(|| env_files.to_vec())
}

/// Computes a relative path from `from` directory to `to` path.
/// If `to` cannot be expressed relative to `from` (e.g. distinct filesystem roots),
/// returns `to` unchanged.
pub fn diff_paths(to: &Path, from: &Path) -> PathBuf {
    use std::path::Component;

    let to_components: Vec<Component> = to.components().collect();
    let from_components: Vec<Component> = from.components().collect();

    let mut common_len = 0;
    while common_len < to_components.len()
        && common_len < from_components.len()
        && to_components[common_len] == from_components[common_len]
    {
        common_len += 1;
    }

    if common_len == 0 && !to_components.is_empty() && !from_components.is_empty() {
        return to.to_path_buf();
    }

    let mut result = PathBuf::new();
    for c in &from_components[common_len..] {
        match c {
            Component::Normal(_) | Component::CurDir => result.push(".."),
            _ => return to.to_path_buf(),
        }
    }

    for c in &to_components[common_len..] {
        result.push(c);
    }

    if result.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        result
    }
}

pub fn rebase_env_file(raw: &str, root: &Path, compose_dir: &Path) -> String {
    let p = Path::new(raw);
    if p.is_absolute() {
        return raw.to_string();
    }
    let abs_target = root.join(p);
    let rel = diff_paths(&abs_target, compose_dir);
    rel.to_string_lossy().into_owned()
}

fn build_env_map(env: &IndexMap<String, serde_json::Value>) -> Option<IndexMap<String, String>> {
    if env.is_empty() {
        return None;
    }
    let mut env_map = IndexMap::new();
    for (k, v) in env {
        let val_str = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => v.to_string(),
        };
        env_map.insert(k.clone(), val_str);
    }
    Some(env_map)
}

fn build_mounts_value(mounts: &[MountSpec]) -> Option<Vec<Value>> {
    (!mounts.is_empty()).then(|| {
        mounts
            .iter()
            .map(|m| {
                if let Some(rest) = m.raw.strip_prefix("long:") {
                    let parts: Vec<&str> = rest.split(':').collect();
                    let v_type = parts.first().copied().unwrap_or("volume");
                    let src = parts.get(1).copied().unwrap_or(&m.host_or_volume);
                    let dst = parts.get(2).copied().unwrap_or(&m.container_path);
                    let read_only = parts.get(3).map(|&s| s == "ro").unwrap_or(m.read_only);

                    let mut map = serde_yaml::Mapping::new();
                    map.insert(
                        Value::String("type".to_string()),
                        Value::String(v_type.to_string()),
                    );
                    map.insert(
                        Value::String("source".to_string()),
                        Value::String(src.to_string()),
                    );
                    map.insert(
                        Value::String("target".to_string()),
                        Value::String(dst.to_string()),
                    );
                    if read_only {
                        map.insert(Value::String("read_only".to_string()), Value::Bool(true));
                    }
                    Value::Mapping(map)
                } else {
                    Value::String(m.raw.clone())
                }
            })
            .collect()
    })
}

fn build_depends_on(
    wait_for: &[DependencySpec],
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
) -> Option<IndexMap<String, ComposeDependency>> {
    let mut dep_map = IndexMap::new();
    for dep in wait_for {
        let dep_name = dep.target();
        if dep.external() || !active_workloads.contains(dep_name) {
            continue;
        }
        let compose_cond = match dep.condition() {
            DependencyCondition::Success => "service_completed_successfully",
            DependencyCondition::Started | DependencyCondition::Tcp(_) => "service_started",
            DependencyCondition::Healthy | DependencyCondition::Ready => {
                if healthy_workloads.contains(dep_name) {
                    "service_healthy"
                } else {
                    "service_started"
                }
            }
        };
        dep_map.insert(
            dep_name.to_string(),
            ComposeDependency {
                condition: compose_cond.to_string(),
            },
        );
    }
    if dep_map.is_empty() {
        None
    } else {
        Some(dep_map)
    }
}

fn build_profiles_value(
    group: &str,
    profile_name: Option<&str>,
    service_profiles: &[String],
) -> Option<Vec<String>> {
    let mut prof_list = Vec::new();
    for p in service_profiles {
        if !prof_list.contains(p) {
            prof_list.push(p.clone());
        }
    }
    if let Some(p) = profile_name {
        let p_str = p.to_string();
        if !prof_list.contains(&p_str) {
            prof_list.push(p_str);
        }
    }
    if group != "default" && group != "infra" && group != "app" && profile_name != Some(group) {
        let g = group.to_string();
        if !prof_list.contains(&g) {
            prof_list.push(g);
        }
    }
    if prof_list.is_empty() {
        None
    } else {
        Some(prof_list)
    }
}

#[allow(dead_code)]
pub fn build_compose_networks(network: &str) -> Mapping {
    let mut networks = Mapping::new();
    networks.insert(Value::String(network.to_string()), Value::Null);
    networks
}

pub fn extract_volume(m: &str) -> Option<String> {
    let clean = if let Some(rest) = m.strip_prefix("long:") {
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.first().copied() == Some("volume") && parts.len() >= 2 {
            parts[1].trim()
        } else {
            return None;
        }
    } else {
        let parts: Vec<&str> = m.split(':').collect();
        if parts.len() >= 2 {
            parts[0].trim()
        } else {
            return None;
        }
    };
    if !clean.is_empty()
        && !clean.starts_with('.')
        && !clean.starts_with('/')
        && !clean.starts_with('~')
        && !clean.contains('/')
        && !clean.contains('\\')
    {
        return Some(clean.to_string());
    }
    None
}

fn build_compose_volumes_validated(validated: &ValidatedSpec) -> Option<Mapping> {
    let spec = validated.as_spec();
    let mut names = BTreeSet::new();
    let mut collect = |profile, name: &str| {
        let workload = validated
            .workload(profile, name)
            .expect("validated workload cache");
        for mount in workload.mounts() {
            if let crate::model::MountKind::PersistentVolumeClaim { claim_name, .. } = &mount.kind {
                names.insert(claim_name.clone());
            }
        }
    };
    for name in spec.services.keys().chain(spec.jobs.keys()) {
        collect(None, name);
    }
    for (profile, value) in &spec.profiles {
        for name in value.services.keys().chain(value.jobs.keys()) {
            collect(Some(profile.as_str()), name);
        }
    }
    (!names.is_empty()).then(|| {
        names
            .into_iter()
            .map(|name| (Value::String(name), Value::Null))
            .collect()
    })
}

pub fn build_compose_volumes(spec: &AppSpec) -> Option<Mapping> {
    let mut named_volumes = BTreeSet::new();

    for svc in spec.services.values() {
        for m in &svc.mounts {
            if let Some(vol) = extract_volume(m) {
                named_volumes.insert(vol);
            }
        }
    }
    for job in spec.jobs.values() {
        for m in &job.mounts {
            if let Some(vol) = extract_volume(m) {
                named_volumes.insert(vol);
            }
        }
    }
    for prof in spec.profiles.values() {
        for svc in prof.services.values() {
            for m in &svc.mounts {
                if let Some(vol) = extract_volume(m) {
                    named_volumes.insert(vol);
                }
            }
        }
        for job in prof.jobs.values() {
            for m in &job.mounts {
                if let Some(vol) = extract_volume(m) {
                    named_volumes.insert(vol);
                }
            }
        }
    }

    if named_volumes.is_empty() {
        None
    } else {
        let mut volumes = Mapping::new();
        for vol in named_volumes {
            volumes.insert(Value::String(vol), Value::Null);
        }
        Some(volumes)
    }
}

fn format_command_value(cmd: &CommandSpec) -> Value {
    match cmd {
        CommandSpec::String(s) => {
            let cleaned = crate::command::clean_command_string(s);
            if cleaned.contains('\n')
                || cleaned.contains('"')
                || cleaned.contains('\'')
                || cleaned.contains(" -c ")
                || cleaned.starts_with("sh -c")
                || cleaned.starts_with("/bin/sh -c")
                || cleaned.starts_with("bash -c")
                || cleaned.starts_with("/bin/bash -c")
            {
                Value::String(cleaned)
            } else {
                let parts: Vec<Value> = cleaned
                    .split_whitespace()
                    .map(|p| Value::String(p.to_string()))
                    .collect();
                Value::Sequence(parts)
            }
        }
        CommandSpec::List(arr) => {
            let parts: Vec<Value> = arr
                .iter()
                .map(|s| Value::String(crate::command::clean_command_string(s)))
                .collect();
            Value::Sequence(parts)
        }
    }
}

/// Rewrites build.context to an absolute path anchored at the workspace root.
/// Skips URLs and already-absolute paths.
pub fn root_anchor_build_context(build_val: &str, root: &std::path::Path) -> String {
    let trimmed = build_val.trim();
    if is_url(trimmed) {
        return build_val.to_string();
    }

    if let Ok(serde_json::Value::Object(mut map)) =
        serde_json::from_str::<serde_json::Value>(trimmed)
    {
        if let Some(serde_json::Value::String(ref ctx)) = map.get("context") {
            if !is_url(ctx) && !std::path::Path::new(ctx).is_absolute() {
                let rel = ctx.trim_start_matches("./");
                let anchored = if rel.is_empty() || rel == "." {
                    root.to_path_buf()
                } else {
                    root.join(rel)
                };
                map.insert(
                    "context".to_string(),
                    serde_json::Value::String(anchored.to_string_lossy().into_owned()),
                );
                return serde_json::to_string(&map).unwrap_or_else(|_| build_val.to_string());
            }
        }
        return build_val.to_string();
    }

    if let Some((ctx, df)) = trimmed.split_once(':') {
        if is_url(ctx) {
            return build_val.to_string();
        }
        if std::path::Path::new(ctx).is_absolute() {
            return build_val.to_string();
        }
        let rel = ctx.trim_start_matches("./");
        let anchored = if rel.is_empty() || rel == "." {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        return format!("{}:{df}", anchored.to_string_lossy());
    }

    if std::path::Path::new(trimmed).is_absolute() {
        return build_val.to_string();
    }

    let rel = trimmed.trim_start_matches("./");
    let anchored = if rel.is_empty() || rel == "." {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    anchored.to_string_lossy().into_owned()
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("git://")
        || s.starts_with("git@")
        || s.starts_with("ssh://")
}

fn resolve_build_mapping(build_val: &str) -> Mapping {
    let trimmed = build_val.trim();

    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let mut build_map = Mapping::new();
        let ctx = map.get("context").and_then(|v| v.as_str()).unwrap_or(".");
        let df = map
            .get("dockerfile")
            .and_then(|v| v.as_str())
            .unwrap_or("Dockerfile");
        build_map.insert(
            Value::String("context".to_string()),
            Value::String(ctx.to_string()),
        );
        build_map.insert(
            Value::String("dockerfile".to_string()),
            Value::String(df.to_string()),
        );
        if let Some(serde_json::Value::Object(args)) = map.get("args") {
            let mut args_map = Mapping::new();
            for (k, v) in args {
                let v_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    _ => v.to_string(),
                };
                args_map.insert(Value::String(k.clone()), Value::String(v_str));
            }
            if !args_map.is_empty() {
                build_map.insert(Value::String("args".to_string()), Value::Mapping(args_map));
            }
        }
        return build_map;
    }

    if let Some((ctx, df)) = trimmed.split_once(':') {
        if !ctx.starts_with("http://") && !ctx.starts_with("https://") {
            let mut build_map = Mapping::new();
            build_map.insert(
                Value::String("context".to_string()),
                Value::String(if ctx.is_empty() {
                    ".".to_string()
                } else {
                    ctx.to_string()
                }),
            );
            build_map.insert(
                Value::String("dockerfile".to_string()),
                Value::String(if df.is_empty() {
                    "Dockerfile".to_string()
                } else {
                    df.to_string()
                }),
            );
            return build_map;
        }
    }

    let (context, dockerfile) = if std::path::Path::new(trimmed).is_absolute() {
        (trimmed.to_string(), "Dockerfile".to_string())
    } else if trimmed.is_empty() || trimmed == "." || trimmed == "./" {
        (".".to_string(), "Dockerfile".to_string())
    } else {
        let clean = trimmed.trim_start_matches("./");
        if clean == "Dockerfile" {
            (".".to_string(), "Dockerfile".to_string())
        } else if clean.starts_with("Dockerfile.") || clean.ends_with(".Dockerfile") {
            (".".to_string(), clean.to_string())
        } else if clean == "docker/Dockerfile"
            || clean.starts_with("docker/Dockerfile.")
            || clean.ends_with("/docker/Dockerfile")
            || clean == ".docker/Dockerfile"
            || clean.starts_with(".docker/Dockerfile.")
            || clean.ends_with("/.docker/Dockerfile")
        {
            let sub = if clean.contains(".docker/Dockerfile") {
                ".docker/Dockerfile"
            } else {
                "docker/Dockerfile"
            };
            let prefix = clean.trim_end_matches(sub).trim_end_matches('/');
            let ctx = if prefix.is_empty() {
                ".".to_string()
            } else {
                format!("./{prefix}")
            };
            let df = if prefix.is_empty() {
                clean.to_string()
            } else {
                clean[prefix.len() + 1..].to_string()
            };
            (ctx, df)
        } else if clean.ends_with("/Dockerfile") {
            let dir = clean.trim_end_matches("/Dockerfile");
            let ctx = if dir.is_empty() {
                ".".to_string()
            } else {
                format!("./{dir}")
            };
            (ctx, "Dockerfile".to_string())
        } else if let Some(pos) = clean.rfind('/') {
            let dir = &clean[..pos];
            let file = &clean[pos + 1..];
            if file == "Dockerfile"
                || file.starts_with("Dockerfile.")
                || file.ends_with(".Dockerfile")
            {
                let ctx = if dir.is_empty() {
                    ".".to_string()
                } else {
                    format!("./{dir}")
                };
                (ctx, file.to_string())
            } else {
                (format!("./{clean}"), "Dockerfile".to_string())
            }
        } else {
            (format!("./{clean}"), "Dockerfile".to_string())
        }
    };

    let mut build_map = Mapping::new();
    build_map.insert(Value::String("context".to_string()), Value::String(context));
    build_map.insert(
        Value::String("dockerfile".to_string()),
        Value::String(dockerfile),
    );
    build_map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ServiceSpec;
    #[cfg(feature = "host")]
    use indexmap::IndexMap;

    #[test]
    fn test_resolve_build_mapping_root() {
        let cases = vec![".", "./", "Dockerfile", "./Dockerfile"];
        for case in cases {
            let m = resolve_build_mapping(case);
            assert_eq!(
                m.get(Value::String("context".to_string())),
                Some(&Value::String(".".to_string()))
            );
            assert_eq!(
                m.get(Value::String("dockerfile".to_string())),
                Some(&Value::String("Dockerfile".to_string()))
            );
        }
    }

    #[test]
    fn test_resolve_build_mapping_custom_dockerfile_in_root() {
        let m = resolve_build_mapping("Dockerfile.dev");
        assert_eq!(
            m.get(Value::String("context".to_string())),
            Some(&Value::String(".".to_string()))
        );
        assert_eq!(
            m.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("Dockerfile.dev".to_string()))
        );

        let m2 = resolve_build_mapping("./Dockerfile.prod");
        assert_eq!(
            m2.get(Value::String("context".to_string())),
            Some(&Value::String(".".to_string()))
        );
        assert_eq!(
            m2.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("Dockerfile.prod".to_string()))
        );
    }

    #[test]
    fn test_resolve_build_mapping_subdirectory() {
        let m = resolve_build_mapping("./api");
        assert_eq!(
            m.get(Value::String("context".to_string())),
            Some(&Value::String("./api".to_string()))
        );
        assert_eq!(
            m.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("Dockerfile".to_string()))
        );

        let m2 = resolve_build_mapping("services/web/Dockerfile");
        assert_eq!(
            m2.get(Value::String("context".to_string())),
            Some(&Value::String("./services/web".to_string()))
        );
        assert_eq!(
            m2.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("Dockerfile".to_string()))
        );

        let m3 = resolve_build_mapping("./services/web/Dockerfile.staging");
        assert_eq!(
            m3.get(Value::String("context".to_string())),
            Some(&Value::String("./services/web".to_string()))
        );
        assert_eq!(
            m3.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("Dockerfile.staging".to_string()))
        );
    }

    #[test]
    fn test_resolve_build_mapping_explicit_colon_and_json() {
        let m = resolve_build_mapping("./api:Dockerfile.custom");
        assert_eq!(
            m.get(Value::String("context".to_string())),
            Some(&Value::String("./api".to_string()))
        );
        assert_eq!(
            m.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("Dockerfile.custom".to_string()))
        );

        let json = r#"{"context": "./backend", "dockerfile": "Dockerfile.release", "args": {"ENV": "prod"}}"#;
        let m2 = resolve_build_mapping(json);
        assert_eq!(
            m2.get(Value::String("context".to_string())),
            Some(&Value::String("./backend".to_string()))
        );
        assert_eq!(
            m2.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("Dockerfile.release".to_string()))
        );
        let args = m2
            .get(Value::String("args".to_string()))
            .and_then(|v| v.as_mapping())
            .unwrap();
        assert_eq!(
            args.get(Value::String("ENV".to_string())),
            Some(&Value::String("prod".to_string()))
        );
    }

    #[test]
    fn test_resolve_build_mapping_docker_folder() {
        let m = resolve_build_mapping("docker/Dockerfile");
        assert_eq!(
            m.get(Value::String("context".to_string())),
            Some(&Value::String(".".to_string()))
        );
        assert_eq!(
            m.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("docker/Dockerfile".to_string()))
        );

        let m2 = resolve_build_mapping("./docker/Dockerfile");
        assert_eq!(
            m2.get(Value::String("context".to_string())),
            Some(&Value::String(".".to_string()))
        );
        assert_eq!(
            m2.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("docker/Dockerfile".to_string()))
        );

        let m3 = resolve_build_mapping(".:docker/Dockerfile");
        assert_eq!(
            m3.get(Value::String("context".to_string())),
            Some(&Value::String(".".to_string()))
        );
        assert_eq!(
            m3.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("docker/Dockerfile".to_string()))
        );

        let m4 = resolve_build_mapping("./services/web/docker/Dockerfile");
        assert_eq!(
            m4.get(Value::String("context".to_string())),
            Some(&Value::String("./services/web".to_string()))
        );
        assert_eq!(
            m4.get(Value::String("dockerfile".to_string())),
            Some(&Value::String("docker/Dockerfile".to_string()))
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_generate_compose_build_service() {
        let mut services = IndexMap::new();
        let svc = ServiceSpec {
            build: Some(".".to_string()),
            ..Default::default()
        };
        services.insert("web".to_string(), svc);

        let spec = AppSpec {
            app: "build-test-app".to_string(),
            services,
            ..Default::default()
        };

        let compose = generate_compose(&spec).expect("Failed to generate compose");
        assert!(compose.contains("name: build-test-app"));
        let root = std::env::current_dir().unwrap();
        assert!(compose.contains(&format!("context: {}", root.display())));
        assert!(compose.contains("dockerfile: Dockerfile"));
        assert!(!compose.contains("REDHAT_BASE_REGISTRY"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_generic_healthcheck_emission() {
        let mut services = IndexMap::new();
        let svc_exec = ServiceSpec {
            image: Some("redis:alpine".to_string()),
            health: Some(serde_json::Value::String("redis-cli ping".to_string())),
            ..Default::default()
        };
        let svc_custom = ServiceSpec {
            image: Some("custom:1.0".to_string()),
            health: Some(serde_json::Value::String(
                "custom-check --status".to_string(),
            )),
            ..Default::default()
        };
        services.insert("cache".to_string(), svc_exec);
        services.insert("custom".to_string(), svc_custom);

        let spec = AppSpec {
            app: "health-test-app".to_string(),
            services,
            ..Default::default()
        };

        let compose = generate_compose(&spec).expect("Failed to generate compose");
        assert!(compose.contains("CMD-SHELL"));
        assert!(compose.contains("redis-cli ping"));
        assert!(compose.contains("custom-check --status"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_parameterized_healthcheck_custom_timings() {
        let mut services = IndexMap::new();
        let hc_json = serde_json::json!({
            "test": "/status",
            "interval": "5s",
            "timeout": "2s",
            "retries": 3,
            "start_period": "10s"
        });
        let svc = ServiceSpec {
            image: Some("api:v1".to_string()),
            ports: vec![serde_json::Value::from(8080)],
            health: Some(hc_json),
            ..Default::default()
        };
        services.insert("api".to_string(), svc);

        let spec = AppSpec {
            app: "timed-health-app".to_string(),
            services,
            ..Default::default()
        };

        let compose = generate_compose(&spec).expect("generate_compose failed");
        assert!(compose.contains("interval: 5s"));
        assert!(compose.contains("timeout: 2s"));
        assert!(compose.contains("retries: 3"));
        assert!(compose.contains("start_period: 10s"));
        assert!(compose.contains("http://localhost:8080/status"));
    }

    #[test]
    fn test_modular_helpers_isolation() {
        let mut active = HashSet::new();
        active.insert("db".to_string());
        let mut healthy = HashSet::new();
        healthy.insert("db".to_string());

        let svc = ServiceSpec {
            image: Some("postgres:16".to_string()),
            mounts: vec!["db-data:/var/lib/postgresql/data".to_string()],
            ..Default::default()
        };
        let mut spec = AppSpec {
            app: "test-app".to_string(),
            network: "custom-net".to_string(),
            ..Default::default()
        };
        spec.services.insert("db".to_string(), svc.clone());

        let svc_map = build_compose_service("db", &svc, &spec, &active, &healthy, None).unwrap();
        assert_eq!(
            svc_map.get(Value::String("image".to_string())),
            Some(&Value::String("postgres:16".to_string()))
        );

        let net_map = build_compose_networks(&spec.network);
        assert!(net_map.contains_key(Value::String("custom-net".to_string())));

        let vol_map = build_compose_volumes(&spec).expect("volumes expected");
        assert!(vol_map.contains_key(Value::String("db-data".to_string())));
    }
    #[cfg(feature = "host")]
    #[test]
    fn test_validated_compose_dependencies_and_cache() {
        let spec: AppSpec = serde_json::from_value(serde_json::json!({
            "app": "cache-test", "version": "1", "env_files": [],
            "services": {
                "db": {"image": "postgres", "health": "pg_isready", "ports": ["5432:5432"]},
                "web": {"image": "nginx", "ports": [8080], "command": ["echo", "a b"],
                    "mounts": ["web-data:/data:ro"], "waitFor": ["db.healthy", "external.example:443"]}
            }
        })).unwrap();
        let validated = ValidatedSpec::try_from(&spec).unwrap();
        let yaml: Value =
            serde_yaml::from_str(&generate_compose_validated(&validated).unwrap()).unwrap();
        let web = &yaml["services"]["web"];
        assert_eq!(
            web["depends_on"]["db"]["condition"].as_str(),
            Some("service_healthy")
        );
        assert_eq!(web["depends_on"].as_mapping().unwrap().len(), 1);
        assert_eq!(web["ports"][0].as_str(), Some("8080:8080"));
        assert_eq!(web["command"][1].as_str(), Some("a b"));
        assert_eq!(web["volumes"][0].as_str(), Some("web-data:/data:ro"));
        assert!(yaml["volumes"]
            .as_mapping()
            .unwrap()
            .contains_key(Value::String("web-data".into())));
    }

    #[test]
    fn test_compose_helper_rejects_invalid_supplied_workload() {
        let spec = AppSpec::default();
        let svc = ServiceSpec {
            ports: vec![serde_json::json!(false)],
            ..Default::default()
        };
        let error =
            build_compose_service("web", &svc, &spec, &HashSet::new(), &HashSet::new(), None)
                .unwrap_err();
        assert!(error.to_string().contains("ports"));
    }

    #[test]
    fn test_compose_healthcheck_array_support() {
        let mut svc = ServiceSpec {
            image: Some("ghcr.io/cerbos/cerbos:0.45.0".to_string()),
            ..Default::default()
        };
        svc.health = Some(serde_json::json!(["cerbos", "healthcheck"]));

        let hc = build_compose_healthcheck(&svc).expect("healthcheck mapping");
        let test_val = hc.get("test").expect("test entry");
        assert_eq!(
            test_val,
            &Value::Sequence(vec![
                Value::String("CMD".to_string()),
                Value::String("cerbos".to_string()),
                Value::String("healthcheck".to_string()),
            ])
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_compose_entrypoint_support() {
        let mut spec = AppSpec {
            app: "entrypoint-test".to_string(),
            ..Default::default()
        };
        let svc = ServiceSpec {
            image: Some("alpine:latest".to_string()),
            entrypoint: Some(serde_json::json!([
                "/bin/custom-entrypoint.sh",
                "--verbose"
            ])),
            command: Some(serde_json::json!(["run", "server"])),
            ..Default::default()
        };
        spec.services.insert("custom".to_string(), svc);

        let compose = generate_compose(&spec).expect("compose yaml");
        let parsed: Value = serde_yaml::from_str(&compose).expect("valid yaml");
        let service = &parsed["services"]["custom"];
        assert_eq!(
            service["entrypoint"],
            Value::Sequence(vec![
                Value::String("/bin/custom-entrypoint.sh".to_string()),
                Value::String("--verbose".to_string()),
            ])
        );
        assert_eq!(
            service["command"],
            Value::Sequence(vec![
                Value::String("run".to_string()),
                Value::String("server".to_string()),
            ])
        );
    }

    #[test]
    fn test_diff_paths_relative_and_absolute() {
        let root = Path::new("/Users/dev/project");
        let to = root.join("config/.env");

        // Compose file in ved/ subdirectory
        let from_ved = root.join("ved");
        assert_eq!(diff_paths(&to, &from_ved), PathBuf::from("../config/.env"));

        // Compose file in .ved/ subdirectory
        let from_dot_ved = root.join(".ved");
        assert_eq!(
            diff_paths(&to, &from_dot_ved),
            PathBuf::from("../config/.env")
        );

        // Compose file in root directory
        assert_eq!(diff_paths(&to, root), PathBuf::from("config/.env"));

        // Deeply nested compose file
        let from_nested = root.join("deploy/staging");
        assert_eq!(
            diff_paths(&to, &from_nested),
            PathBuf::from("../../config/.env")
        );

        // Same directory
        let from_config = root.join("config");
        assert_eq!(diff_paths(&to, &from_config), PathBuf::from(".env"));
    }

    #[test]
    fn test_rebase_env_file_absolute_and_relative() {
        let root = Path::new("/Users/dev/project");
        let compose_dir = root.join("ved");

        // Relative path rebasing
        assert_eq!(
            rebase_env_file("config/.env", root, &compose_dir),
            "../config/.env"
        );
        assert_eq!(
            rebase_env_file("config/.local.env", root, &compose_dir),
            "../config/.local.env"
        );

        // Absolute path preserved
        assert_eq!(
            rebase_env_file("/etc/ssl/env.conf", root, &compose_dir),
            "/etc/ssl/env.conf"
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_compose_rebases_env_file_to_compose_location() {
        let tmp =
            std::env::temp_dir().join(format!("ved-compose-rebase-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("config")).unwrap();
        std::fs::create_dir_all(tmp.join("ved")).unwrap();

        let mut spec = AppSpec {
            app: "rebase-app".to_string(),
            version: "1.0.0".to_string(),
            env_files: vec!["config/.env".to_string(), "config/.local.env".to_string()],
            ..Default::default()
        };
        spec.services.insert(
            "api".to_string(),
            ServiceSpec {
                image: Some("alpine:latest".to_string()),
                ..Default::default()
            },
        );

        // 1. Context configured with COMPOSE_PATH=ved/docker-compose.yml
        let mut env_map = std::collections::BTreeMap::new();
        env_map.insert(
            "COMPOSE_PATH".to_string(),
            "ved/docker-compose.yml".to_string(),
        );
        let locator =
            crate::ConfigLocator::new(tmp.join("config"), tmp.join("config/.generated.env"));
        let snapshot = crate::ConfigSnapshot::with_environment(&locator, env_map);
        let ctx = crate::WorkspaceContext::with_snapshot(
            tmp.clone(),
            tmp.join("config"),
            tmp.join(".ved"),
            None,
            tmp.join("config/.generated.env"),
            snapshot,
        );

        let validated = ValidatedSpec::try_from(&spec).unwrap();
        let yaml_str = generate_compose_validated_with_context(&validated, &ctx).unwrap();
        let parsed: Value = serde_yaml::from_str(&yaml_str).unwrap();
        let env_file = &parsed["services"]["api"]["env_file"];
        assert_eq!(
            env_file,
            &Value::Sequence(vec![
                Value::String("config/.env".to_string()),
                Value::String("config/.local.env".to_string()),
            ])
        );

        // 2. Context configured with COMPOSE_PATH=docker-compose.yml (at root)
        let mut root_env = std::collections::BTreeMap::new();
        root_env.insert("COMPOSE_PATH".to_string(), "docker-compose.yml".to_string());
        let root_snapshot = crate::ConfigSnapshot::with_environment(&locator, root_env);
        let root_ctx = crate::WorkspaceContext::with_snapshot(
            tmp.clone(),
            tmp.join("config"),
            tmp.join(".ved"),
            None,
            tmp.join("config/.generated.env"),
            root_snapshot,
        );
        let root_yaml = generate_compose_validated_with_context(&validated, &root_ctx).unwrap();
        let root_parsed: Value = serde_yaml::from_str(&root_yaml).unwrap();
        let root_env_file = &root_parsed["services"]["api"]["env_file"];
        assert_eq!(
            root_env_file,
            &Value::Sequence(vec![
                Value::String("config/.env".to_string()),
                Value::String("config/.local.env".to_string()),
            ])
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
