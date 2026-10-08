#![cfg(feature = "host")]
use std::fs;
mod support;
use std::collections::BTreeMap;

use ved_core::{generate_k8s, AppSpec, ConfigLocator, ConfigSnapshot, ServiceSpec};

#[test]
fn test_get_prop_and_ingress_env_override() {
    let tmp = std::env::temp_dir().join(format!("ved-config-snapshot-{}", std::process::id()));
    let context = support::context(
        &tmp,
        BTreeMap::from([("INGRESS_SUFFIX".into(), "mycorp.internal".into())]),
    );
    assert_eq!(
        context.config().get_prop("INGRESS_SUFFIX", "default.local"),
        "mycorp.internal"
    );
    assert_eq!(
        context.config().resolve_ingress_suffix(None),
        "mycorp.internal"
    );
    assert_eq!(
        context
            .config()
            .resolve_ingress_suffix(Some("override.domain")),
        "override.domain"
    );
}

#[test]
fn test_stacks_repo_url_env_override() {
    let tmp = std::env::temp_dir().join(format!("ved-config-snapshot-{}", std::process::id()));
    let context = support::context(
        &tmp,
        BTreeMap::from([(
            "VED_STACKS_REPO".into(),
            "git@github.com:myorg/custom-stacks.git".into(),
        )]),
    );
    assert_eq!(
        context.config().get_prop("VED_STACKS_REPO", "fallback"),
        "git@github.com:myorg/custom-stacks.git"
    );
}

fn ingress_host(text: &str) -> String {
    use serde::Deserialize;
    serde_yaml::Deserializer::from_str(text)
        .map(|doc| serde_yaml::Value::deserialize(doc).unwrap())
        .find(|doc| doc["kind"].as_str() == Some("Ingress"))
        .unwrap()["spec"]["rules"][0]["host"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn test_k8s_manifest_custom_ingress_suffix() {
    let tmp = std::env::temp_dir().join(format!("ved-test-k8s-ing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let out_dir = tmp.join("installer/k8s");

    let mut spec = AppSpec {
        ingress: Some("corp.example.com".to_string()),
        ..Default::default()
    };
    spec.services.insert(
        "api-gateway".to_string(),
        ServiceSpec {
            image: Some("envoyproxy/envoy:v1.30-latest".to_string()),
            group: "agent".to_string(),
            port: Some(8080),
            ingress: Some("app".to_string()),
            ..Default::default()
        },
    );
    spec.services.insert(
        "otel-collector".to_string(),
        ServiceSpec {
            image: Some("otel/opentelemetry-collector-contrib:0.100.0".to_string()),
            group: "tools".to_string(),
            port: Some(4318),
            ingress: Some("otel".to_string()),
            ..Default::default()
        },
    );

    generate_k8s(&spec, &out_dir).expect("Failed to generate k8s manifests");

    let api_gw = fs::read_to_string(out_dir.join("base/agent/api-gateway.yaml"))
        .expect("Failed to read api-gateway manifest");
    assert_eq!(ingress_host(&api_gw), "app.corp.example.com");

    let otel = fs::read_to_string(out_dir.join("base/tools/otel-collector.yaml"))
        .expect("Failed to read otel-collector manifest");
    assert_eq!(ingress_host(&otel), "otel.corp.example.com");

    let patch = fs::read_to_string(out_dir.join("overlays/local/ns-1/ingress-patch.yaml"))
        .expect("Failed to read ingress-patch manifest");
    assert_eq!(ingress_host(&patch), "app1.corp.example.com");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_get_prop_opt_and_file_resolution() {
    let tmp = std::env::temp_dir().join(format!("ved-test-config-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("config")).expect("create config dir");

    fs::write(
        tmp.join("config/.env"),
        "MY_CUSTOM_PROP=hello_from_env_file\nOVERRIDDEN_PROP=from_base\n",
    )
    .expect("write .env");
    fs::write(
        tmp.join("config/.local.env"),
        "OVERRIDDEN_PROP=from_local\n",
    )
    .expect("write .local.env");

    let locator = ConfigLocator::legacy(&tmp);
    let context = ved_core::WorkspaceContext::with_snapshot(
        tmp.clone(),
        tmp.join("config"),
        tmp.join(".ved"),
        None,
        locator.generated_env_file(),
        ConfigSnapshot::with_environment(&locator, BTreeMap::new()),
    );

    assert_eq!(
        context.config().get_prop("MY_CUSTOM_PROP", "fallback"),
        "hello_from_env_file"
    );
    assert_eq!(
        context.config().get_prop_opt("MY_CUSTOM_PROP"),
        Some("hello_from_env_file".to_string())
    );

    assert_eq!(
        context.config().get_prop("OVERRIDDEN_PROP", "fallback"),
        "from_local"
    );
    assert_eq!(
        context.config().get_prop_opt("OVERRIDDEN_PROP"),
        Some("from_local".to_string())
    );

    let override_context = support::context(
        &tmp,
        BTreeMap::from([("OVERRIDDEN_PROP".into(), "from_process_env".into())]),
    );
    assert_eq!(
        override_context
            .config()
            .get_prop("OVERRIDDEN_PROP", "fallback"),
        "from_process_env"
    );

    assert_eq!(
        context.config().get_prop("NON_EXISTENT_KEY", "my_fallback"),
        "my_fallback"
    );
    assert_eq!(context.config().get_prop_opt("NON_EXISTENT_KEY"), None);

    let _ = fs::remove_dir_all(&tmp);
}
