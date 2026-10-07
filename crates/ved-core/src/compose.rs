use crate::error::Result;
use crate::validated::{DependencyCondition, DependencySpec, ValidatedSpec, ValidatedWorkload};
use serde_yaml::{Mapping, Value};

use std::collections::{BTreeSet, HashSet};

use crate::model::{
    AppSpec, CommandSpec, HealthCheckSpec, JobSpec, MountSpec, PortSpec, ServiceSpec,
};

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
    let mut root = Mapping::new();
    root.insert(
        Value::String("name".to_string()),
        Value::String(spec.app.clone()),
    );
    let mut services = Mapping::new();

    let (active_workloads, healthy_workloads) = collect_active_workloads(spec);

    for (name, svc) in &spec.services {
        if svc.enabled.unwrap_or(true) {
            let svc_map = build_compose_service_typed(
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
            );
            services.insert(Value::String(name.clone()), Value::Mapping(svc_map));
        }
    }

    for (name, job) in &spec.jobs {
        if job.enabled.unwrap_or(true) {
            let job_map = build_compose_job_typed(
                name,
                job,
                spec,
                &active_workloads,
                &healthy_workloads,
                None,
                validated.workload(None, name).expect("validated job cache"),
            );
            services.insert(Value::String(name.clone()), Value::Mapping(job_map));
        }
    }

    for (prof_name, prof) in &spec.profiles {
        for (name, svc) in &prof.services {
            if svc.enabled.unwrap_or(true) {
                let svc_map = build_compose_service_typed(
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
                );
                services.insert(Value::String(name.clone()), Value::Mapping(svc_map));
            }
        }
        for (name, job) in &prof.jobs {
            if job.enabled.unwrap_or(true) {
                let job_map = build_compose_job_typed(
                    name,
                    job,
                    spec,
                    &active_workloads,
                    &healthy_workloads,
                    Some(prof_name),
                    validated
                        .workload(Some(prof_name), name)
                        .expect("validated job cache"),
                );
                services.insert(Value::String(name.clone()), Value::Mapping(job_map));
            }
        }
    }

    root.insert(
        Value::String("services".to_string()),
        Value::Mapping(services),
    );

    root.insert(
        Value::String("networks".to_string()),
        Value::Mapping(build_compose_networks(&spec.network)),
    );

    if let Some(volumes) = build_compose_volumes_validated(validated) {
        root.insert(
            Value::String("volumes".to_string()),
            Value::Mapping(volumes),
        );
    }

    let yaml_str = serde_yaml::to_string(&Value::Mapping(root))?;
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
    Ok(build_compose_service_typed(
        name,
        value,
        validated.as_spec(),
        active_workloads,
        healthy_workloads,
        profile_name,
        workload,
        None,
    ))
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
    Ok(build_compose_job_typed(
        name,
        value,
        validated.as_spec(),
        active_workloads,
        healthy_workloads,
        profile_name,
        workload,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_compose_service_typed(
    name: &str,
    svc: &ServiceSpec,
    spec: &AppSpec,
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
    profile_name: Option<&str>,
    workload: &ValidatedWorkload,
    ingress_suffix: Option<&str>,
) -> Mapping {
    let mut svc_map = Mapping::new();

    if let Some(img) = &svc.image {
        svc_map.insert(
            Value::String("image".to_string()),
            Value::String(img.clone()),
        );
    }

    svc_map.insert(
        Value::String("container_name".to_string()),
        Value::String(name.to_string()),
    );

    if let Some(ports) = build_ports_value(workload.ports()) {
        svc_map.insert(Value::String("ports".to_string()), ports);
    }

    if let Some(env_file) = build_env_file_value(&spec.env_files) {
        svc_map.insert(Value::String("env_file".to_string()), env_file);
    }

    if let Some(env_map) = build_env_map(&svc.env) {
        svc_map.insert(
            Value::String("environment".to_string()),
            Value::Mapping(env_map),
        );
    }

    if let Some(volumes) = build_mounts_value(workload.mounts()) {
        svc_map.insert(Value::String("volumes".to_string()), volumes);
    }

    if let Some(hc) = build_compose_healthcheck_typed(svc, workload.ports()) {
        svc_map.insert(Value::String("healthcheck".to_string()), Value::Mapping(hc));
    }

    if let Some(deps) =
        build_depends_on(workload.dependencies(), active_workloads, healthy_workloads)
    {
        svc_map.insert(
            Value::String("depends_on".to_string()),
            Value::Mapping(deps),
        );
    }

    svc_map.insert(
        Value::String("restart".to_string()),
        Value::String(
            svc.restart
                .clone()
                .unwrap_or_else(|| "unless-stopped".to_string()),
        ),
    );

    if let Some(b) = &svc.build {
        svc_map.insert(
            Value::String("build".to_string()),
            Value::Mapping(resolve_build_mapping(b)),
        );
    }

    if let Some(cmd) = workload.command() {
        let cmd_val = format_command_value(cmd);
        if cmd_val != Value::Null {
            svc_map.insert(Value::String("command".to_string()), cmd_val);
        }
    }

    svc_map.insert(
        Value::String("networks".to_string()),
        Value::Sequence(vec![Value::String(spec.network.clone())]),
    );

    if let Some(prof) = build_profiles_value(&svc.group, profile_name) {
        svc_map.insert(Value::String("profiles".to_string()), prof);
    }

    if let Some(labels) = build_ingress_labels(svc.ingress.as_deref(), ingress_suffix) {
        svc_map.insert(Value::String("labels".to_string()), Value::Mapping(labels));
    }

    svc_map
}

fn build_ingress_labels(ingress: Option<&str>, ingress_suffix: Option<&str>) -> Option<Mapping> {
    let ingress = ingress?.trim();
    if ingress.is_empty() {
        return None;
    }
    let suffix = ingress_suffix?.trim();
    if suffix.is_empty() {
        return None;
    }
    Some(
        [("ingress.host".to_string(), format!("{ingress}.{suffix}"))]
            .into_iter()
            .map(|(k, v)| (Value::String(k), Value::String(v)))
            .collect(),
    )
}

fn build_compose_job_typed(
    name: &str,
    job: &JobSpec,
    spec: &AppSpec,
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
    profile_name: Option<&str>,
    workload: &ValidatedWorkload,
) -> Mapping {
    let mut job_map = Mapping::new();
    job_map.insert(
        Value::String("image".to_string()),
        Value::String(job.image.clone()),
    );
    job_map.insert(
        Value::String("container_name".to_string()),
        Value::String(name.to_string()),
    );

    if let Some(cmd) = workload.command() {
        let cmd_val = format_command_value(cmd);
        if cmd_val != Value::Null {
            job_map.insert(Value::String("command".to_string()), cmd_val);
        }
    }

    if let Some(volumes) = build_mounts_value(workload.mounts()) {
        job_map.insert(Value::String("volumes".to_string()), volumes);
    }

    if let Some(env_file) = build_env_file_value(&spec.env_files) {
        job_map.insert(Value::String("env_file".to_string()), env_file);
    }

    if let Some(env_map) = build_env_map(&job.env) {
        job_map.insert(
            Value::String("environment".to_string()),
            Value::Mapping(env_map),
        );
    }

    if let Some(deps) =
        build_depends_on(workload.dependencies(), active_workloads, healthy_workloads)
    {
        job_map.insert(
            Value::String("depends_on".to_string()),
            Value::Mapping(deps),
        );
    }

    job_map.insert(
        Value::String("restart".to_string()),
        Value::String("no".to_string()),
    );

    job_map.insert(
        Value::String("networks".to_string()),
        Value::Sequence(vec![Value::String(spec.network.clone())]),
    );

    if let Some(prof) = build_profiles_value(&job.group, profile_name) {
        job_map.insert(Value::String("profiles".to_string()), prof);
    }

    job_map
}

pub fn build_compose_healthcheck(svc: &ServiceSpec) -> Option<Mapping> {
    build_compose_healthcheck_typed(svc, &svc.typed_ports())
}

fn build_compose_healthcheck_typed(svc: &ServiceSpec, ports: &[PortSpec]) -> Option<Mapping> {
    let health_val = svc.health_value()?;
    let hc_spec = HealthCheckSpec::from_value(health_val);

    let mut test_seq = Vec::new();
    if let Some(test_val) = &hc_spec.test {
        if let Some(cmd) = test_val.as_str() {
            test_seq = build_health_test_seq(cmd, ports);
        } else if let Some(arr) = test_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    test_seq.push(Value::String(s.to_string()));
                }
            }
        }
    }

    if test_seq.is_empty() {
        return None;
    }

    let mut hc = Mapping::new();
    hc.insert(Value::String("test".to_string()), Value::Sequence(test_seq));
    hc.insert(
        Value::String("interval".to_string()),
        Value::String(hc_spec.interval),
    );
    hc.insert(
        Value::String("timeout".to_string()),
        Value::String(hc_spec.timeout),
    );
    hc.insert(
        Value::String("retries".to_string()),
        Value::Number(hc_spec.retries.into()),
    );
    hc.insert(
        Value::String("start_period".to_string()),
        Value::String(hc_spec.start_period),
    );
    Some(hc)
}

fn build_health_test_seq(cmd: &str, ports: &[PortSpec]) -> Vec<Value> {
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

        test_seq.push(Value::String("CMD-SHELL".to_string()));
        test_seq.push(Value::String(format!(
            "wget -qO- {url} > /dev/null 2>&1 || curl -fs {url} > /dev/null 2>&1 || exit 1"
        )));
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
        test_seq.push(Value::String("CMD-SHELL".to_string()));
        test_seq.push(Value::String(format!(
            "nc -z localhost {port_str} || (echo > /dev/tcp/localhost/{port_str}) 2>/dev/null || exit 1"
        )));
    } else {
        test_seq.push(Value::String("CMD-SHELL".to_string()));
        test_seq.push(Value::String(cmd.to_string()));
    }
    test_seq
}

fn build_ports_value(ports: &[PortSpec]) -> Option<Value> {
    (!ports.is_empty()).then(|| {
        Value::Sequence(
            ports
                .iter()
                .map(|port| Value::String(format!("{}:{}", port.host, port.container)))
                .collect(),
        )
    })
}

fn build_env_file_value(env_files: &[String]) -> Option<Value> {
    if env_files.is_empty() {
        None
    } else {
        Some(Value::Sequence(
            env_files.iter().map(|f| Value::String(f.clone())).collect(),
        ))
    }
}

fn build_env_map(env: &indexmap::IndexMap<String, serde_json::Value>) -> Option<Mapping> {
    if env.is_empty() {
        return None;
    }
    let mut env_map = Mapping::new();
    for (k, v) in env {
        let val_str = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => v.to_string(),
        };
        env_map.insert(Value::String(k.clone()), Value::String(val_str));
    }
    Some(env_map)
}

fn build_mounts_value(mounts: &[MountSpec]) -> Option<Value> {
    if mounts.is_empty() {
        None
    } else {
        Some(Value::Sequence(
            mounts
                .iter()
                .map(|m| Value::String(m.raw.clone()))
                .collect(),
        ))
    }
}

fn build_depends_on(
    wait_for: &[DependencySpec],
    active_workloads: &HashSet<String>,
    healthy_workloads: &HashSet<String>,
) -> Option<Mapping> {
    let mut dep_map = Mapping::new();
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
        let mut cond_map = Mapping::new();
        cond_map.insert(
            Value::String("condition".to_string()),
            Value::String(compose_cond.to_string()),
        );
        dep_map.insert(
            Value::String(dep_name.to_string()),
            Value::Mapping(cond_map),
        );
    }
    if dep_map.is_empty() {
        None
    } else {
        Some(dep_map)
    }
}

fn build_profiles_value(group: &str, profile_name: Option<&str>) -> Option<Value> {
    let mut prof_list = Vec::new();
    if let Some(p) = profile_name {
        prof_list.push(Value::String(p.to_string()));
    }
    if group != "default" && group != "infra" && group != "app" && profile_name != Some(group) {
        prof_list.push(Value::String(group.to_string()));
    }
    if prof_list.is_empty() {
        None
    } else {
        Some(Value::Sequence(prof_list))
    }
}

pub fn build_compose_networks(network: &str) -> Mapping {
    let mut networks = Mapping::new();
    networks.insert(Value::String(network.to_string()), Value::Null);
    networks
}

pub fn extract_volume(m: &str) -> Option<String> {
    let parts: Vec<&str> = m.split(':').collect();
    if parts.len() >= 2 {
        let src = parts[0].trim();
        if !src.is_empty()
            && !src.starts_with('.')
            && !src.starts_with('/')
            && !src.starts_with('~')
            && !src.contains('/')
            && !src.contains('\\')
        {
            return Some(src.to_string());
        }
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

    let (context, dockerfile) = if trimmed.is_empty() || trimmed == "." || trimmed == "./" {
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
        assert!(compose.contains("context: ."));
        assert!(compose.contains("dockerfile: Dockerfile"));
        assert!(!compose.contains("/Dockerfile"));
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
}
