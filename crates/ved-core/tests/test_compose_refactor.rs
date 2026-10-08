#![cfg(feature = "host")]
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[test]
fn test_compose_baseline_parity_full_example() {
    let environment = BTreeMap::from([
        ("REDIS_ENABLED".into(), "false".into()),
        ("OTEL_ENABLED".into(), "true".into()),
    ]);
    let spec = support::load(
        Path::new("../../examples/full/Vedfile"),
        environment.clone(),
    )
    .expect("load full example Vedfile");
    let generated = support::compose(&spec, environment).expect("generate compose");
    let generated: serde_yaml::Value = serde_yaml::from_str(&generated).expect("parse compose");
    let services: BTreeSet<_> = generated["services"]
        .as_mapping()
        .expect("services")
        .keys()
        .map(|key| key.as_str().expect("service name"))
        .collect();
    assert_eq!(
        services,
        BTreeSet::from([
            "api-gateway",
            "cleanup-job",
            "flyway-demo",
            "lambda-emulator",
            "otel-collector",
            "postgres",
            "smoke-test",
        ])
    );
    let volumes = generated["volumes"].as_mapping().expect("volumes");
    assert_eq!(volumes.len(), 1);
    assert!(volumes.contains_key(serde_yaml::Value::String("postgres-data".into())));
}
