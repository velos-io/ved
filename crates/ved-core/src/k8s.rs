use crate::error::Result;
use std::borrow::Cow;
use std::collections::BTreeMap;
#[cfg(all(test, feature = "host"))]
use std::fs;
use std::path::Path;

use crate::model::{AppSpec, JobSpec, ServiceSpec};
#[cfg(test)]
use crate::model::{MountKind, MountSpec};

mod emission;
mod yaml;

#[cfg(test)]
use emission::format_mounts;
use emission::{build_job_manifest, build_service_manifest};
pub use yaml::{ManifestDocBuilder, YamlNode, YamlWriter};

pub fn resolve_group(grp: &str, _name: &str) -> String {
    let trimmed = grp.trim();
    if !trimmed.is_empty() && trimmed != "default" {
        trimmed.to_string()
    } else {
        "agent".to_string()
    }
}

/// Extensible resolver mapping workload groups to Kubernetes namespaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceResolver {
    slot: String,
    infra_ns: String,
    tools_ns: String,
    mappings: std::collections::HashMap<String, String>,
}

impl NamespaceResolver {
    pub fn slot(&self) -> &str {
        &self.slot
    }
    /// Create a resolver with the given deployment slot.
    pub fn new(slot: impl Into<String>) -> Self {
        Self {
            slot: slot.into(),
            infra_ns: "infra".to_string(),
            tools_ns: "tools".to_string(),
            mappings: std::collections::HashMap::new(),
        }
    }

    /// Construct a resolver from the workspace root and deployment slot.
    #[cfg(feature = "host")]
    pub fn from_root(root: &Path, slot: &str) -> Self {
        Self::from_context(&crate::workspace::WorkspaceContext::new(root), slot)
    }

    /// Resolve configured namespace aliases from a stable workspace snapshot.
    pub fn from_context(context: &crate::workspace::WorkspaceContext, slot: &str) -> Self {
        let infra_ns = context
            .config()
            .get_prop(crate::r::keys::K8S_INFRA_NAMESPACE, "infra");
        let tools_ns = context
            .config()
            .get_prop(crate::r::keys::K8S_TOOLS_NAMESPACE, "tools");
        Self {
            slot: slot.to_string(),
            infra_ns,
            tools_ns,
            mappings: std::collections::HashMap::new(),
        }
    }

    /// Add an explicit group-to-namespace mapping.
    pub fn with_mapping(mut self, group: impl Into<String>, namespace: impl Into<String>) -> Self {
        self.mappings.insert(group.into(), namespace.into());
        self
    }

    /// Resolve a group name to its corresponding Kubernetes namespace.
    pub fn resolve(&self, grp: &str) -> String {
        let normalized = resolve_group(grp, "");
        if let Some(target) = self.mappings.get(&normalized) {
            return target.clone();
        }
        match normalized.as_str() {
            "infra" => self.infra_ns.clone(),
            "tools" => self.tools_ns.clone(),
            "agent" => format!("ns-{}", self.slot),
            other => other.to_string(),
        }
    }

    /// Resolve namespace for a service specification.
    pub fn resolve_for_service(&self, svc: &ServiceSpec, name: &str) -> String {
        let grp = resolve_group(&svc.group, name);
        self.resolve(&grp)
    }

    /// Resolve namespace for a job specification.
    pub fn resolve_for_job(&self, job: &JobSpec, name: &str) -> String {
        let grp = resolve_group(&job.group, name);
        self.resolve(&grp)
    }

    /// Return the list of baseline known namespaces.
    pub fn known_namespaces(&self) -> Vec<String> {
        let mut list = vec![
            format!("ns-{}", self.slot),
            self.infra_ns.clone(),
            self.tools_ns.clone(),
        ];
        for ns in self.mappings.values() {
            if !list.contains(ns) {
                list.push(ns.clone());
            }
        }
        list
    }

    /// Discover and return all active namespaces for an AppSpec, preserving deterministic ordering.
    pub fn all_namespaces(&self, spec: &AppSpec) -> Vec<String> {
        let mut list = vec![
            self.infra_ns.clone(),
            self.tools_ns.clone(),
            format!("ns-{}", self.slot),
        ];

        for (name, svc) in &spec.services {
            if svc.is_enabled(name) {
                let ns = self.resolve_for_service(svc, name);
                if !list.contains(&ns) {
                    list.push(ns);
                }
            }
        }

        for (name, job) in &spec.jobs {
            if job.is_enabled(name) {
                let ns = self.resolve_for_job(job, name);
                if !list.contains(&ns) {
                    list.push(ns);
                }
            }
        }

        list
    }
}

/// Fully resolved Kubernetes destinations and their logical source groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubernetesRoutingPlan {
    resolver: NamespaceResolver,
    groups: BTreeMap<String, Vec<String>>,
    group_namespaces: BTreeMap<String, String>,
}

impl KubernetesRoutingPlan {
    pub fn build(spec: &AppSpec, resolver: NamespaceResolver) -> Result<Self> {
        let mut groups = BTreeMap::<String, Vec<String>>::new();
        let mut group_namespaces = BTreeMap::new();
        for group in ["infra", "tools", "agent"] {
            let ns = resolver.resolve(group);
            validate_namespace(&ns)?;
            groups
                .entry(ns.clone())
                .or_default()
                .push(group.to_string());
            group_namespaces.insert(group.to_string(), ns);
        }
        for (name, svc) in &spec.services {
            if svc.is_enabled(name) {
                let group = resolve_group(&svc.group, name);
                let ns = resolver.resolve(&group);
                validate_namespace(&ns)?;
                groups.entry(ns.clone()).or_default().push(group.clone());
                group_namespaces.insert(group, ns);
            }
        }
        for (name, job) in &spec.jobs {
            if job.is_enabled(name) {
                let group = resolve_group(&job.group, name);
                let ns = resolver.resolve(&group);
                validate_namespace(&ns)?;
                groups.entry(ns.clone()).or_default().push(group.clone());
                group_namespaces.insert(group, ns);
            }
        }
        for source_groups in groups.values_mut() {
            source_groups.sort();
            source_groups.dedup();
        }
        Ok(Self {
            resolver,
            groups,
            group_namespaces,
        })
    }
    pub fn from_context(
        spec: &AppSpec,
        context: &crate::workspace::WorkspaceContext,
        slot: &str,
    ) -> Result<Self> {
        Self::build(spec, NamespaceResolver::from_context(context, slot))
    }
    pub fn namespaces(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }
    pub fn groups_for_namespace(&self, namespace: &str) -> &[String] {
        self.groups.get(namespace).map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn namespace_for_service(&self, service: &ServiceSpec, name: &str) -> String {
        self.resolver.resolve_for_service(service, name)
    }
    pub fn namespace_for_job(&self, job: &JobSpec, name: &str) -> String {
        self.resolver.resolve_for_job(job, name)
    }
    pub fn fingerprint(&self) -> String {
        let mut value = format!("slot={}\n", self.resolver.slot());
        for (ns, groups) in &self.groups {
            value.push_str(&format!("{ns}:{}\n", groups.join(",")));
        }
        value
    }
}

fn validate_namespace(namespace: &str) -> Result<()> {
    if crate::model::is_valid_dns1123_label(namespace) {
        Ok(())
    } else {
        Err(crate::error::VedError::config(format!(
            "invalid Kubernetes namespace: {namespace}"
        )))
    }
}

fn resolve_cross_namespace_host<'a>(
    val_str: &'a str,
    caller_grp: &str,
    spec: &AppSpec,
) -> Cow<'a, str> {
    if val_str.contains('.') || val_str.contains(':') {
        return Cow::Borrowed(val_str);
    }
    if let Some(target_svc) = spec.services.get(val_str) {
        let target_grp = resolve_group(&target_svc.group, val_str);
        if target_grp != caller_grp {
            return Cow::Owned(format!("{val_str}.{target_grp}.svc.cluster.local"));
        }
    }
    Cow::Borrowed(val_str)
}

fn parse_http_probe(
    cmd: &str,
    default_port: u16,
    ports: &[(u16, u16)],
) -> (String, u16, &'static str) {
    if cmd.starts_with('/') {
        return (cmd.to_string(), default_port, "HTTP");
    }
    let (rest, scheme) = if let Some(stripped) = cmd.strip_prefix("https://") {
        (stripped, "HTTPS")
    } else if let Some(stripped) = cmd.strip_prefix("http://") {
        (stripped, "HTTP")
    } else {
        return (cmd.to_string(), default_port, "HTTP");
    };

    let (host_port, path) = if let Some(idx) = rest.find('/') {
        (&rest[..idx], &rest[idx..])
    } else {
        (rest, "/")
    };

    let port = if let Some(colon_idx) = host_port.find(':') {
        let parsed = host_port[colon_idx + 1..]
            .parse::<u16>()
            .unwrap_or(default_port);
        ports
            .iter()
            .find(|(hp, _)| *hp == parsed)
            .map(|(_, cp)| *cp)
            .unwrap_or(parsed)
    } else {
        default_port
    };

    (path.to_string(), port, scheme)
}

#[cfg(feature = "host")]
pub fn generate_secrets_yaml() -> Result<String> {
    let context = crate::workspace::WorkspaceContext::new(".");
    generate_secrets_yaml_with(|key, default| context.config().get_prop(key, default))
}

fn generate_secrets_yaml_with(mut lookup: impl FnMut(&str, &str) -> String) -> Result<String> {
    let db_pass = lookup("DB_PASSWORD", "");
    let pg_pass = lookup("POSTGRES_PASSWORD", &db_pass);
    let flyway_pass = lookup("FLYWAY_PASSWORD", &db_pass);
    let data = [
        ("DB_PASSWORD", db_pass),
        ("POSTGRES_PASSWORD", pg_pass),
        ("FLYWAY_PASSWORD", flyway_pass),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| (key.to_string(), YamlNode::scalar(value)))
    .collect();
    ManifestDocBuilder::new("v1", "Secret", "app-secrets")
        .with_field("type", YamlNode::scalar("Opaque"))
        .with_field("stringData", YamlNode::Mapping(data))
        .render()
}

const INGRESS_NGINX_YAML: &str = r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: ingress-nginx-controller
  labels:
    app.kubernetes.io/name: ingress-nginx
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: ingress-nginx
  template:
    metadata:
      labels:
        app.kubernetes.io/name: ingress-nginx
    spec:
      containers:
      - name: controller
        image: registry.k8s.io/ingress-nginx/controller:v1.11.2
        ports:
        - name: http
          containerPort: 80
        - name: https
          containerPort: 443
---
apiVersion: v1
kind: Service
metadata:
  name: ingress-nginx-controller
spec:
  type: NodePort
  selector:
    app.kubernetes.io/name: ingress-nginx
  ports:
  - name: http
    port: 80
    targetPort: 80
  - name: https
    port: 443
    targetPort: 443
"#;

fn render_documents(docs: &[YamlNode]) -> Result<String> {
    let mut writer = YamlWriter::new();
    for doc in docs {
        writer.write_doc(doc)?;
    }
    if docs.is_empty() {
        writer.write_doc(&YamlNode::EmptyMapping)?;
    }
    Ok(writer.into_string())
}

fn write_yaml(
    output: &mut BTreeMap<std::path::PathBuf, Vec<u8>>,
    path: impl AsRef<Path>,
    text: &str,
) -> Result<()> {
    use serde::Deserialize;
    let mut document_count = 0;
    for doc in serde_yaml::Deserializer::from_str(text) {
        document_count += 1;
        let value = serde_yaml::Value::deserialize(doc)?;
        if !value.is_mapping() {
            return Err(crate::error::VedError::emission(format!(
                "Kubernetes document in '{}' must be a mapping",
                path.as_ref().display()
            )));
        }
    }
    if document_count == 0 {
        return Err(crate::error::VedError::emission(
            "Empty Kubernetes document stream",
        ));
    }
    output.insert(path.as_ref().to_path_buf(), text.as_bytes().to_vec());
    Ok(())
}

fn kustomization(resources: Vec<String>, namespace: Option<&str>, patches: &[&str]) -> YamlNode {
    let mut fields = vec![
        (
            "apiVersion",
            YamlNode::scalar("kustomize.config.k8s.io/v1beta1"),
        ),
        ("kind", YamlNode::scalar("Kustomization")),
        (
            "resources",
            YamlNode::sequence(resources.into_iter().map(YamlNode::scalar).collect()),
        ),
    ];
    if let Some(namespace) = namespace {
        fields.push(("namespace", YamlNode::scalar(namespace)));
    }
    if !patches.is_empty() {
        fields.push((
            "patches",
            YamlNode::sequence(
                patches
                    .iter()
                    .map(|path| YamlNode::mapping(vec![("path", YamlNode::scalar(*path))]))
                    .collect(),
            ),
        ));
    }
    YamlNode::mapping(fields)
}

fn security_patch(name: &str, kind: &str) -> YamlNode {
    let pod = YamlNode::mapping(vec![
        (
            "securityContext",
            YamlNode::mapping(vec![
                ("runAsNonRoot", YamlNode::boolean(false)),
                ("runAsUser", YamlNode::integer(0)),
                ("runAsGroup", YamlNode::integer(0)),
                ("fsGroup", YamlNode::integer(0)),
            ]),
        ),
        (
            "containers",
            YamlNode::sequence(vec![YamlNode::mapping(vec![
                ("name", YamlNode::scalar(name)),
                (
                    "securityContext",
                    YamlNode::mapping(vec![
                        ("allowPrivilegeEscalation", YamlNode::boolean(true)),
                        ("readOnlyRootFilesystem", YamlNode::boolean(false)),
                    ]),
                ),
            ])]),
        ),
    ]);
    let template = YamlNode::mapping(vec![("template", YamlNode::mapping(vec![("spec", pod)]))]);
    let spec = if kind == "CronJob" {
        YamlNode::mapping(vec![(
            "jobTemplate",
            YamlNode::mapping(vec![("spec", template)]),
        )])
    } else {
        template
    };
    ManifestDocBuilder::new(
        if kind == "Deployment" {
            "apps/v1"
        } else {
            "batch/v1"
        },
        kind,
        name,
    )
    .with_field("spec", spec)
    .to_node()
}

fn ingress_patch(name: &str, port: u16, host: String) -> YamlNode {
    let service = YamlNode::mapping(vec![
        ("name", YamlNode::scalar(name)),
        (
            "port",
            YamlNode::mapping(vec![("number", YamlNode::integer(port))]),
        ),
    ]);
    let path = YamlNode::mapping(vec![
        ("path", YamlNode::scalar("/")),
        ("pathType", YamlNode::scalar("Prefix")),
        ("backend", YamlNode::mapping(vec![("service", service)])),
    ]);
    let rule = YamlNode::mapping(vec![
        ("host", YamlNode::scalar(host)),
        (
            "http",
            YamlNode::mapping(vec![("paths", YamlNode::sequence(vec![path]))]),
        ),
    ]);
    ManifestDocBuilder::new("networking.k8s.io/v1", "Ingress", name)
        .with_field(
            "spec",
            YamlNode::mapping(vec![("rules", YamlNode::sequence(vec![rule]))]),
        )
        .to_node()
}

#[cfg(feature = "host")]
pub fn generate_k8s(spec: &AppSpec, out_dir: &Path) -> Result<()> {
    let validated = crate::validated::ValidatedSpec::try_from(spec)?;
    generate_k8s_validated(&validated, out_dir)
}

/// Emit from an immutable validated snapshot, reusing parsed workload fields.
#[cfg(feature = "host")]
pub fn generate_k8s_validated(
    validated: &crate::validated::ValidatedSpec,
    out_dir: &Path,
) -> Result<()> {
    generate_k8s_validated_with_context(
        validated,
        &crate::workspace::WorkspaceContext::new("."),
        out_dir,
    )
}

/// Emit Kubernetes using one resolved workspace configuration snapshot.
#[cfg(feature = "host")]
pub fn generate_k8s_validated_with_context(
    validated: &crate::validated::ValidatedSpec,
    context: &crate::workspace::WorkspaceContext,
    out_dir: &Path,
) -> Result<()> {
    let slot = context.config().get_prop(crate::r::keys::K8S_APP_SLOT, "1");
    let plan = KubernetesRoutingPlan::from_context(validated.as_spec(), context, &slot)?;
    generate_k8s_validated_with_plan(validated, context, out_dir, &plan)
}

#[cfg(feature = "host")]
pub fn generate_k8s_validated_with_plan(
    validated: &crate::validated::ValidatedSpec,
    context: &crate::workspace::WorkspaceContext,
    out_dir: &Path,
    plan: &KubernetesRoutingPlan,
) -> Result<()> {
    let files = render_k8s_validated_with_plan(validated, context, plan)?;
    let output = crate::filesystem::ConfinedDir::create(out_dir)?;
    output.preflight_tree("")?;
    // Preserve the legacy directory layout, including empty overlay directories.
    for path in ["base/infra", "base/tools", "base/agent", "overlays/dev"] {
        output.create_dir_all(path)?;
    }
    for (path, bytes) in files {
        if let Some(parent) = path.parent() {
            output.create_dir_all(parent)?;
        }
        output.write(path, &bytes)?;
    }
    Ok(())
}

/// Render relative Kubernetes artifacts in memory using explicit routing and configuration.
pub fn render_k8s_validated_with_plan(
    validated: &crate::validated::ValidatedSpec,
    context: &crate::workspace::WorkspaceContext,
    plan: &KubernetesRoutingPlan,
) -> Result<BTreeMap<std::path::PathBuf, Vec<u8>>> {
    let prepared = validated.with_context(context);
    let validated = &prepared;
    let spec = validated.as_spec();
    context.validate_absolute_paths()?;
    let mut output = BTreeMap::new();
    let ingress_suffix = context
        .config()
        .resolve_ingress_suffix(spec.ingress.as_deref());
    let secrets_yaml =
        generate_secrets_yaml_with(|key, default| context.config().get_prop(key, default))?;
    write_yaml(&mut output, "base/infra/secrets.yaml", &secrets_yaml)?;
    write_yaml(&mut output, "base/agent/secrets.yaml", &secrets_yaml)?;
    write_yaml(
        &mut output,
        "base/tools/ingress-nginx.yaml",
        INGRESS_NGINX_YAML,
    )?;
    let mut group_resources: BTreeMap<String, Vec<String>> = BTreeMap::from([
        ("infra".into(), vec!["secrets.yaml".into()]),
        ("agent".into(), vec!["secrets.yaml".into()]),
        ("tools".into(), vec!["ingress-nginx.yaml".into()]),
    ]);
    let mut ingress_patches = Vec::new();
    let mut infra_sec_patches = Vec::new();
    let mut agent_sec_patches = Vec::new();
    for (name, svc) in &spec.services {
        if !svc.enabled.unwrap_or(true) {
            continue;
        }
        let group = resolve_group(&svc.group, name);
        write_yaml(
            &mut output,
            format!("base/{group}/{name}.yaml"),
            &build_service_manifest(name, svc, &ingress_suffix, validated)?,
        )?;
        group_resources
            .entry(group.clone())
            .or_default()
            .push(format!("{name}.yaml"));
        if group == "infra" {
            infra_sec_patches.push(security_patch(name, "Deployment"));
        } else if group == "agent" {
            agent_sec_patches.push(security_patch(name, "Deployment"));
        }
        if svc.ingress.is_some() && group == "agent" {
            let port = validated
                .workload(None, name)
                .expect("validated service")
                .ports()
                .first()
                .map(|p| p.host())
                .unwrap_or(8080);
            ingress_patches.push(ingress_patch(name, port, format!("app1.{ingress_suffix}")));
        }
    }
    for (name, job) in &spec.jobs {
        if !job.enabled.unwrap_or(true) {
            continue;
        }
        let group = resolve_group(&job.group, name);
        write_yaml(
            &mut output,
            format!("base/{group}/{name}.yaml"),
            &build_job_manifest(name, job, validated)?,
        )?;
        group_resources
            .entry(group.clone())
            .or_default()
            .push(format!("{name}.yaml"));
        if group == "infra" {
            infra_sec_patches.push(security_patch(
                name,
                if job.schedule.is_some() {
                    "CronJob"
                } else {
                    "Job"
                },
            ));
        }
    }
    for (group, resources) in &group_resources {
        write_yaml(
            &mut output,
            format!("base/{group}/kustomization.yaml"),
            &render_documents(&[kustomization(resources.clone(), None, &[])])?,
        )?;
    }
    // Local group bases need their own secret; combined base/dev use it once.
    let mut root_resources = Vec::new();
    let mut secret_included = false;
    for (group, resources) in &group_resources {
        for resource in resources {
            if resource == "secrets.yaml" {
                if secret_included {
                    continue;
                }
                secret_included = true;
            }
            root_resources.push(format!("{group}/{resource}"));
        }
    }
    write_yaml(
        &mut output,
        "base/kustomization.yaml",
        &render_documents(&[kustomization(root_resources, None, &[])])?,
    )?;
    let mut patches_by_group = BTreeMap::<String, Vec<YamlNode>>::new();
    patches_by_group.insert("infra".into(), infra_sec_patches);
    patches_by_group.insert("agent".into(), agent_sec_patches);
    patches_by_group.insert("__ingress".into(), ingress_patches);
    for namespace in plan.namespaces() {
        let groups = plan.groups_for_namespace(&namespace);
        let resources = groups
            .iter()
            .map(|group| format!("../../../base/{group}"))
            .collect();
        let mut patch_entries = BTreeMap::<&str, Vec<YamlNode>>::new();
        for group in groups {
            if let Some(docs) = patches_by_group.get(group) {
                if !docs.is_empty() {
                    patch_entries
                        .entry("local-security-patch.yaml")
                        .or_default()
                        .extend(docs.clone());
                }
            }
        }
        if groups.contains(&"agent".to_string()) {
            if let Some(docs) = patches_by_group.get("__ingress") {
                if !docs.is_empty() {
                    patch_entries
                        .entry("ingress-patch.yaml")
                        .or_default()
                        .extend(docs.clone());
                }
            }
        }
        let patch_names: Vec<&str> = patch_entries.keys().copied().collect();
        let path = format!("overlays/local/{namespace}");
        write_yaml(
            &mut output,
            format!("{path}/kustomization.yaml"),
            &render_documents(&[kustomization(resources, Some(&namespace), &patch_names)])?,
        )?;
        for (name, docs) in patch_entries {
            write_yaml(
                &mut output,
                format!("{path}/{name}"),
                &render_documents(&docs)?,
            )?;
        }
    }
    write_yaml(
        &mut output,
        "overlays/dev/kustomization.yaml",
        &render_documents(&[kustomization(vec!["../../base".into()], Some("dev"), &[])])?,
    )?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml_document(text: &str, kind: &str) -> serde_yaml::Value {
        use serde::Deserialize;
        serde_yaml::Deserializer::from_str(text)
            .map(|doc| serde_yaml::Value::deserialize(doc).unwrap())
            .find(|doc| doc["kind"].as_str() == Some(kind))
            .expect("expected resource kind")
    }

    #[test]
    fn secret_strings_round_trip_without_ambient_configuration() {
        for value in [
            "a\"b",
            "back\\slash",
            "line\nnext",
            "key: value",
            "# hash",
            "日本語🦀",
            "true",
            "null",
            "001",
            "2026-10-10",
            "\0\t\r",
            "---\nkind: Injected",
        ] {
            let text = generate_secrets_yaml_with(|key, fallback| {
                if key == "DB_PASSWORD" {
                    value.to_string()
                } else {
                    fallback.to_string()
                }
            })
            .unwrap();
            let secret = yaml_document(&text, "Secret");
            for key in ["DB_PASSWORD", "POSTGRES_PASSWORD", "FLYWAY_PASSWORD"] {
                assert_eq!(secret["stringData"][key].as_str(), Some(value));
            }
        }
        let text = generate_secrets_yaml_with(|_, _| String::new()).unwrap();
        assert_eq!(
            yaml_document(&text, "Secret")["stringData"],
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        );
    }

    fn mount_yaml(mounts: &[MountSpec]) -> (String, String, String) {
        let (mounts, volumes, claims) = format_mounts(mounts);
        (
            render_documents(&[YamlNode::sequence(mounts)]).unwrap(),
            render_documents(&[YamlNode::sequence(volumes)]).unwrap(),
            render_documents(&claims).unwrap(),
        )
    }

    #[test]
    fn test_format_mounts_pvc() {
        let mount = MountSpec::parse("postgres-data:/var/lib/postgresql/data").unwrap();
        assert_eq!(
            mount.kind,
            MountKind::PersistentVolumeClaim {
                claim_name: "postgres-data".to_string(),
                mount_path: "/var/lib/postgresql/data".to_string(),
                read_only: false,
            }
        );
        let (vm, vol, pvc) = mount_yaml(&[mount]);
        assert!(vm.contains("name: postgres-data"));
        assert!(vm.contains("mountPath: /var/lib/postgresql/data"));
        assert!(vol.contains("claimName: postgres-data"));
        assert!(pvc.contains("kind: PersistentVolumeClaim"));
    }

    #[test]
    fn test_format_mounts_hostpath_ro() {
        let mount = MountSpec::parse("./config/app.json:/etc/app/config.json:ro").unwrap();
        assert_eq!(
            mount.kind,
            MountKind::HostPath {
                volume_name: "vol-config-app-json".to_string(),
                host_path: "./config/app.json".to_string(),
                mount_path: "/etc/app/config.json".to_string(),
                read_only: true,
            }
        );
        let (vm, vol, _) = mount_yaml(&[mount]);
        assert!(vm.contains("name: vol-config-app-json"));
        assert!(vm.contains("readOnly: true"));
        assert!(vol.contains("emptyDir: {}"));
    }

    #[test]
    fn test_format_mounts_hostpath_bind() {
        let mount = MountSpec::parse("./data/exports:/exports").unwrap();
        assert_eq!(
            mount.kind,
            MountKind::HostPath {
                volume_name: "vol-data-exports".to_string(),
                host_path: "./data/exports".to_string(),
                mount_path: "/exports".to_string(),
                read_only: false,
            }
        );
        let (vm, vol, _) = mount_yaml(&[mount]);
        assert!(vm.contains("name: vol-data-exports"));
        assert!(vol.contains("emptyDir: {}"));
    }

    #[test]
    fn test_format_mounts_configmap() {
        let mount = MountSpec::parse("cm:app-config:/etc/config:ro").unwrap();
        assert_eq!(
            mount.kind,
            MountKind::ConfigMap {
                config_map_name: "app-config".to_string(),
                mount_path: "/etc/config".to_string(),
                read_only: true,
            }
        );
        let (vm, vol, _) = mount_yaml(&[mount]);
        assert!(vm.contains("name: app-config"));
        assert!(vm.contains("mountPath: /etc/config"));
        assert!(vm.contains("readOnly: true"));
        assert_eq!(
            serde_yaml::from_str::<serde_yaml::Value>(&vol).unwrap()[0]["configMap"]["name"]
                .as_str(),
            Some("app-config")
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_k8s_kustomize_infra_overlay_without_flyway() {
        let spec = AppSpec {
            app: "test-no-flyway".to_string(),
            version: "0.1.0".to_string(),
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join(format!("k8s_test_no_flyway_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        generate_k8s(&spec, &tmp).expect("Failed to generate k8s");

        assert!(!tmp.join("base/infra/flyway.yaml").exists());
        assert!(!tmp.join("base/infra/flyway-demo.yaml").exists());

        let kustomize_res = std::process::Command::new(crate::r::bins::KUBECTL)
            .arg("kustomize")
            .arg(tmp.join("overlays/local/infra"))
            .output();
        if let Ok(out) = kustomize_res {
            assert!(
                out.status.success(),
                "kubectl kustomize failed for infra overlay: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_k8s_kustomize_infra_overlay_with_flyway_service() {
        use crate::model::ServiceSpec;
        use indexmap::IndexMap;

        let mut services = IndexMap::new();
        services.insert(
            "flyway".to_string(),
            ServiceSpec {
                image: Some("flyway/flyway:13.3".to_string()),
                group: "infra".to_string(),
                ..Default::default()
            },
        );

        let spec = AppSpec {
            app: "test-with-flyway".to_string(),
            version: "0.1.0".to_string(),
            services,
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join(format!("k8s_test_with_flyway_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        generate_k8s(&spec, &tmp).expect("Failed to generate k8s");

        assert!(tmp.join("base/infra/flyway.yaml").exists());
        let sec_patch =
            fs::read_to_string(tmp.join("overlays/local/infra/local-security-patch.yaml")).unwrap();
        assert!(sec_patch.contains("flyway"));

        let kustomize_res = std::process::Command::new(crate::r::bins::KUBECTL)
            .arg("kustomize")
            .arg(tmp.join("overlays/local/infra"))
            .output();
        if let Ok(out) = kustomize_res {
            assert!(
                out.status.success(),
                "kubectl kustomize failed for infra overlay with flyway: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_dynamic_workload_generation() {
        use crate::model::{JobSpec, ServiceSpec};
        use indexmap::IndexMap;

        let mut services = IndexMap::new();
        services.insert(
            "custom-api".to_string(),
            ServiceSpec {
                image: Some("myorg/custom-api:2.0".to_string()),
                port: Some(9090),
                ingress: Some("custom".to_string()),
                ..Default::default()
            },
        );
        let mut jobs = IndexMap::new();
        jobs.insert(
            "nightly-sync".to_string(),
            JobSpec {
                image: "myorg/sync:1.0".to_string(),
                schedule: Some("0 1 * * *".to_string()),
                group: "default".to_string(),
                entrypoint: None,
                command: Some(serde_json::json!(["./sync.sh"])),
                hook: None,
                mounts: vec![],
                env: IndexMap::new(),
                wait_for: vec![],
                restart: None,
                when: None,
                enabled: None,
                ..Default::default()
            },
        );

        let spec = AppSpec {
            app: "dynamic-k8s-app".to_string(),
            version: "1.0.0".to_string(),
            services,
            jobs,
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join(format!("k8s_test_dynamic_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        generate_k8s(&spec, &tmp).expect("Failed to generate dynamic k8s manifests");

        let api_yaml = fs::read_to_string(tmp.join("base/agent/custom-api.yaml")).unwrap();
        assert!(api_yaml.contains("image: myorg/custom-api:2.0"));
        assert!(api_yaml.contains("port: 9090"));
        assert!(api_yaml.contains("host: custom.app.localapp"));

        let cron_yaml = fs::read_to_string(tmp.join("base/agent/nightly-sync.yaml")).unwrap();
        assert!(cron_yaml.contains("kind: CronJob"));
        assert_eq!(
            yaml_document(&cron_yaml, "CronJob")["spec"]["schedule"].as_str(),
            Some("0 1 * * *")
        );
        assert!(cron_yaml.contains("image: myorg/sync:1.0"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_generate_secrets_yaml_dynamic() {
        let secrets_yaml = generate_secrets_yaml_with(|key, default| match key {
            "DB_PASSWORD" => "super_secret_db_pass".into(),
            "POSTGRES_PASSWORD" => "custom_pg_pass".into(),
            _ => default.into(),
        })
        .unwrap();
        assert!(secrets_yaml.contains("kind: Secret"));
        assert!(secrets_yaml.contains("DB_PASSWORD: super_secret_db_pass"));
        assert!(secrets_yaml.contains("POSTGRES_PASSWORD: custom_pg_pass"));
    }

    #[test]
    fn test_resolve_group_decoupled() {
        assert_eq!(resolve_group("infra", "custom-db"), "infra");
        assert_eq!(resolve_group("tools", "collector"), "tools");
        assert_eq!(resolve_group("analytics", "spark"), "analytics");
        assert_eq!(resolve_group("  infra  ", "db"), "infra");
        assert_eq!(resolve_group("", "custom-db"), "agent");
        assert_eq!(resolve_group("default", "custom-db"), "agent");

        assert_eq!(resolve_group("", "postgres"), "agent");
        assert_eq!(resolve_group("", "redis"), "agent");
        assert_eq!(resolve_group("", "mysql"), "agent");
        assert_eq!(resolve_group("", "flyway"), "agent");
        assert_eq!(resolve_group("", "otel-collector"), "agent");
        assert_eq!(resolve_group("default", "postgres"), "agent");
    }

    #[test]
    fn test_namespace_resolver() {
        let resolver = NamespaceResolver::new("2").with_mapping("database", "db-prod");

        assert_eq!(resolver.resolve("infra"), "infra");
        assert_eq!(resolver.resolve("tools"), "tools");
        assert_eq!(resolver.resolve("agent"), "ns-2");
        assert_eq!(resolver.resolve(""), "ns-2");
        assert_eq!(resolver.resolve("default"), "ns-2");
        assert_eq!(resolver.resolve("database"), "db-prod");
        assert_eq!(resolver.resolve("analytics"), "analytics");

        let mut spec = AppSpec {
            app: "test-app".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };
        spec.services.insert(
            "web".to_string(),
            ServiceSpec {
                image: Some("nginx".to_string()),
                group: "agent".to_string(),
                ..Default::default()
            },
        );
        spec.services.insert(
            "db".to_string(),
            ServiceSpec {
                image: Some("postgres".to_string()),
                group: "database".to_string(),
                ..Default::default()
            },
        );
        spec.services.insert(
            "spark".to_string(),
            ServiceSpec {
                image: Some("spark".to_string()),
                group: "analytics".to_string(),
                ..Default::default()
            },
        );

        let all = resolver.all_namespaces(&spec);
        assert_eq!(all, vec!["infra", "tools", "ns-2", "db-prod", "analytics"]);
    }

    #[test]
    fn routing_plan_supports_slots_renames_and_shared_namespaces() {
        use crate::model::ServiceSpec;
        use indexmap::IndexMap;
        let mut services = IndexMap::new();
        services.insert(
            "api".into(),
            ServiceSpec {
                group: "agent".into(),
                ..Default::default()
            },
        );
        services.insert(
            "worker".into(),
            ServiceSpec {
                group: "batch".into(),
                ..Default::default()
            },
        );
        let spec = AppSpec {
            services,
            ..Default::default()
        };
        let resolver = NamespaceResolver::new("2")
            .with_mapping("infra", "platform")
            .with_mapping("tools", "platform")
            .with_mapping("agent", "shared")
            .with_mapping("batch", "shared");
        let plan = KubernetesRoutingPlan::build(&spec, resolver).unwrap();
        assert_eq!(plan.namespaces(), vec!["platform", "shared"]);
        assert_eq!(plan.groups_for_namespace("shared"), &["agent", "batch"]);
        assert!(plan.fingerprint().contains("slot=2"));
        assert_eq!(
            plan.namespace_for_service(&spec.services["api"], "api"),
            "shared"
        );
    }

    #[test]
    fn invalid_routing_namespace_fails_before_output_creation() {
        let spec = AppSpec::default();
        let plan =
            KubernetesRoutingPlan::build(&spec, NamespaceResolver::new("bad_slot!")).unwrap_err();
        assert!(plan.to_string().contains("invalid Kubernetes namespace"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_cross_namespace_host_resolution() {
        use crate::model::{JobSpec, ServiceSpec};
        use indexmap::IndexMap;

        let mut services = IndexMap::new();
        services.insert(
            "postgres".to_string(),
            ServiceSpec {
                image: Some("postgres:16".to_string()),
                group: "infra".to_string(),
                port: Some(5432),
                ..Default::default()
            },
        );
        services.insert(
            "cache".to_string(),
            ServiceSpec {
                image: Some("redis:7".to_string()),
                group: "infra".to_string(),
                port: Some(6379),
                ..Default::default()
            },
        );
        services.insert(
            "internal-auth".to_string(),
            ServiceSpec {
                image: Some("auth:latest".to_string()),
                group: "agent".to_string(),
                port: Some(8081),
                ..Default::default()
            },
        );

        let mut api_env = IndexMap::new();
        api_env.insert("DB_HOST".to_string(), serde_json::json!("postgres"));
        api_env.insert("REDIS_HOST".to_string(), serde_json::json!("cache"));
        api_env.insert("AUTH_HOST".to_string(), serde_json::json!("internal-auth"));
        api_env.insert(
            "CLOUD_DB".to_string(),
            serde_json::json!("db.us-east-1.rds.amazonaws.com"),
        );
        api_env.insert(
            "LOCAL_HOST".to_string(),
            serde_json::json!("localhost:5432"),
        );
        api_env.insert("IP_TARGET".to_string(), serde_json::json!("10.0.1.50"));

        services.insert(
            "api".to_string(),
            ServiceSpec {
                image: Some("api:1.0".to_string()),
                group: "agent".to_string(),
                port: Some(8080),
                env: api_env,
                ..Default::default()
            },
        );

        let mut migration_env = IndexMap::new();
        migration_env.insert("DB_HOST".to_string(), serde_json::json!("postgres"));
        migration_env.insert("AUTH_SVC".to_string(), serde_json::json!("internal-auth"));

        let mut jobs = IndexMap::new();
        jobs.insert(
            "migrate".to_string(),
            JobSpec {
                image: "migration:latest".to_string(),
                group: "agent".to_string(),
                env: migration_env,
                entrypoint: None,
                command: Some(serde_json::json!(["run", "migrations"])),
                hook: None,
                schedule: None,
                mounts: vec![],
                wait_for: vec![],
                restart: None,
                when: None,
                enabled: None,
                ..Default::default()
            },
        );

        let spec = AppSpec {
            app: "cross-ns-test".to_string(),
            version: "1.0.0".to_string(),
            services,
            jobs,
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join(format!("k8s_test_cross_ns_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        generate_k8s(&spec, &tmp).expect("generate_k8s");

        let api_manifest = fs::read_to_string(tmp.join("base/agent/api.yaml")).unwrap();
        assert_eq!(
            yaml_document(&api_manifest, "ConfigMap")["data"]["DB_HOST"].as_str(),
            Some("postgres.infra.svc.cluster.local")
        );
        assert_eq!(
            yaml_document(&api_manifest, "ConfigMap")["data"]["REDIS_HOST"].as_str(),
            Some("cache.infra.svc.cluster.local")
        );
        assert_eq!(
            yaml_document(&api_manifest, "ConfigMap")["data"]["AUTH_HOST"].as_str(),
            Some("internal-auth")
        );
        assert_eq!(
            yaml_document(&api_manifest, "ConfigMap")["data"]["CLOUD_DB"].as_str(),
            Some("db.us-east-1.rds.amazonaws.com")
        );
        assert_eq!(
            yaml_document(&api_manifest, "ConfigMap")["data"]["LOCAL_HOST"].as_str(),
            Some("localhost:5432")
        );
        assert_eq!(
            yaml_document(&api_manifest, "ConfigMap")["data"]["IP_TARGET"].as_str(),
            Some("10.0.1.50")
        );

        let job_manifest = fs::read_to_string(tmp.join("base/agent/migrate.yaml")).unwrap();
        assert_eq!(
            yaml_document(&job_manifest, "ConfigMap")["data"]["DB_HOST"].as_str(),
            Some("postgres.infra.svc.cluster.local")
        );
        assert_eq!(
            yaml_document(&job_manifest, "ConfigMap")["data"]["AUTH_SVC"].as_str(),
            Some("internal-auth")
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_dep_port_resolution_and_port_mappings() {
        use crate::model::ServiceSpec;
        use indexmap::IndexMap;

        let mut services = IndexMap::new();
        services.insert(
            "mapped-svc".to_string(),
            ServiceSpec {
                image: Some("nginx".to_string()),
                group: "tools".to_string(),
                ports: vec![serde_json::json!("9000:90")],
                ..Default::default()
            },
        );
        services.insert(
            "scalar-svc".to_string(),
            ServiceSpec {
                image: Some("redis".to_string()),
                group: "infra".to_string(),
                port: Some(6380),
                ..Default::default()
            },
        );
        services.insert(
            "no-port-svc".to_string(),
            ServiceSpec {
                image: Some("worker".to_string()),
                group: "agent".to_string(),
                ..Default::default()
            },
        );

        services.insert(
            "caller".to_string(),
            ServiceSpec {
                image: Some("app".to_string()),
                group: "agent".to_string(),
                wait_for: vec![
                    "mapped-svc".to_string(),
                    "scalar-svc".to_string(),
                    "no-port-svc".to_string(),
                    "mapped-svc:7777".to_string(),
                ],
                ..Default::default()
            },
        );

        let spec = AppSpec {
            app: "dep-port-test".to_string(),
            version: "1.0.0".to_string(),
            services,
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join(format!("k8s_test_dep_ports_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        generate_k8s(&spec, &tmp).expect("generate_k8s");

        let caller_manifest = fs::read_to_string(tmp.join("base/agent/caller.yaml")).unwrap();
        assert!(caller_manifest
            .contains("until nc -z mapped-svc.tools.svc.cluster.local 90; do sleep 2; done"));
        assert!(caller_manifest
            .contains("until nc -z scalar-svc.infra.svc.cluster.local 6380; do sleep 2; done"));
        assert!(caller_manifest.contains("until nc -z no-port-svc 8080; do sleep 2; done"));
        assert!(caller_manifest
            .contains("until nc -z mapped-svc.tools.svc.cluster.local 7777; do sleep 2; done"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_job_command_invariance() {
        use crate::model::JobSpec;
        use indexmap::IndexMap;

        let mut jobs = IndexMap::new();
        jobs.insert(
            "db-migration".to_string(),
            JobSpec {
                image: "flyway/flyway:10.0".to_string(),
                group: "infra".to_string(),
                entrypoint: None,
                command: Some(serde_json::json!([
                    "migrate",
                    "-url=jdbc:postgresql://localhost/db"
                ])),
                hook: None,
                schedule: None,
                mounts: vec![],
                env: IndexMap::new(),
                wait_for: vec![],
                restart: None,
                when: None,
                ..Default::default()
            },
        );

        let spec = AppSpec {
            app: "flyway-cmd-test".to_string(),
            version: "1.0.0".to_string(),
            jobs,
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join(format!("k8s_test_flyway_cmd_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        generate_k8s(&spec, &tmp).expect("generate_k8s");

        let job_yaml = fs::read_to_string(tmp.join("base/infra/db-migration.yaml")).unwrap();
        let job = yaml_document(&job_yaml, "Job");
        let command = &job["spec"]["template"]["spec"]["containers"][0]["command"];
        assert_eq!(
            command,
            &serde_yaml::to_value(vec!["migrate", "-url=jdbc:postgresql://localhost/db"]).unwrap()
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "host")]
    #[test]
    fn test_service_entrypoint_and_args_k8s() {
        let mut services = indexmap::IndexMap::new();
        services.insert(
            "custom-app".to_string(),
            ServiceSpec {
                image: Some("custom:1.0".to_string()),
                group: "agent".to_string(),
                entrypoint: Some(serde_json::json!(["/docker-entrypoint.sh", "--mode=prod"])),
                command: Some(serde_json::json!(["run", "server"])),
                ..Default::default()
            },
        );

        let spec = AppSpec {
            app: "entrypoint-k8s-test".to_string(),
            version: "1.0.0".to_string(),
            services,
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join(format!("k8s_test_ep_cmd_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        generate_k8s(&spec, &tmp).expect("generate_k8s");

        let dep_yaml = fs::read_to_string(tmp.join("base/agent/custom-app.yaml")).unwrap();
        let dep = yaml_document(&dep_yaml, "Deployment");
        let container = &dep["spec"]["template"]["spec"]["containers"][0];
        assert_eq!(
            container["command"],
            serde_yaml::to_value(vec!["/docker-entrypoint.sh", "--mode=prod"]).unwrap()
        );
        assert_eq!(
            container["args"],
            serde_yaml::to_value(vec!["run", "server"]).unwrap()
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
