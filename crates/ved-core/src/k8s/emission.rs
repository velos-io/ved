use super::*;
use crate::model::{MountKind, MountSpec};

fn map(entries: Vec<(&str, YamlNode)>) -> YamlNode {
    YamlNode::mapping(entries)
}
fn strings(values: impl IntoIterator<Item = String>) -> YamlNode {
    YamlNode::sequence(values.into_iter().map(YamlNode::scalar).collect())
}
fn labels(name: &str) -> YamlNode {
    map(vec![("app.kubernetes.io/name", YamlNode::scalar(name))])
}
fn render_docs(docs: &[YamlNode]) -> Result<String> {
    let mut writer = YamlWriter::new();
    for doc in docs {
        writer.write_doc_separator();
        writer.write_doc(doc)?;
    }
    Ok(writer.into_string())
}
fn env_document(
    name: &str,
    group: &str,
    env: &indexmap::IndexMap<String, serde_json::Value>,
    spec: &AppSpec,
) -> YamlNode {
    let caller = resolve_group(group, name);
    let data = env
        .iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            (
                key.clone(),
                YamlNode::scalar(resolve_cross_namespace_host(&value, &caller, spec).into_owned()),
            )
        })
        .collect();
    ManifestDocBuilder::new("v1", "ConfigMap", &format!("{name}-env"))
        .with_field("data", YamlNode::Mapping(data))
        .to_node()
}
fn env_from(name: &str, has_env: bool) -> YamlNode {
    let mut refs = Vec::new();
    if has_env {
        refs.push(map(vec![(
            "configMapRef",
            map(vec![("name", YamlNode::scalar(format!("{name}-env")))]),
        )]));
    }
    refs.push(map(vec![(
        "secretRef",
        map(vec![("name", YamlNode::scalar("app-secrets"))]),
    )]));
    YamlNode::sequence(refs)
}

pub(super) fn format_mounts(mounts: &[MountSpec]) -> (Vec<YamlNode>, Vec<YamlNode>, Vec<YamlNode>) {
    let mut attached = Vec::new();
    let mut volumes = Vec::new();
    let mut claims = Vec::new();
    for mount in mounts {
        let (name, path, read_only, source) = match &mount.kind {
            MountKind::PersistentVolumeClaim {
                claim_name,
                mount_path,
                read_only,
            } => {
                claims.push(
                    ManifestDocBuilder::new("v1", "PersistentVolumeClaim", claim_name)
                        .with_field(
                            "spec",
                            map(vec![
                                ("accessModes", strings(vec!["ReadWriteOnce".into()])),
                                (
                                    "resources",
                                    map(vec![(
                                        "requests",
                                        map(vec![("storage", YamlNode::scalar("1Gi"))]),
                                    )]),
                                ),
                            ]),
                        )
                        .to_node(),
                );
                (
                    claim_name,
                    mount_path,
                    read_only,
                    (
                        "persistentVolumeClaim",
                        map(vec![("claimName", YamlNode::scalar(claim_name))]),
                    ),
                )
            }
            MountKind::HostPath {
                volume_name,
                host_path,
                mount_path,
                read_only,
            } => {
                let source = if host_path.starts_with('/') {
                    ("hostPath", map(vec![("path", YamlNode::scalar(host_path))]))
                } else {
                    ("emptyDir", YamlNode::EmptyMapping)
                };
                (volume_name, mount_path, read_only, source)
            }
            MountKind::ConfigMap {
                config_map_name,
                mount_path,
                read_only,
            } => (
                config_map_name,
                mount_path,
                read_only,
                (
                    "configMap",
                    map(vec![("name", YamlNode::scalar(config_map_name))]),
                ),
            ),
        };
        attached.push(map(vec![
            ("name", YamlNode::scalar(name)),
            ("mountPath", YamlNode::scalar(path)),
            ("readOnly", YamlNode::boolean(*read_only)),
        ]));
        volumes.push(map(vec![("name", YamlNode::scalar(name)), source]));
    }
    (attached, volumes, claims)
}

pub(super) fn build_service_manifest(
    name: &str,
    svc: &ServiceSpec,
    ingress_suffix: &str,
    validated: &crate::validated::ValidatedSpec,
) -> Result<String> {
    let spec = validated.as_spec();
    let typed = validated.workload(None, name).expect("validated service");
    let mut docs = Vec::new();
    if !svc.env.is_empty() {
        docs.push(env_document(name, &svc.group, &svc.env, spec));
    }
    let (mounts, volumes, claims) = format_mounts(typed.mounts());
    let ports: Vec<_> = typed
        .ports()
        .iter()
        .map(|p| (p.host(), p.container()))
        .collect();
    let image = svc.image.as_deref().unwrap_or(if svc.build.is_some() {
        name
    } else {
        "busybox:latest"
    });
    let mut container = vec![
        ("name", YamlNode::scalar(name)),
        ("image", YamlNode::scalar(image)),
        ("imagePullPolicy", YamlNode::scalar("IfNotPresent")),
        ("envFrom", env_from(name, !svc.env.is_empty())),
    ];
    if let Some(command) = typed.command() {
        let args = match command {
            crate::model::CommandSpec::List(args) => args.clone(),
            crate::model::CommandSpec::String(command) => {
                vec!["sh".into(), "-c".into(), command.clone()]
            }
        };
        container.push(("command", strings(args)));
    }
    if !ports.is_empty() {
        container.push((
            "ports",
            YamlNode::sequence(
                ports
                    .iter()
                    .map(|(_, cp)| {
                        map(vec![
                            ("containerPort", YamlNode::integer(*cp)),
                            ("name", YamlNode::scalar(format!("port-{cp}"))),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    if !mounts.is_empty() {
        container.push(("volumeMounts", YamlNode::sequence(mounts)));
    }
    if let Some(probe) = svc.health_probe(ports.first().map(|p| p.1).unwrap_or(8080)) {
        let (kind, value, period) = match probe {
            crate::model::HealthProbe::Http { path, port, scheme } => {
                let (path, port, scheme) =
                    if path.starts_with("http://") || path.starts_with("https://") {
                        let (p, n, s) = parse_http_probe(&path, port, &ports);
                        (p, n, s.to_string())
                    } else {
                        (path, port, scheme)
                    };
                let mut entries = vec![
                    ("path", YamlNode::scalar(path)),
                    ("port", YamlNode::integer(port)),
                ];
                if scheme == "HTTPS" {
                    entries.push(("scheme", YamlNode::scalar("HTTPS")));
                }
                ("httpGet", map(entries), 5)
            }
            crate::model::HealthProbe::Tcp { port } => {
                let port = ports
                    .iter()
                    .find(|(hp, _)| *hp == port)
                    .map(|(_, cp)| *cp)
                    .unwrap_or(port);
                ("tcpSocket", map(vec![("port", YamlNode::integer(port))]), 3)
            }
            crate::model::HealthProbe::Exec { command } => (
                "exec",
                map(vec![(
                    "command",
                    strings(vec!["sh".into(), "-c".into(), command]),
                )]),
                5,
            ),
        };
        container.push((
            "readinessProbe",
            map(vec![
                (kind, value),
                ("initialDelaySeconds", YamlNode::integer(2)),
                ("periodSeconds", YamlNode::integer(period)),
            ]),
        ));
    }
    let mut pod = Vec::new();
    let mut checks = Vec::new();
    for dep in typed.dependencies() {
        use crate::validated::DependencyCondition;
        let dep_name = dep.target();
        if spec.jobs.contains_key(dep_name) || dep.condition() == DependencyCondition::Success {
            continue;
        }
        if let Some(target) = spec.services.get(dep_name) {
            if !target.is_enabled(dep_name) {
                continue;
            }
            let target_group = resolve_group(&target.group, dep_name);
            let host = if target_group != resolve_group(&svc.group, name) {
                format!("{dep_name}.{target_group}.svc.cluster.local")
            } else {
                dep_name.into()
            };
            let port = match dep.condition() {
                DependencyCondition::Tcp(port) => port,
                _ => target.port.unwrap_or_else(|| {
                    validated
                        .workload(None, dep_name)
                        .expect("validated dependency")
                        .ports()
                        .first()
                        .map(|p| p.container())
                        .unwrap_or(8080)
                }),
            };
            checks.push(format!("until nc -z {host} {port}; do sleep 2; done"));
        } else if dep.external() {
            if let DependencyCondition::Tcp(port) = dep.condition() {
                checks.push(format!("until nc -z {dep_name} {port}; do sleep 2; done"));
            }
        }
    }
    if !checks.is_empty() {
        pod.push((
            "initContainers",
            YamlNode::sequence(vec![map(vec![
                ("name", YamlNode::scalar("wait-for-deps")),
                ("image", YamlNode::scalar("busybox:1.36")),
                ("imagePullPolicy", YamlNode::scalar("IfNotPresent")),
                (
                    "command",
                    strings(vec!["sh".into(), "-c".into(), checks.join(" && ")]),
                ),
            ])]),
        ));
    }
    if let Some(sec) = &svc.security {
        let mut ctx = Vec::new();
        let mut caps = Vec::new();
        let mut pod_ctx = Vec::new();
        if let Some(value) = sec.escalate {
            ctx.push(("allowPrivilegeEscalation", YamlNode::boolean(value)));
        }
        if !sec.caps.is_empty() {
            caps.push(("add", strings(sec.caps.clone())));
        }
        if sec.drop_all == Some(true) {
            caps.push(("drop", strings(vec!["ALL".into()])));
        }
        if !caps.is_empty() {
            ctx.push(("capabilities", map(caps)));
        }
        if !ctx.is_empty() {
            container.push(("securityContext", map(ctx)));
        }
        if let Some(user) = &sec.user {
            if let Some(text) = user.as_str() {
                let (uid, gid) = text
                    .split_once(':')
                    .map(|(u, g)| (u, Some(g)))
                    .unwrap_or((text, None));
                if let Ok(uid) = uid.parse::<i64>() {
                    pod_ctx.push(("runAsUser", YamlNode::integer(uid)));
                }
                if let Some(gid) = gid.and_then(|g| g.parse::<i64>().ok()) {
                    pod_ctx.push(("runAsGroup", YamlNode::integer(gid)));
                }
            } else if let Some(uid) = user.as_i64() {
                pod_ctx.push(("runAsUser", YamlNode::integer(uid)));
            }
        }
        if let Some(gid) = sec.fs_group {
            pod_ctx.push(("fsGroup", YamlNode::integer(gid)));
        }
        if !pod_ctx.is_empty() {
            pod.push(("securityContext", map(pod_ctx)));
        }
    }
    pod.push(("containers", YamlNode::sequence(vec![map(container)])));
    if !volumes.is_empty() {
        pod.push(("volumes", YamlNode::sequence(volumes)));
    }
    docs.push(
        ManifestDocBuilder::new("apps/v1", "Deployment", name)
            .with_label("app.kubernetes.io/name", name)
            .with_field(
                "spec",
                map(vec![
                    ("replicas", YamlNode::integer(1)),
                    ("selector", map(vec![("matchLabels", labels(name))])),
                    (
                        "template",
                        map(vec![
                            ("metadata", map(vec![("labels", labels(name))])),
                            ("spec", map(pod)),
                        ]),
                    ),
                ]),
            )
            .to_node(),
    );
    if !ports.is_empty() {
        let mut service_ports = Vec::new();
        for (hp, cp) in &ports {
            for port in if hp == cp { vec![*cp] } else { vec![*cp, *hp] } {
                service_ports.push(map(vec![
                    ("name", YamlNode::scalar(format!("port-{port}"))),
                    ("port", YamlNode::integer(port)),
                    ("targetPort", YamlNode::integer(*cp)),
                ]));
            }
        }
        docs.push(
            ManifestDocBuilder::new("v1", "Service", name)
                .with_field(
                    "spec",
                    map(vec![
                        ("selector", labels(name)),
                        ("ports", YamlNode::sequence(service_ports)),
                    ]),
                )
                .to_node(),
        );
    }
    if let Some(host) = &svc.ingress {
        let path = map(vec![
            ("path", YamlNode::scalar("/")),
            ("pathType", YamlNode::scalar("Prefix")),
            (
                "backend",
                map(vec![(
                    "service",
                    map(vec![
                        ("name", YamlNode::scalar(name)),
                        (
                            "port",
                            map(vec![(
                                "number",
                                YamlNode::integer(ports.first().map(|p| p.0).unwrap_or(8080)),
                            )]),
                        ),
                    ]),
                )]),
            ),
        ]);
        let rule = map(vec![
            ("host", YamlNode::scalar(format!("{host}.{ingress_suffix}"))),
            ("http", map(vec![("paths", YamlNode::sequence(vec![path]))])),
        ]);
        docs.push(
            ManifestDocBuilder::new("networking.k8s.io/v1", "Ingress", name)
                .with_field(
                    "spec",
                    map(vec![
                        ("ingressClassName", YamlNode::scalar("nginx")),
                        ("rules", YamlNode::sequence(vec![rule])),
                    ]),
                )
                .to_node(),
        );
    }
    docs.extend(claims);
    render_docs(&docs)
}

pub(super) fn build_job_manifest(
    name: &str,
    job: &JobSpec,
    validated: &crate::validated::ValidatedSpec,
) -> Result<String> {
    let spec = validated.as_spec();
    let typed = validated.workload(None, name).expect("validated job");
    let mut docs = Vec::new();
    if !job.env.is_empty() {
        docs.push(env_document(name, &job.group, &job.env, spec));
    }
    let (mounts, volumes, _) = format_mounts(typed.mounts());
    let mut container = vec![
        ("name", YamlNode::scalar(name)),
        (
            "image",
            YamlNode::scalar(if job.image.is_empty() {
                "busybox:1.36"
            } else {
                &job.image
            }),
        ),
        ("imagePullPolicy", YamlNode::scalar("IfNotPresent")),
    ];
    if let Some(command) = typed.command() {
        let args = match command {
            crate::model::CommandSpec::List(args) => args.clone(),
            crate::model::CommandSpec::String(command) => {
                vec!["sh".into(), "-c".into(), command.clone()]
            }
        };
        container.push(("command", strings(args)));
    }
    container.push(("envFrom", env_from(name, !job.env.is_empty())));
    if !mounts.is_empty() {
        container.push(("volumeMounts", YamlNode::sequence(mounts)));
    }
    let mut pod = vec![
        (
            "restartPolicy",
            YamlNode::scalar(if job.schedule.is_some() {
                "OnFailure"
            } else {
                "Never"
            }),
        ),
        ("containers", YamlNode::sequence(vec![map(container)])),
    ];
    if !volumes.is_empty() {
        pod.push(("volumes", YamlNode::sequence(volumes)));
    }
    let doc = if let Some(schedule) = &job.schedule {
        ManifestDocBuilder::new("batch/v1", "CronJob", name)
            .with_field(
                "spec",
                map(vec![
                    ("schedule", YamlNode::scalar(schedule)),
                    (
                        "jobTemplate",
                        map(vec![(
                            "spec",
                            map(vec![("template", map(vec![("spec", map(pod))]))]),
                        )]),
                    ),
                ]),
            )
            .to_node()
    } else {
        let mut builder = ManifestDocBuilder::new("batch/v1", "Job", name);
        if let Some(hook) = &job.hook {
            builder = builder
                .with_annotation("helm.sh/hook", hook)
                .with_annotation("helm.sh/hook-delete-policy", "before-hook-creation");
        }
        builder
            .with_field(
                "spec",
                map(vec![(
                    "template",
                    map(vec![
                        ("metadata", map(vec![("labels", labels(name))])),
                        ("spec", map(pod)),
                    ]),
                )]),
            )
            .to_node()
    };
    docs.push(doc);
    render_docs(&docs)
}
