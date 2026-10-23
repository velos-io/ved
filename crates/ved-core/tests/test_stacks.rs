#![cfg(feature = "host")]
mod support;
use std::collections::BTreeMap;
use std::fs;

use ved_core::parse_shorthand;

#[test]
fn test_starlark_stack_add_otel_end_to_end() {
    let tmp = std::env::temp_dir().join(format!("ved-test-stack-otel-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("Failed to create test directory");
    fs::create_dir_all(tmp.join("config")).expect("create project config directory");

    let vedfile_content = r#"
"""Vedfile with OpenTelemetry stack."""
from ved import App, Health, Mount, Port, Service, Stack

app = App(
    name="my-otel-app",
    version="1.0.0",
    network="my-net",
)

Stack.add("otel")

app.services.add(
    Service(
        name="web",
        image="nginx:alpine",
        ports=[Port(80, host=8080)],
        health=Health.http("/"),
        wait_for=["db:healthy"],
    )
)

app.services.add(
    Service(
        name="db",
        image="postgres:16-alpine",
        ports=[Port(5432, host=5432)],
        health=Health.exec("pg_isready -U admin"),
    )
)
"#;

    let vedfile_path = tmp.join("Vedfile");
    fs::write(&vedfile_path, vedfile_content).expect("Failed to write Vedfile");

    // Keep the integration test independent of a user-installed or network-synced catalog.
    let catalog = tmp.join("catalog/otel");
    fs::create_dir_all(catalog.join("config")).expect("create local stack catalog");
    fs::write(
        tmp.join("config/.env"),
        format!("VED_STACKS_PATH={}\n", tmp.join("catalog").display()),
    )
    .expect("write local stack catalog setting");
    fs::write(
        catalog.join("Vedfile"),
        r#"
services = [
    Service(name="jaeger", image="jaegertracing/all-in-one:1.62.0"),
    Service(name="prometheus", image="prom/prometheus:v2.55.1", mounts=["config/prometheus.yml:/etc/prometheus/prometheus.yml:ro"]),
    Service(name="otel-collector", image="otel/opentelemetry-collector:0.114.0", mounts=["config/otel-collector-config.yaml:/etc/otelcol/config.yaml:ro"]),
    Service(name="grafana", image="grafana/grafana:11.0.0", wait_for=["prometheus"]),
]
"#,
    )
    .expect("write local stack definition");
    fs::write(catalog.join("config/prometheus.yml"), "global: {}\n")
        .expect("write Prometheus config");
    fs::write(
        catalog.join("config/otel-collector-config.yaml"),
        "receivers: {}\n",
    )
    .expect("write collector config");

    let spec = support::load(&vedfile_path, BTreeMap::new())
        .expect("Failed to load Starlark spec with otel stack");

    assert!(spec.stacks.contains(&"otel".to_string()));

    assert!(spec.services.contains_key("web"));
    assert!(spec.services.contains_key("db"));
    assert!(spec.services.contains_key("jaeger"));
    assert!(spec.services.contains_key("prometheus"));
    assert!(spec.services.contains_key("otel-collector"));
    assert!(spec.services.contains_key("grafana"));

    let prom_cfg = tmp.join(".ved/stacks/otel/config/prometheus.yml");
    let otel_cfg = tmp.join(".ved/stacks/otel/config/otel-collector-config.yaml");
    assert!(
        prom_cfg.exists(),
        "prometheus.yml was not copied to project .ved"
    );
    assert!(
        otel_cfg.exists(),
        "otel-collector-config.yaml was not copied to project .ved"
    );

    let prom_svc = &spec.services["prometheus"];
    assert!(prom_svc
        .mounts
        .iter()
        .any(|m| m.starts_with(&format!("{}:", prom_cfg.display()))));

    let collector_svc = &spec.services["otel-collector"];
    assert!(collector_svc
        .mounts
        .iter()
        .any(|m| m.starts_with(&format!("{}:", otel_cfg.display()))));

    let compose_yaml =
        support::compose(&spec, BTreeMap::new()).expect("Failed to generate compose YAML");
    assert!(compose_yaml.contains("image: jaegertracing/all-in-one:1.62.0"));
    assert!(compose_yaml.contains("image: prom/prometheus:v2.55.1"));
    assert!(compose_yaml.contains("image: otel/opentelemetry-collector:0.114.0"));
    assert!(compose_yaml.contains("image: grafana/grafana:11.0.0"));
    assert!(compose_yaml.contains(&prom_cfg.display().to_string()));
    assert!(compose_yaml.contains(&otel_cfg.display().to_string()));

    assert!(compose_yaml.contains("condition: service_healthy"));
    assert!(compose_yaml.contains("condition: service_started"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_shorthand_stack_otel() {
    let shorthand = r#"
        App shorthand-otel-app 1.0.0
        Network otel-net

        Stack otel

        Service api node:20-alpine 3000
    "#;

    let spec = parse_shorthand(shorthand).expect("Failed to parse shorthand with Stack otel");
    assert!(spec.stacks.contains(&"otel".to_string()));
    assert!(spec.services.contains_key("api"));
}

#[test]
fn test_sync_stacks_catalog_local_override_integration() {
    let tmp = std::env::temp_dir().join(format!("ved-test-local-stacks-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("Failed to create test directory");

    let context = support::context(
        &tmp,
        BTreeMap::from([("VED_STACKS_PATH".into(), tmp.to_string_lossy().into_owned())]),
    );
    let res = ved_core::sync_stacks_catalog_with_context(&context);

    assert!(
        res.is_ok(),
        "sync_stacks_catalog should succeed on local override without .git"
    );
    let _ = fs::remove_dir_all(&tmp);
}
