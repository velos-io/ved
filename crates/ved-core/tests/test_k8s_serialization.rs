#![cfg(feature = "host")]
use serde::Deserialize;
use serde_json::json;
use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use ved_core::{generate_k8s, AppSpec};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Output(PathBuf);
impl Output {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ved-k8s-serialization-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn parse(path: &Path) -> Vec<Value> {
    let text = fs::read_to_string(path).unwrap();
    serde_yaml::Deserializer::from_str(&text)
        .map(|doc| {
            Value::deserialize(doc).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        })
        .filter(|v| !v.is_null())
        .collect()
}
fn parse_tree(path: &Path) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            parse_tree(&path);
        } else if path.extension().is_some_and(|ext| ext == "yaml") {
            parse(&path);
        }
    }
}
fn document(path: &Path, kind: &str) -> Value {
    parse(path)
        .into_iter()
        .find(|doc| doc["kind"].as_str() == Some(kind))
        .unwrap_or_else(|| panic!("missing {kind} in {}", path.display()))
}
fn fixture(raw: serde_json::Value) -> AppSpec {
    serde_json::from_value(raw).unwrap()
}

#[test]
fn adversarial_workload_values_round_trip_with_correct_types() {
    let command = "printf 'a\"b\\c: # 世界'\nprintf done";
    let values = [
        "a\"b",
        "C:\\tmp\\app",
        "line one\nline two",
        "colon: space",
        "# hash",
        "世界 🦀",
        "true",
        "null",
        "0123",
        "[one, two]",
        "",
        "---\nkind: injected",
    ];
    let mut env = serde_json::Map::new();
    for (i, value) in values.iter().enumerate() {
        env.insert(format!("KEY_{i}"), json!(value));
    }
    env.insert("key: # \"世界\nnext".into(), json!("mapping key survives"));
    env.insert("BOOL".into(), json!(true));
    env.insert("NUMBER".into(), json!(42));
    env.insert("OBJECT".into(), json!({"nested": true}));
    env.insert("DB_HOST".into(), json!("db"));
    let schedule = "0 1 * * * # \"quoted\"\\suffix\nnext";
    let hook = "pre-install: # \"hook\"\nnext";
    let mount_path = "/etc/quoted\"path\\file\nnext";
    let spec = fixture(json!({
        "app": "roundtrip", "version": "1", "ingress": "example.test",
        "services": {
            "db": {"enabled": true, "image": "postgres", "group": "infra", "port": 5432},
            "api": {"enabled": true, "image": "image: \"quoted\"\\path\nnext", "ports": ["9000:90"], "env": env,
                "health": command, "waitFor": ["db"], "security": {"caps": ["NET_ADMIN", "quoted: # value"], "dropAll": true, "escalate": false, "user": "1000:1001", "fsGroup": 1002},
                "mounts": [format!("/host/data:{mount_path}:ro"), format!("cm:app-config:{mount_path}:ro")], "ingress": "api"}
        },
        "jobs": {
            "list-job": {"enabled": true, "image": "worker", "command": ["run", command, "true"], "hook": hook, "env": {"MESSAGE": values[0]}},
            "string-job": {"enabled": true, "image": "worker", "command": command, "mounts": [format!("cm:app-config:{mount_path}:ro")]},
            "cron": {"enabled": true, "image": "worker", "schedule": schedule, "command": [command]}
        }
    }));
    let output = Output::new();
    generate_k8s(&spec, &output.0).unwrap();
    parse_tree(&output.0);
    let file = output.0.join("base/agent/api.yaml");
    let cm = document(&file, "ConfigMap");
    for (i, value) in values.iter().enumerate() {
        assert_eq!(cm["data"][format!("KEY_{i}")].as_str(), Some(*value));
    }
    assert_eq!(
        cm["data"]["key: # \"世界\nnext"].as_str(),
        Some("mapping key survives")
    );
    assert_eq!(cm["data"]["BOOL"].as_str(), Some("true"));
    assert_eq!(cm["data"]["NUMBER"].as_str(), Some("42"));
    assert_eq!(cm["data"]["OBJECT"].as_str(), Some("{\"nested\":true}"));
    assert_eq!(
        cm["data"]["DB_HOST"].as_str(),
        Some("db.infra.svc.cluster.local")
    );
    let deployment = document(&file, "Deployment");
    assert_eq!(deployment["spec"]["replicas"].as_i64(), Some(1));
    let pod = &deployment["spec"]["template"]["spec"];
    let container = &pod["containers"][0];
    assert_eq!(
        container["image"].as_str(),
        spec.services["api"].image.as_deref()
    );
    assert_eq!(container["ports"][0]["containerPort"].as_i64(), Some(90));
    assert_eq!(
        container["readinessProbe"]["exec"]["command"][2].as_str(),
        Some(command)
    );
    assert_eq!(
        pod["initContainers"][0]["command"][2].as_str(),
        Some("until nc -z db.infra.svc.cluster.local 5432; do sleep 2; done")
    );
    for i in 0..2 {
        assert_eq!(
            container["volumeMounts"][i]["mountPath"].as_str(),
            Some(mount_path)
        );
        assert_eq!(
            container["volumeMounts"][i]["readOnly"].as_bool(),
            Some(true)
        );
    }
    assert_eq!(
        pod["volumes"][0]["hostPath"]["path"].as_str(),
        Some("/host/data")
    );
    assert_eq!(
        pod["volumes"][1]["configMap"]["name"].as_str(),
        Some("app-config")
    );
    assert_eq!(
        container["securityContext"]["allowPrivilegeEscalation"].as_bool(),
        Some(false)
    );
    assert_eq!(
        container["securityContext"]["capabilities"]["add"][1].as_str(),
        Some("quoted: # value")
    );
    assert_eq!(
        container["securityContext"]["capabilities"]["drop"][0].as_str(),
        Some("ALL")
    );
    for (key, value) in [("runAsUser", 1000), ("runAsGroup", 1001), ("fsGroup", 1002)] {
        assert_eq!(pod["securityContext"][key].as_i64(), Some(value));
    }
    let service = document(&file, "Service");
    assert_eq!(service["spec"]["ports"][0]["port"].as_i64(), Some(90));
    assert_eq!(service["spec"]["ports"][1]["port"].as_i64(), Some(9000));
    assert_eq!(service["spec"]["ports"][1]["targetPort"].as_i64(), Some(90));
    let list = document(&output.0.join("base/agent/list-job.yaml"), "Job");
    let list_env = document(&output.0.join("base/agent/list-job.yaml"), "ConfigMap");
    assert_eq!(list_env["data"]["MESSAGE"].as_str(), Some(values[0]));
    assert_eq!(
        list["spec"]["template"]["spec"]["restartPolicy"].as_str(),
        Some("Never")
    );
    let list_args = &list["spec"]["template"]["spec"]["containers"][0]["command"];
    assert_eq!(list_args.as_sequence().unwrap().len(), 3);
    assert_eq!(list_args[0].as_str(), Some("run"));
    assert_eq!(list_args[2].as_str(), Some("true"));
    assert_eq!(
        list["metadata"]["annotations"]["helm.sh/hook"].as_str(),
        Some(hook)
    );
    assert_eq!(
        list["spec"]["template"]["spec"]["containers"][0]["command"][1].as_str(),
        Some(command)
    );
    let string = document(&output.0.join("base/agent/string-job.yaml"), "Job");
    let args = &string["spec"]["template"]["spec"]["containers"][0]["command"];
    assert_eq!(args.as_sequence().unwrap().len(), 3);
    assert_eq!(args[0].as_str(), Some("sh"));
    assert_eq!(args[1].as_str(), Some("-c"));
    assert_eq!(args[2].as_str(), Some(command));
    let cron = document(&output.0.join("base/agent/cron.yaml"), "CronJob");
    assert_eq!(
        cron["spec"]["jobTemplate"]["spec"]["template"]["spec"]["restartPolicy"].as_str(),
        Some("OnFailure")
    );
    assert_eq!(cron["spec"]["schedule"].as_str(), Some(schedule));
    assert_eq!(
        cron["spec"]["jobTemplate"]["spec"]["template"]["spec"]["containers"][0]["command"][0]
            .as_str(),
        Some(command)
    );
}

#[test]
#[ignore = "opt-in local kubectl validation; each invocation has a 10-second deadline"]
fn complete_generated_trees_build_with_real_kustomize() {
    let specs = [
        fixture(json!({"app": "empty", "version": "1"})),
        fixture(json!({
            "app": "combined", "version": "1", "ingress": "example.test",
            "services": {
                "db": {"enabled": true, "image": "postgres:16", "group": "infra", "port": 5432, "mounts": ["db-data:/var/lib/postgresql/data"]},
                "api": {"enabled": true, "image": "nginx:alpine", "port": 8080, "ingress": "api", "env": {"MESSAGE": "quote \" and newline\nnext"}, "health": "/health", "waitFor": ["db"]},
                "metrics": {"enabled": true, "image": "busybox", "group": "analytics", "port": 9090}
            },
            "jobs": {"cleanup": {"enabled": true, "image": "busybox", "group": "infra", "schedule": "0 1 * * *", "command": ["sh", "-c", "echo done"]}}
        })),
    ];
    let mut failures = Vec::new();
    for spec in specs {
        let output = Output::new();
        generate_k8s(&spec, &output.0).unwrap();
        parse_tree(&output.0);
        let mut targets = vec![output.0.join("base"), output.0.join("overlays/dev")];
        for entry in fs::read_dir(output.0.join("overlays/local")).unwrap() {
            targets.push(entry.unwrap().path());
        }
        for target in targets {
            let result = ved_process::Runner::new("kubectl")
                .arg("kustomize")
                .arg(&target)
                .capture()
                .check_status(false)
                .timeout(std::time::Duration::from_secs(10))
                .run()
                .unwrap();
            if !result.status.success() {
                failures.push(format!(
                    "{}: {}",
                    target.display(),
                    String::from_utf8_lossy(&result.stderr)
                ));
                continue;
            }
            let text = String::from_utf8(result.stdout).unwrap();
            for doc in serde_yaml::Deserializer::from_str(&text) {
                Value::deserialize(doc).unwrap();
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Kustomize failures:\n{}",
        failures.join("\n")
    );
}
