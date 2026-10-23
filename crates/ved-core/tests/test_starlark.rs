#![cfg(feature = "host")]
#![allow(clippy::needless_borrows_for_generic_args)]

use std::path::Path;
mod support;
use std::collections::BTreeMap;

#[test]
fn test_vedfile_starlark_evaluation() {
    let environment = BTreeMap::from([("REDIS_ENABLED".into(), "true".into())]);

    let vedfile = Path::new("../../examples/full/Vedfile");
    let spec =
        support::load(vedfile, environment.clone()).expect("Failed to load Vedfile with Starlark");

    assert_eq!(spec.app, "vt-deployer");
    assert_eq!(spec.version, "0.1.0");
    assert_eq!(spec.network, "demo-net");
    assert_eq!(spec.ingress.as_deref(), Some("app.localapp"));

    assert!(spec.services.contains_key("postgres"));
    assert!(spec.services.contains_key("redis"));
    assert!(spec.services.contains_key("lambda-emulator"));
    assert!(spec.services.contains_key("api-gateway"));

    let redis = &spec.services["redis"];
    assert_eq!(redis.image.as_deref(), Some("redis:7-alpine"));
    assert_eq!(redis.group, "infra");

    let pg = &spec.services["postgres"];
    assert_eq!(pg.image.as_deref(), Some("postgres:16"));
    assert_eq!(pg.group, "infra");
    assert_eq!(pg.mounts, vec!["postgres-data:/var/lib/postgresql/data"]);

    let lambda = &spec.services["lambda-emulator"];
    assert_eq!(lambda.build.as_deref(), Some("./lambda-emulator"));
    assert_eq!(lambda.memory.as_deref(), Some("512Mi"));
    assert_eq!(
        lambda.mounts,
        vec!["./config/app.json:/etc/app/config.json:ro"]
    );

    assert!(spec.jobs.contains_key("flyway-demo"));
    assert!(spec.jobs.contains_key("smoke-test"));

    let flyway = &spec.jobs["flyway-demo"];
    assert_eq!(flyway.image, "flyway/flyway:13.3");
    assert_eq!(flyway.group, "infra");
}

#[test]
fn test_starlark_yaml_parity() {
    let environment = BTreeMap::from([("REDIS_ENABLED".into(), "true".into())]);

    let vedfile = Path::new("../../examples/full/Vedfile");
    let yaml_file = Path::new("../../examples/full/ved.yaml");

    let star_spec = support::load(vedfile, environment.clone()).expect("Failed to load Vedfile");
    let yaml_spec = support::load(yaml_file, environment.clone()).expect("Failed to load ved.yaml");

    assert_eq!(star_spec.app, yaml_spec.app);
    assert_eq!(star_spec.version, yaml_spec.version);
    assert_eq!(star_spec.network, yaml_spec.network);
    assert_eq!(star_spec.ingress, yaml_spec.ingress);

    let star_compose = support::compose(&star_spec, environment.clone())
        .expect("generate_compose failed for star_spec");
    let yaml_compose = support::compose(&yaml_spec, environment.clone())
        .expect("generate_compose failed for yaml_spec");

    for svc in [
        "postgres",
        "redis",
        "lambda-emulator",
        "api-gateway",
        "flyway-demo",
    ] {
        assert!(star_compose.contains(svc), "Vedfile compose missing {svc}");
        assert!(yaml_compose.contains(svc), "ved.yaml compose missing {svc}");
    }
}

#[test]
fn test_startup_sequence_pruning_and_groups() {
    let environment = BTreeMap::new();
    let vedfile = Path::new("../../examples/full/Vedfile");
    let spec = support::load(vedfile, environment.clone()).expect("Failed to load Vedfile");
    let compose_str =
        support::compose(&spec, environment.clone()).expect("generate_compose failed");

    let val: serde_yaml::Value = serde_yaml::from_str(&compose_str).expect("parse compose YAML");
    let services = val
        .get("services")
        .and_then(|v| v.as_mapping())
        .expect("services mapping");

    let api_gw = services
        .get(&serde_yaml::Value::String("api-gateway".to_string()))
        .and_then(|v| v.as_mapping())
        .expect("api-gateway service");
    let deps = api_gw
        .get(&serde_yaml::Value::String("depends_on".to_string()))
        .and_then(|v| v.as_mapping())
        .expect("api-gateway depends_on");
    assert!(deps.contains_key(&serde_yaml::Value::String("lambda-emulator".to_string())));

    if let Some(otel) = services.get(&serde_yaml::Value::String("otel-collector".to_string())) {
        let prof = otel
            .get(&serde_yaml::Value::String("profiles".to_string()))
            .and_then(|v| v.as_sequence())
            .expect("otel profiles");
        let prof_names: Vec<&str> = prof.iter().filter_map(|p| p.as_str()).collect();
        assert!(prof_names.contains(&"otel"));
        assert!(prof_names.contains(&"tools"));
    }
}

#[test]
fn test_disabled_service_pruning() {
    let environment = BTreeMap::from([("REPORT_EXPORT_ENABLED".into(), "false".into())]);

    let vedfile = Path::new("../../examples/full/Vedfile");
    let spec = support::load(vedfile, environment.clone()).expect("Failed to load Vedfile");
    let compose_str =
        support::compose(&spec, environment.clone()).expect("generate_compose failed");

    let val: serde_yaml::Value = serde_yaml::from_str(&compose_str).expect("parse compose YAML");
    let services = val
        .get("services")
        .and_then(|v| v.as_mapping())
        .expect("services mapping");

    assert!(
        !services.contains_key(&serde_yaml::Value::String("report-export".to_string())),
        "report-export should be pruned when disabled"
    );
}

#[test]
fn test_centralised_env_files() {
    let environment = BTreeMap::new();
    let vedfile = Path::new("../../examples/full/Vedfile");
    let spec = support::load(vedfile, environment.clone()).expect("Failed to load Vedfile");
    assert_eq!(spec.env_files, vec!["config/.env", "config/.local.env"]);

    let compose_str =
        support::compose(&spec, environment.clone()).expect("generate_compose failed");
    let val: serde_yaml::Value = serde_yaml::from_str(&compose_str).expect("parse compose YAML");
    let services = val
        .get("services")
        .and_then(|v| v.as_mapping())
        .expect("services mapping");
    let pg = services
        .get(&serde_yaml::Value::String("postgres".to_string()))
        .and_then(|v| v.as_mapping())
        .expect("postgres service");
    let env_file = pg
        .get(&serde_yaml::Value::String("env_file".to_string()))
        .and_then(|v| v.as_sequence())
        .expect("env_file sequence");
    assert_eq!(env_file.len(), 2);
    assert_eq!(env_file[0].as_str(), Some("config/.env"));
    assert_eq!(env_file[1].as_str(), Some("config/.local.env"));
}

#[test]
fn test_redis_flag_on_generates_dependency() {
    let environment = BTreeMap::from([("REDIS_ENABLED".into(), "true".into())]);

    let vedfile = Path::new("../../examples/full/Vedfile");
    let star_spec = support::load(vedfile, environment.clone()).expect("Failed to load Vedfile");
    let compose_str =
        support::compose(&star_spec, environment.clone()).expect("generate_compose failed");

    let val: serde_yaml::Value = serde_yaml::from_str(&compose_str).expect("parse compose YAML");
    let services = val
        .get("services")
        .and_then(|v| v.as_mapping())
        .expect("services mapping");

    assert!(services.contains_key(&serde_yaml::Value::String("redis".to_string())));

    let api_gw = services
        .get(&serde_yaml::Value::String("api-gateway".to_string()))
        .and_then(|v| v.as_mapping())
        .expect("api-gateway service");
    let deps = api_gw
        .get(&serde_yaml::Value::String("depends_on".to_string()))
        .and_then(|v| v.as_mapping())
        .expect("api-gateway depends_on");

    assert!(deps.contains_key(&serde_yaml::Value::String("lambda-emulator".to_string())));
    assert!(deps.contains_key(&serde_yaml::Value::String("redis".to_string())));
}

#[test]
fn test_redis_flag_off_prunes_workload_and_dependency() {
    let environment = BTreeMap::from([("REDIS_ENABLED".into(), "false".into())]);

    let vedfile = Path::new("../../examples/full/Vedfile");
    let yaml_file = Path::new("../../examples/full/ved.yaml");

    let star_spec = support::load(vedfile, environment.clone()).expect("Failed to load Vedfile");
    let yaml_spec = support::load(yaml_file, environment.clone()).expect("Failed to load ved.yaml");

    for spec in [&star_spec, &yaml_spec] {
        let compose_str =
            support::compose(spec, environment.clone()).expect("generate_compose failed");
        let val: serde_yaml::Value =
            serde_yaml::from_str(&compose_str).expect("parse compose YAML");
        let services = val
            .get("services")
            .and_then(|v| v.as_mapping())
            .expect("services mapping");

        assert!(
            !services.contains_key(&serde_yaml::Value::String("redis".to_string())),
            "redis service should be pruned when REDIS_ENABLED=false"
        );

        let api_gw = services
            .get(&serde_yaml::Value::String("api-gateway".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("api-gateway service");
        let deps = api_gw
            .get(&serde_yaml::Value::String("depends_on".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("api-gateway depends_on");

        assert!(deps.contains_key(&serde_yaml::Value::String("lambda-emulator".to_string())));
        assert!(
            !deps.contains_key(&serde_yaml::Value::String("redis".to_string())),
            "api-gateway should not depend on pruned redis service"
        );
    }
}

#[test]
fn test_named_volume_extraction_in_compose() {
    let environment = BTreeMap::new();
    let vedfile = Path::new("../../examples/nginx-postgres/Vedfile");
    let spec =
        support::load(vedfile, environment.clone()).expect("Failed to load nginx-postgres Vedfile");
    let compose_str =
        support::compose(&spec, environment.clone()).expect("generate_compose failed");
    let val: serde_yaml::Value = serde_yaml::from_str(&compose_str).expect("parse compose YAML");
    let volumes = val
        .get("volumes")
        .and_then(|v| v.as_mapping())
        .expect("volumes mapping");
    assert!(volumes.contains_key(&serde_yaml::Value::String("db-data".to_string())));
}

#[test]
fn test_health_http_resolves_against_ports_and_accepts_full_url() {
    let environment = BTreeMap::new();
    let tmp = std::env::temp_dir().join(format!("ved-test-health-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create_dir_all");
    let vedfile_path = tmp.join("Vedfile");

    let vedfile_content = r#"
app = App(
    name="health-test",
    version="1.0.0",
)

app.services.add(
    Service(
        name="web-custom-port",
        image="app:latest",
        ports=[Port(3000)],
        health=Health.http("/health"),
    )
)

app.services.add(
    Service(
        name="web-full-url",
        image="app:latest",
        ports=[3000],
        health=Health.http("http://localhost:3000/ready"),
    )
)

app.services.add(
    Service(
        name="web-explicit-port",
        image="app:latest",
        ports=[3000],
        health=Health.http("/ping", port=9000),
    )
)
"#;
    std::fs::write(&vedfile_path, vedfile_content).expect("write Vedfile");

    let spec = support::load(&vedfile_path, environment.clone()).expect("load starlark");

    let compose_str =
        support::compose(&spec, environment.clone()).expect("generate_compose failed");

    assert!(
        compose_str.contains("http://localhost:3000/health"),
        "expected compose to use port 3000 for web-custom-port, got:\n{compose_str}"
    );
    assert!(
        !compose_str.contains("8080/health"),
        "did not expect 8080/health in compose"
    );

    assert!(
        compose_str.contains("http://localhost:3000/ready"),
        "expected compose to use full url 3000/ready, got:\n{compose_str}"
    );
    assert!(
        !compose_str.contains("8080http"),
        "did not expect double url prefix 8080http"
    );

    assert!(
        compose_str.contains("http://localhost:9000/ping"),
        "expected compose to use explicit port 9000, got:\n{compose_str}"
    );

    let k8s_dir = tmp.join("k8s");
    ved_core::generate_k8s(&spec, &k8s_dir).expect("generate_k8s");
    let web_custom_deployment =
        std::fs::read_to_string(k8s_dir.join("base/agent/web-custom-port.yaml"))
            .expect("read web-custom-port yaml");
    assert!(
        web_custom_deployment.contains("path: /health"),
        "k8s probe missing path /health"
    );
    assert!(
        web_custom_deployment.contains("port: 3000"),
        "k8s probe missing port 3000"
    );
}

#[test]
fn test_starlark_ved_get_env_and_implicit_flags_with_config_files() {
    let environment = BTreeMap::new();
    let tmp = std::env::temp_dir().join(format!("ved-test-starlark-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("config")).expect("create config dir");
    std::fs::write(
        tmp.join("config/.env"),
        "API_SECRET=super_secret_token\nFEATURE_X_ENABLED=false\n",
    )
    .expect("write .env");

    let vedfile_path = tmp.join("Vedfile");
    let vedfile_content = r#"
app = App(
    name="env-test",
    version="1.0.0",
)

app.services.add(
    Service(
        name="feature-x",
        image="app:latest",
        env={
            "SECRET": env.secret("API_SECRET", "default_secret"),
        },
    )
)
"#;
    std::fs::write(&vedfile_path, vedfile_content).expect("write Vedfile");

    let spec = support::load(&vedfile_path, environment.clone()).expect("load starlark");
    let svc = &spec.services["feature-x"];

    assert_eq!(svc.env["SECRET"], "super_secret_token");
    assert!(!support::context(&tmp, environment).enabled("feature-x"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_starlark_rejects_malformed_mounts() {
    let environment = BTreeMap::new();
    let tmp = std::env::temp_dir().join(format!("ved-test-starlark-mounts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create dir");

    let vedfile_path = tmp.join("Vedfile");
    let content = r#"
app = App(name="test-app", version="1.0.0")
app.services.add(
    Service(
        name="web",
        image="nginx",
        mounts=["cm:-bad-name-:/etc/config"],
    )
)
"#;
    std::fs::write(&vedfile_path, content).expect("write Vedfile");

    let res = support::load(&vedfile_path, environment.clone());
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("Service 'web': invalid mounts[0]"),
        "got: {err}"
    );
}

#[test]
fn test_app_config_dir_and_env_files() {
    let tmp = std::env::temp_dir().join(format!("ved-test-cfg-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create dir");

    // 1. Default config_dir is "."
    let v1 = tmp.join("Vedfile1");
    std::fs::write(&v1, "app = App(name='app1', version='1.0.0')\n").unwrap();
    let spec1 = support::load(&v1, BTreeMap::new()).unwrap();
    assert_eq!(spec1.config_dir, ".");
    assert_eq!(spec1.env_files, vec!["config/.env", "config/.local.env"]);

    // 2. Custom config_dir with relative env_files
    let v2 = tmp.join("Vedfile2");
    std::fs::write(
        &v2,
        "app = App(name='app2', version='1.0.0', config_dir='custom-cfg', env_files=['.env', '.local.env'])\n",
    )
    .unwrap();
    let spec2 = support::load(&v2, BTreeMap::new()).unwrap();
    assert_eq!(spec2.config_dir, "custom-cfg");
    assert_eq!(
        spec2.env_files,
        vec!["custom-cfg/.env", "custom-cfg/.local.env"]
    );

    // 3. Explicit config_dir="." with env_files=[".env", ".local.env"]
    let v3 = tmp.join("Vedfile3");
    std::fs::write(
        &v3,
        "app = App(name='app3', version='1.0.0', config_dir='.', env_files=['.env', '.local.env'])\n",
    )
    .unwrap();
    let spec3 = support::load(&v3, BTreeMap::new()).unwrap();
    assert_eq!(spec3.config_dir, ".");
    assert_eq!(spec3.env_files, vec![".env", ".local.env"]);

    // 4. Overridden by VED_CONFIG_DIR environment variable
    let mut env = BTreeMap::new();
    env.insert("VED_CONFIG_DIR".to_string(), "env-override-dir".to_string());
    let v4 = tmp.join("Vedfile4");
    std::fs::write(
        &v4,
        "app = App(name='app4', version='1.0.0', env_files=['.env', '.local.env'])\n",
    )
    .unwrap();
    let spec4 = support::load(&v4, env).unwrap();
    assert_eq!(spec4.config_dir, "env-override-dir");
    assert_eq!(
        spec4.env_files,
        vec!["env-override-dir/.env", "env-override-dir/.local.env"]
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
