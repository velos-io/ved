#![cfg(feature = "host")]
use std::path::PathBuf;
mod support;
use std::collections::BTreeMap;

#[test]
fn test_compile_ved_yaml() {
    let mut manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_path.pop();
    manifest_path.pop();
    let ved_yaml = manifest_path.join("examples/full/ved.yaml");

    let spec = support::load(&ved_yaml, BTreeMap::new()).expect("Failed to load ved.yaml");
    assert_eq!(spec.app, "vt-deployer");
    assert_eq!(spec.version, "0.1.0");
    assert!(spec.services.contains_key("postgres"));
    assert!(spec.services.contains_key("lambda-emulator"));
    assert!(spec.services.contains_key("api-gateway"));

    let compose_str = support::compose(&spec, BTreeMap::new()).expect("Failed to generate compose");
    assert!(compose_str.contains("postgres:16"));
    assert!(compose_str.contains("container_name: postgres"));
    assert!(compose_str.contains("demo-net"));
}

#[test]
fn test_compile_completed_wait_for_condition() {
    let mut spec = ved_core::AppSpec::default();
    spec.services.insert(
        "migration".into(),
        ved_core::ServiceSpec {
            image: Some("flyway/flyway".into()),
            ..Default::default()
        },
    );
    spec.services.insert(
        "api".into(),
        ved_core::ServiceSpec {
            image: Some("node:18".into()),
            wait_for: vec!["migration.completed".into()],
            ..Default::default()
        },
    );
    let compose_str = support::compose(&spec, BTreeMap::new()).expect("Failed to generate compose");
    assert!(compose_str.contains("condition: service_completed_successfully"));
}
