#![cfg(feature = "host")]
use std::fs;
use std::path::PathBuf;
use ved_core::{AppSpec, ServiceSpec, ValidatedSpec, VedCompiler};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ved-boundary-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn facade_loads_all_formats_and_compiles_validated_snapshots() {
    let fixture = Fixture::new();
    let compiler = VedCompiler::new(&fixture.0);
    let sources = [
        ("spec.yaml", "app: boundary\nversion: '1'\nenv_files: []\nservices:\n  web:\n    image: nginx\n    ports: ['8080:80']\n    command: ['echo', 'quoted: # value']\n"),
        ("Vedfile", "app = App(name='boundary', version='1', env_files=[])\napp.services.add(Service(name='web', image='nginx', ports=['8080:80'], command=['echo', 'quoted: # value']))\n"),
        ("spec.ved", "App boundary 1\nService web nginx 8080:80\n"),
    ];
    for (name, source) in sources {
        fs::write(fixture.0.join(name), source).unwrap();
        let validated = compiler.load_validated_spec(name).unwrap();
        assert_eq!(validated.as_spec().app, "boundary");
        let ports = validated.workload(None, "web").unwrap().ports();
        assert_eq!((ports[0].host(), ports[0].container()), (8080, 80));
        let compose = compiler.compile_compose_validated(&validated).unwrap();
        let compose: serde_yaml::Value = serde_yaml::from_str(&compose).unwrap();
        assert_eq!(compose["services"]["web"]["image"].as_str(), Some("nginx"));
        compiler.compile_helm_validated(&validated).unwrap();
        compiler
            .compile_k8s_validated(&validated, format!("out-{name}"))
            .unwrap();
        let yaml =
            fs::read_to_string(fixture.0.join(format!("out-{name}/base/agent/web.yaml"))).unwrap();
        if name != "spec.ved" {
            use serde::Deserialize;
            let docs: Vec<serde_yaml::Value> = serde_yaml::Deserializer::from_str(&yaml)
                .map(|doc| serde_yaml::Value::deserialize(doc).unwrap())
                .collect();
            let deployment = docs
                .iter()
                .find(|doc| doc["kind"].as_str() == Some("Deployment"))
                .unwrap();
            assert_eq!(
                deployment["spec"]["template"]["spec"]["containers"][0]["command"][1].as_str(),
                Some("quoted: # value")
            );
        }
    }
}

#[test]
fn every_public_emitter_rejects_bad_fields_before_output_creation() {
    let output = std::env::temp_dir().join(format!("ved-invalid-boundary-{}", std::process::id()));
    let _ = fs::remove_dir_all(&output);
    for field in ["ports", "command", "mounts", "waitFor"] {
        let mut spec = AppSpec {
            app: "boundary".into(),
            ..Default::default()
        };
        let mut svc = ServiceSpec::default();
        match field {
            "ports" => svc.ports = vec![serde_json::json!("invalid")],
            "command" => svc.command = Some(serde_json::json!(["echo", 1])),
            "mounts" => svc.mounts = vec!["data:relative".into()],
            _ => svc.wait_for = vec!["missing:healthy".into()],
        }
        spec.services.insert("web".into(), svc);
        assert!(ValidatedSpec::try_from(&spec)
            .unwrap_err()
            .to_string()
            .contains(field.trim_end_matches('s')));
        assert!(ved_core::generate_compose(&spec).is_err());
        assert!(ved_core::generate_helm_values(&spec).is_err());
        assert!(ved_core::generate_k8s(&spec, &output).is_err());
        assert!(!output.exists());
    }
}

#[test]
fn failed_facade_loading_reports_source_path() {
    let path = std::env::temp_dir().join(format!("ved-bad-source-{}.yaml", std::process::id()));
    fs::write(
        &path,
        "app: boundary\nversion: '1'\nservices:\n  web:\n    ports: [false]\n",
    )
    .unwrap();
    let err = VedCompiler::new(".")
        .load_spec(&path)
        .unwrap_err()
        .to_string();
    fs::remove_file(&path).unwrap();
    assert!(err.contains(path.to_str().unwrap()));
    assert!(err.contains("web"));
    assert!(err.contains("ports[0]"));
}
