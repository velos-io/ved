mod support;
use serde::Deserialize;
use serde_yaml::Value;
use std::fs;
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("ved-cli-yaml-roundtrip-{}", std::process::id()));
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
fn yaml_and_starlark_cli_compilation_preserve_quoted_multiline_values() {
    let fixture = Fixture::new();
    let value = "a\"b\\backslash\nkey: value # 日本語🦀";
    let yaml = serde_yaml::to_string(&serde_json::json!({
        "app": "audit", "version": "1", "env_files": [],
        "services": { "web": { "image": "nginx", "enabled": true,
            "env": { "MESSAGE": value }, "health": { "test": format!("echo '{value}'") }
        }}
    }))
    .unwrap();
    let literal = serde_json::to_string(value).unwrap();
    let command_literal = serde_json::to_string(&format!("echo '{value}'")).unwrap();
    let starlark = format!("app = App(name='audit', version='1', env_files=[])\napp.services.add(Service(name='web', image='nginx', env={{'MESSAGE': {literal}}}, health=Health.exec({command_literal})))\n");
    for (name, input) in [("spec.yaml", yaml), ("Vedfile", starlark)] {
        let source = fixture.0.join(name);
        fs::write(&source, input).unwrap();
        let output_dir = fixture.0.join(format!("out-{name}"));
        let result = support::command(env!("CARGO_BIN_EXE_ved"), &fixture.0)
            .args([
                "-d",
                fixture.0.to_str().unwrap(),
                "compile",
                "-f",
                source.to_str().unwrap(),
                "-t",
                "k8s",
                "--output-dir",
                output_dir.to_str().unwrap(),
            ])
            .env_remove("VED_CONFIG_DIR")
            .env_remove("VED_WORK_DIR")
            .env_remove("VED_OUTPUT_DIR")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = fs::read_to_string(output_dir.join("k8s/base/agent/web.yaml")).unwrap();
        let docs: Vec<Value> = serde_yaml::Deserializer::from_str(&text)
            .map(|d| Value::deserialize(d).unwrap())
            .collect();
        let config = docs
            .iter()
            .find(|d| d["kind"].as_str() == Some("ConfigMap"))
            .unwrap();
        assert_eq!(config["data"]["MESSAGE"].as_str(), Some(value));
        let deployment = docs
            .iter()
            .find(|d| d["kind"].as_str() == Some("Deployment"))
            .unwrap();
        assert_eq!(
            deployment["spec"]["template"]["spec"]["containers"][0]["readinessProbe"]["exec"]
                ["command"],
            serde_yaml::to_value(vec![
                "sh".to_string(),
                "-c".to_string(),
                format!("echo '{value}'")
            ])
            .unwrap()
        );
    }
}
