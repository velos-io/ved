//! Immutable, semantically checked compiler inputs.
use crate::error::{Result, ValidationError, ValidationReason, VedError};
use crate::model::{AppSpec, CheckedPortSpec, CommandSpec, MountSpec, PortSpec};
use indexmap::IndexMap;

/// A specification whose typed fields and dependency graph have been checked.
///
/// ```compile_fail
/// use ved_core::{model::AppSpec, ValidatedSpec};
/// let mut spec = ValidatedSpec::try_from(AppSpec::default()).unwrap();
/// spec.as_spec().app = "changed".into();
/// ```
/// ```compile_fail
/// use ved_core::{model::AppSpec, ValidatedSpec};
/// let spec = ValidatedSpec::try_from(AppSpec::default()).unwrap();
/// let _ = spec.wire;
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkloadKey {
    pub profile: Option<String>,
    pub name: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct WorkloadQuery<'a> {
    profile: Option<&'a str>,
    name: &'a str,
}

impl indexmap::Equivalent<WorkloadKey> for WorkloadQuery<'_> {
    fn equivalent(&self, key: &WorkloadKey) -> bool {
        self.profile == key.profile.as_deref() && self.name == key.name
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedSpec {
    wire: AppSpec,
    workloads: IndexMap<WorkloadKey, ValidatedWorkload>,
}
#[derive(Debug, Clone)]
pub struct ValidatedWorkload {
    ports: Vec<PortSpec>,
    checked_ports: Vec<CheckedPortSpec>,
    command: Option<CommandSpec>,
    mounts: Vec<MountSpec>,
    dependencies: Vec<DependencySpec>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyCondition {
    Ready,
    Healthy,
    Started,
    Success,
    Tcp(u16),
}
#[derive(Debug, Clone)]
pub struct DependencySpec {
    target: String,
    condition: DependencyCondition,
    external: bool,
}
impl DependencySpec {
    pub fn target(&self) -> &str {
        &self.target
    }
    pub fn condition(&self) -> DependencyCondition {
        self.condition
    }
    pub fn external(&self) -> bool {
        self.external
    }
}
impl ValidatedWorkload {
    pub fn ports(&self) -> &[PortSpec] {
        &self.ports
    }
    /// Checked mappings corresponding to the compatibility `ports()` slice.
    pub fn checked_ports(&self) -> &[CheckedPortSpec] {
        &self.checked_ports
    }
    pub fn command(&self) -> Option<&CommandSpec> {
        self.command.as_ref()
    }
    pub fn mounts(&self) -> &[MountSpec] {
        &self.mounts
    }
    pub fn dependencies(&self) -> &[DependencySpec] {
        &self.dependencies
    }
}
impl ValidatedSpec {
    /// Freeze activation and default environment paths without reparsing cached fields.
    pub fn with_context(&self, context: &crate::WorkspaceContext) -> Self {
        let mut result = self.clone();
        result.wire = context.prepare_spec(&self.wire);
        result
    }
    pub fn as_spec(&self) -> &AppSpec {
        &self.wire
    }
    pub fn workload(&self, profile: Option<&str>, name: &str) -> Option<&ValidatedWorkload> {
        self.workloads.get(&WorkloadQuery { profile, name })
    }
}
#[derive(Clone)]
struct Context {
    profile: Option<String>,
    kind: String,
    name: String,
}
impl std::fmt::Display for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(p) = &self.profile {
            write!(f, "Profile '{p}' ")?;
        }
        write!(f, "{} '{}'", self.kind, self.name)
    }
}
fn error(
    context: &Context,
    field: &str,
    reason: ValidationReason,
    message: impl std::fmt::Display,
) -> VedError {
    ValidationError::new(
        reason,
        context.profile.clone(),
        Some(context.kind.clone()),
        Some(context.name.clone()),
        field,
    )
    .with_detail(message.to_string())
    .into()
}
fn context(profile: Option<&str>, kind: &str, name: &str) -> Context {
    Context {
        profile: profile.map(str::to_owned),
        kind: kind.into(),
        name: name.into(),
    }
}
fn workload(
    context: &Context,
    port: Option<u16>,
    ports: &[serde_json::Value],
    command: Option<&serde_json::Value>,
    mounts: &[String],
) -> Result<ValidatedWorkload> {
    if port == Some(0) {
        return Err(error(
            context,
            "port",
            ValidationReason::InvalidPort,
            "must be between 1 and 65535",
        ));
    }
    let mut checked_ports = Vec::new();
    for (index, value) in ports.iter().enumerate() {
        let parsed = CheckedPortSpec::try_from(value.clone()).map_err(|source| {
            VedError::from(
                ValidationError::new(
                    ValidationReason::InvalidPort,
                    context.profile.clone(),
                    Some(context.kind.clone()),
                    Some(context.name.clone()),
                    format!("ports[{index}]"),
                )
                .with_detail("expected a nonzero port or port mapping")
                .with_source(source),
            )
        })?;
        checked_ports.push(parsed);
    }
    if checked_ports.is_empty() {
        if let Some(port) = port {
            checked_ports.push(CheckedPortSpec::single(port).map_err(|_e| {
                error(
                    context,
                    "port",
                    ValidationReason::InvalidPort,
                    "expected a nonzero port",
                )
            })?);
        }
    }
    let parsed_ports = checked_ports.iter().copied().map(PortSpec::from).collect();
    let command = command
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|e| {
                let cause = ValidationError::new(
                    ValidationReason::InvalidCommand,
                    context.profile.clone(),
                    Some(context.kind.clone()),
                    Some(context.name.clone()),
                    "command",
                )
                .with_detail("expected a command string or list of strings")
                .with_source(e);
                VedError::from(cause)
            })
        })
        .transpose()?;
    let mounts = mounts
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            MountSpec::parse(raw).map_err(|source| {
                ValidationError::new(
                    ValidationReason::InvalidMount,
                    context.profile.clone(),
                    Some(context.kind.clone()),
                    Some(context.name.clone()),
                    format!("mounts[{index}]"),
                )
                .with_detail("invalid mount: expected a supported mount syntax")
                .with_source(source)
                .into()
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ValidatedWorkload {
        ports: parsed_ports,
        checked_ports,
        command,
        mounts,
        dependencies: Vec::new(),
    })
}
fn dependency(
    context: &Context,
    index: usize,
    raw: &str,
    names: &IndexMap<String, usize>,
) -> Result<DependencySpec> {
    let field = format!("waitFor[{index}]");
    let normalised = if !raw.contains(':') && !names.contains_key(raw) {
        match raw.rsplit_once('.') {
            Some((target, condition))
                if matches!(
                    condition,
                    "ready" | "healthy" | "started" | "success" | "completed"
                ) =>
            {
                format!("{target}:{condition}")
            }
            _ => raw.to_owned(),
        }
    } else {
        raw.to_owned()
    };
    let parts: Vec<_> = normalised.split(':').collect();
    if parts.len() > 2 || parts[0].is_empty() {
        return Err(error(
            context,
            &field,
            ValidationReason::MalformedDependency,
            "malformed dependency",
        ));
    }
    let target = parts[0];
    let condition = match parts.get(1).copied() {
        None => DependencyCondition::Healthy,
        Some("ready") => DependencyCondition::Ready,
        Some("healthy") => DependencyCondition::Healthy,
        Some("started") => DependencyCondition::Started,
        Some("success" | "completed") => DependencyCondition::Success,
        Some(value) => {
            let port = value.parse::<u16>().map_err(|source| {
                VedError::from(
                    ValidationError::new(
                        ValidationReason::InvalidDependencyCondition,
                        context.profile.clone(),
                        Some(context.kind.clone()),
                        Some(context.name.clone()),
                        &field,
                    )
                    .with_detail("unknown condition or invalid TCP port")
                    .with_source(source),
                )
            })?;
            if port == 0 {
                return Err(error(
                    context,
                    &field,
                    ValidationReason::InvalidDependencyCondition,
                    "TCP port must be nonzero",
                ));
            }
            DependencyCondition::Tcp(port)
        }
    };
    let external = !names.contains_key(target);
    if external {
        if !matches!(condition, DependencyCondition::Tcp(_)) {
            return Err(error(
                context,
                &field,
                ValidationReason::UnknownDependency,
                "unknown workload",
            ));
        }
        if !target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            || !target
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric())
        {
            return Err(error(
                context,
                &field,
                ValidationReason::InvalidExternalHost,
                "invalid external hostname",
            ));
        }
    }
    Ok(DependencySpec {
        target: target.into(),
        condition,
        external,
    })
}
impl TryFrom<&AppSpec> for ValidatedSpec {
    type Error = VedError;
    fn try_from(spec: &AppSpec) -> Result<Self> {
        Self::try_from(spec.clone())
    }
}
impl TryFrom<AppSpec> for ValidatedSpec {
    type Error = VedError;
    fn try_from(wire: AppSpec) -> Result<Self> {
        Self::build(wire, true)
    }
}

impl ValidatedSpec {
    /// Check catalog fields before merge; references resolve against the merged app.
    #[cfg(feature = "host")]
    pub(crate) fn validate_fields(spec: &AppSpec) -> Result<()> {
        Self::build(spec.clone(), false).map(|_| ())
    }

    fn build(wire: AppSpec, resolve_references: bool) -> Result<Self> {
        wire.validate_identifiers()?;
        let mut workloads = IndexMap::new();
        let mut names = IndexMap::new();
        let mut waits = Vec::new();
        let mut contexts = Vec::new();
        let mut insert = |profile: Option<&str>,
                          kind: &str,
                          name: &str,
                          parsed: ValidatedWorkload,
                          deps: &[String]|
         -> Result<()> {
            let ctx = context(profile, kind, name);
            if let Some(previous) = names.get(name) {
                return Err(error(
                    &ctx,
                    "name",
                    ValidationReason::DuplicateWorkload,
                    format!("duplicates workload defined by {}", contexts[*previous]),
                ));
            }
            names.insert(name.to_owned(), contexts.len());
            contexts.push(ctx);
            waits.push(deps.to_vec());
            workloads.insert(
                WorkloadKey {
                    profile: profile.map(str::to_owned),
                    name: name.to_owned(),
                },
                parsed,
            );
            Ok(())
        };
        for (name, svc) in &wire.services {
            insert(
                None,
                "Service",
                name,
                workload(
                    &context(None, "Service", name),
                    svc.port,
                    &svc.ports,
                    svc.command.as_ref(),
                    &svc.mounts,
                )?,
                &svc.wait_for,
            )?;
        }
        for (name, job) in &wire.jobs {
            insert(
                None,
                "Job",
                name,
                workload(
                    &context(None, "Job", name),
                    None,
                    &[],
                    job.command.as_ref(),
                    &job.mounts,
                )?,
                &job.wait_for,
            )?;
        }
        for (profile, spec) in &wire.profiles {
            crate::identifier::validate_identifier("profile", profile)?;
            for (name, svc) in &spec.services {
                insert(
                    Some(profile),
                    "Service",
                    name,
                    workload(
                        &context(Some(profile), "Service", name),
                        svc.port,
                        &svc.ports,
                        svc.command.as_ref(),
                        &svc.mounts,
                    )?,
                    &svc.wait_for,
                )?;
            }
            for (name, job) in &spec.jobs {
                insert(
                    Some(profile),
                    "Job",
                    name,
                    workload(
                        &context(Some(profile), "Job", name),
                        None,
                        &[],
                        job.command.as_ref(),
                        &job.mounts,
                    )?,
                    &job.wait_for,
                )?;
            }
        }
        if !resolve_references {
            return Ok(Self { wire, workloads });
        }
        let mut graph = vec![Vec::new(); workloads.len()];
        for (index, parsed) in workloads.values_mut().enumerate() {
            for (dep_index, raw) in waits[index].iter().enumerate() {
                let dep = dependency(&contexts[index], dep_index, raw, &names)?;
                if !dep.external {
                    graph[index].push(names[&dep.target]);
                }
                parsed.dependencies.push(dep);
            }
        }
        // Iterative depth-first traversal avoids stack overflow on deeply nested input.
        let mut state = vec![0u8; graph.len()];
        for start in 0..graph.len() {
            if state[start] != 0 {
                continue;
            }
            state[start] = 1;
            let mut stack = vec![(start, 0usize)];
            while let Some(&(node, next)) = stack.last() {
                if next == graph[node].len() {
                    state[node] = 2;
                    stack.pop();
                    continue;
                }
                let target = graph[node][next];
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                if state[target] == 1 {
                    let begin = stack
                        .iter()
                        .position(|(node, _)| *node == target)
                        .unwrap_or(0);
                    let mut cycle: Vec<_> = stack[begin..]
                        .iter()
                        .filter_map(|(node, _)| {
                            workloads.get_index(*node).map(|e| e.0.name.as_str())
                        })
                        .collect();
                    if let Some(target_entry) = workloads.get_index(target) {
                        cycle.push(target_entry.0.name.as_str());
                    }
                    return Err(error(
                        &contexts[node],
                        "waitFor",
                        ValidationReason::DependencyCycle,
                        format!("dependency cycle: {}", cycle.join(" -> ")),
                    ));
                }
                if state[target] == 0 {
                    state[target] = 1;
                    stack.push((target, 0));
                }
            }
        }
        Ok(Self { wire, workloads })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{JobSpec, ProfileSpec, ServiceSpec};
    use serde_json::json;

    fn spec() -> AppSpec {
        let mut spec = AppSpec::default();
        spec.services.insert("web".into(), ServiceSpec::default());
        spec
    }
    #[test]
    fn checked_cache_matches_compatibility_ports_for_singular_list_and_profile() {
        let mut spec = spec();
        spec.services["web"].port = Some(1);
        spec.profiles.insert(
            "tools".into(),
            ProfileSpec {
                when: json!(false),
                services: IndexMap::from([(
                    "worker".into(),
                    ServiceSpec {
                        port: Some(80),
                        ports: vec![json!("9000:8080"), json!(65535)],
                        ..Default::default()
                    },
                )]),
                jobs: IndexMap::new(),
            },
        );
        let validated = ValidatedSpec::try_from(spec).unwrap();
        for (profile, name, expected) in [
            (None, "web", vec![CheckedPortSpec::single(1).unwrap()]),
            (
                Some("tools"),
                "worker",
                vec![
                    CheckedPortSpec::new(9000, 8080).unwrap(),
                    CheckedPortSpec::single(65535).unwrap(),
                ],
            ),
        ] {
            let workload = validated.workload(profile, name).unwrap();
            assert_eq!(workload.checked_ports(), expected);
            assert_eq!(
                workload.ports(),
                expected.into_iter().map(PortSpec::from).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn rejects_every_malformed_port_with_field_context() {
        for value in [
            json!(0),
            json!(-1),
            json!(65536),
            json!(2.5),
            json!(true),
            json!({}),
            json!("1:0"),
            json!("bad"),
        ] {
            let mut spec = spec();
            spec.services["web"].ports = vec![json!(8080), value];
            let error = ValidatedSpec::try_from(spec).unwrap_err().to_string();
            assert!(error.contains("Service 'web': invalid ports[1]"), "{error}");
        }
        let mut spec = spec();
        spec.services["web"].port = Some(0);
        spec.services["web"].ports = vec![json!(8080)];
        assert!(ValidatedSpec::try_from(spec)
            .unwrap_err()
            .to_string()
            .contains("invalid port:"));
    }
    #[test]
    fn parses_once_preserving_precedence_and_command_arguments() {
        let mut spec = spec();
        let service = &mut spec.services["web"];
        service.port = Some(80);
        service.ports = vec![json!("9000:8080")];
        service.command = Some(json!(["sh", "", "a b"]));
        service.mounts = vec!["cm:settings:/etc/settings:ro".into()];
        let validated = ValidatedSpec::try_from(&spec).unwrap();
        spec.services["web"].ports.clear();
        let workload = validated.workload(None, "web").unwrap();
        assert_eq!(workload.ports(), &[PortSpec::new(9000, 8080)]);
        assert_eq!(
            workload.command(),
            Some(&CommandSpec::List(vec![
                "sh".into(),
                "".into(),
                "a b".into()
            ]))
        );
        assert!(workload.mounts()[0].read_only);
    }
    #[test]
    fn rejects_command_shape_but_preserves_empty_commands() {
        for value in [json!(42), json!({}), json!(["echo", 2]), json!(true)] {
            let mut spec = spec();
            spec.services["web"].command = Some(value);
            assert!(ValidatedSpec::try_from(spec)
                .unwrap_err()
                .to_string()
                .contains("invalid command"));
        }
        for value in [json!(""), json!([])] {
            let mut spec = spec();
            spec.services["web"].command = Some(value);
            assert!(ValidatedSpec::try_from(spec).is_ok());
        }
    }
    #[test]
    fn validates_disabled_profiles_and_detects_name_collisions() {
        let mut spec = spec();
        let service = ServiceSpec {
            enabled: Some(false),
            ports: vec![json!(false)],
            ..Default::default()
        };
        spec.profiles.insert(
            "tools".into(),
            ProfileSpec {
                when: json!(false),
                services: IndexMap::from([("collector".into(), service)]),
                jobs: IndexMap::new(),
            },
        );
        assert!(ValidatedSpec::try_from(&spec)
            .unwrap_err()
            .to_string()
            .contains("Profile 'tools' Service 'collector'"));
        spec.profiles.clear();
        spec.jobs.insert(
            "web".into(),
            serde_json::from_value::<JobSpec>(json!({"image":"busybox"})).unwrap(),
        );
        assert!(ValidatedSpec::try_from(spec)
            .unwrap_err()
            .to_string()
            .contains("duplicates workload"));
    }
    #[test]
    fn validates_dependencies_and_keeps_external_tcp_waits() {
        let mut spec = spec();
        spec.services.insert("db".into(), ServiceSpec::default());
        spec.services["web"].wait_for = vec![
            "db:ready".into(),
            "db:5432".into(),
            "db.example.com:443".into(),
        ];
        let validated = ValidatedSpec::try_from(&spec).unwrap();
        let deps = validated.workload(None, "web").unwrap().dependencies();
        assert_eq!(deps[0].condition(), DependencyCondition::Ready);
        assert!(!deps[1].external());
        assert!(deps[2].external());
        for value in [
            "missing",
            "missing:healthy",
            "db:invalid",
            "db:0",
            "db:70000",
            "db:1:2",
            "host$(touch):443",
            ":443",
        ] {
            spec.services["web"].wait_for = vec![value.into()];
            assert!(
                ValidatedSpec::try_from(&spec)
                    .unwrap_err()
                    .to_string()
                    .contains("waitFor[0]"),
                "{value}"
            );
        }
    }
    #[test]
    fn detects_cross_kind_profile_cycles_deterministically() {
        let mut spec = spec();
        spec.services["web"].wait_for = vec!["worker:success".into()];
        spec.profiles.insert(
            "tools".into(),
            ProfileSpec {
                when: json!(true),
                services: IndexMap::new(),
                jobs: IndexMap::from([(
                    "worker".into(),
                    serde_json::from_value::<JobSpec>(
                        json!({"image":"busybox", "waitFor":["web:started"]}),
                    )
                    .unwrap(),
                )]),
            },
        );
        assert!(ValidatedSpec::try_from(spec)
            .unwrap_err()
            .to_string()
            .contains("web -> worker -> web"));
    }
    #[test]
    fn dotted_conditions_do_not_shadow_exact_dotted_workload_names() {
        let mut spec = spec();
        spec.services.insert("db".into(), ServiceSpec::default());
        spec.services
            .insert("db.healthy".into(), ServiceSpec::default());
        spec.services["web"].wait_for = vec![
            "db.ready".into(),
            "db.healthy".into(),
            "db.healthy.started".into(),
        ];
        let validated = ValidatedSpec::try_from(spec).unwrap();
        let deps = validated.workload(None, "web").unwrap().dependencies();
        assert_eq!(deps[0].target(), "db");
        assert_eq!(deps[0].condition(), DependencyCondition::Ready);
        assert_eq!(deps[1].target(), "db.healthy");
        assert_eq!(deps[1].condition(), DependencyCondition::Healthy);
        assert_eq!(deps[2].target(), "db.healthy");
        assert_eq!(deps[2].condition(), DependencyCondition::Started);
    }
    #[test]
    fn completed_condition_alias_maps_to_success() {
        let mut spec = spec();
        spec.services
            .insert("flyway".into(), ServiceSpec::default());
        spec.services
            .insert("worker".into(), ServiceSpec::default());
        spec.services["web"].wait_for = vec!["flyway.completed".into(), "worker:completed".into()];
        let validated = ValidatedSpec::try_from(spec).unwrap();
        let deps = validated.workload(None, "web").unwrap().dependencies();
        assert_eq!(deps[0].target(), "flyway");
        assert_eq!(deps[0].condition(), DependencyCondition::Success);
        assert_eq!(deps[1].target(), "worker");
        assert_eq!(deps[1].condition(), DependencyCondition::Success);
    }
    #[test]
    fn deep_dependency_graph_uses_iterative_traversal() {
        let mut spec = AppSpec::default();
        for i in 0..4000 {
            spec.services.insert(
                format!("service-{i}"),
                ServiceSpec {
                    wait_for: if i < 3999 {
                        vec![format!("service-{}:started", i + 1)]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
            );
        }
        assert!(ValidatedSpec::try_from(spec).is_ok());
    }
}

#[cfg(test)]
mod reason_tests {
    use super::*;
    use crate::model::ServiceSpec;
    use serde_json::json;
    #[test]
    fn checks_classify_without_rendered_text_and_hide_values() {
        for (field, value, reason, expected_field) in [
            (
                "ports",
                json!(["secret-marker"]),
                ValidationReason::InvalidPort,
                "ports[0]",
            ),
            (
                "command",
                json!({"secret-marker": 1}),
                ValidationReason::InvalidCommand,
                "command",
            ),
            (
                "mounts",
                json!(["secret-marker"]),
                ValidationReason::InvalidMount,
                "mounts[0]",
            ),
            (
                "waitFor",
                json!([":443"]),
                ValidationReason::MalformedDependency,
                "waitFor[0]",
            ),
            (
                "waitFor",
                json!(["db:secret-marker"]),
                ValidationReason::InvalidDependencyCondition,
                "waitFor[0]",
            ),
            (
                "waitFor",
                json!(["secret-marker"]),
                ValidationReason::UnknownDependency,
                "waitFor[0]",
            ),
            (
                "waitFor",
                json!(["secret-marker!:443"]),
                ValidationReason::InvalidExternalHost,
                "waitFor[0]",
            ),
        ] {
            let mut wire = AppSpec::default();
            let mut service = json!({});
            service[field] = value;
            wire.services
                .insert("web".into(), serde_json::from_value(service).unwrap());
            let error = ValidatedSpec::try_from(wire).unwrap_err();
            let cause = error.validation_error().unwrap();
            assert_eq!(cause.reason(), reason);
            assert_eq!(cause.field(), expected_field);
            assert_eq!(cause.workload(), Some("web"));
            assert!(!format!("{error} {error:?}").contains("secret-marker"));
        }
        let mut wire = AppSpec::default();
        wire.services
            .insert("../secret-marker".into(), ServiceSpec::default());
        let error = ValidatedSpec::try_from(wire).unwrap_err();
        assert_eq!(
            error.validation_error().unwrap().reason(),
            ValidationReason::InvalidIdentifier
        );
        assert!(!error.to_string().contains("secret-marker"));
    }
}

#[cfg(test)]
mod graph_reason_tests {
    use super::*;
    use crate::model::{JobSpec, ProfileSpec, ServiceSpec};
    #[test]
    fn duplicate_and_cycle_reasons_are_typed() {
        let mut wire = AppSpec::default();
        wire.services.insert("web".into(), ServiceSpec::default());
        wire.jobs.insert(
            "web".into(),
            serde_json::from_value::<JobSpec>(serde_json::json!({"image":"busybox"})).unwrap(),
        );
        assert_eq!(
            ValidatedSpec::try_from(wire)
                .unwrap_err()
                .validation_error()
                .unwrap()
                .reason(),
            ValidationReason::DuplicateWorkload
        );
        let mut wire = AppSpec::default();
        wire.services.insert(
            "web".into(),
            ServiceSpec {
                wait_for: vec!["web:ready".into()],
                ..Default::default()
            },
        );
        assert_eq!(
            ValidatedSpec::try_from(wire)
                .unwrap_err()
                .validation_error()
                .unwrap()
                .reason(),
            ValidationReason::DependencyCycle
        );
    }
    #[test]
    fn dependency_port_parse_sources_survive_validation() {
        use std::error::Error;
        for value in ["SECRET-CONDITION", "65536", "0"] {
            let mut wire = AppSpec::default();
            wire.services.insert(
                "web".into(),
                ServiceSpec {
                    wait_for: vec![format!("db:{value}")],
                    ..Default::default()
                },
            );
            let error = ValidatedSpec::try_from(wire).unwrap_err();
            let validation = error.validation_error().unwrap();
            assert_eq!(
                validation.reason(),
                ValidationReason::InvalidDependencyCondition
            );
            assert_eq!(validation.workload(), Some("web"));
            assert_eq!(validation.field(), "waitFor[0]");
            if value == "0" {
                assert!(validation.source().is_none());
            } else {
                assert!(validation
                    .source()
                    .unwrap()
                    .downcast_ref::<std::num::ParseIntError>()
                    .is_some());
            }
            assert!(!format!("{error} {error:?}").contains("SECRET-CONDITION"));
        }
    }
    #[test]
    fn workload_query_resolves_across_default_and_custom_profiles() {
        let mut wire = AppSpec::default();
        wire.services.insert("web".into(), ServiceSpec::default());
        let mut profile_spec = ProfileSpec {
            when: serde_json::Value::Null,
            services: IndexMap::new(),
            jobs: IndexMap::new(),
        };
        profile_spec
            .services
            .insert("worker".into(), ServiceSpec::default());
        wire.profiles.insert("prod".into(), profile_spec);

        let validated = ValidatedSpec::try_from(wire).unwrap();
        assert!(validated.workload(None, "web").is_some());
        assert!(validated.workload(Some("prod"), "worker").is_some());
        assert!(validated.workload(Some("dev"), "web").is_none());
        assert!(validated.workload(None, "nonexistent").is_none());
    }
}
