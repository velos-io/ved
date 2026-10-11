#![cfg(unix)]
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ved-context-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir = fs::canonicalize(dir).unwrap();
        for name in [
            "project/config",
            "selected-config",
            "selected-work",
            "selected-output",
            "wrong-config",
            "bin",
            "project/module",
        ] {
            fs::create_dir_all(dir.join(name)).unwrap();
        }
        fs::write(dir.join("project/ved.yaml"), "app: context-test\nversion: '1'\nservices:\n  web:\n    image: nginx\n    ports: ['8080:80']\n    ingress: web\njobs:\n  smoke-test:\n    image: alpine\n    command: echo ok\n").unwrap();
        let tool = dir.join("bin/fake-build");
        fs::write(dir.join("selected-config/.env"), format!("DEPLOYMENT_TARGET=compose\nINGRESS_SUFFIX=chosen.example\nOTEL_ENABLED=true\nK8S_APP_SLOT=7\nNPM_BIN={}\n", tool.display())).unwrap();
        fs::write(
            dir.join("selected-config/.local.env"),
            "INGRESS_SUFFIX=local.example\n",
        )
        .unwrap();
        fs::write(
            dir.join("project/config/.env"),
            "DEPLOYMENT_TARGET=bad-root\nINGRESS_SUFFIX=wrong-root.example\n",
        )
        .unwrap();
        fs::write(dir.join("wrong-config/.env"), "DEPLOYMENT_TARGET=bad-env\n").unwrap();
        let script = r#"#!/bin/sh
{
 printf 'BEGIN\nEXE=%s\nCWD=%s\nCONFIG=%s\nWORK=%s\nOUTPUT=%s\nPROFILES=%s\n' "${0##*/}" "$PWD" "$VED_CONFIG_DIR" "$VED_WORK_DIR" "$VED_OUTPUT_DIR" "$COMPOSE_PROFILES"
 for arg do printf 'ARG=%s\n' "$arg"; done
 printf 'END\n'
} >> "$TEST_TRACE"
cat > /dev/null 2>&1 || true
exit 0
"#;
        for name in ["docker", "kubectl", "fake-build"] {
            let file = dir.join("bin").join(name);
            fs::write(&file, script).unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let state = serde_json::json!({"updated_at":"test","root":dir.join("project"),"modules":[{"name":"stored-module","path":"module","kinds":["Node"],"build_cmd":"npm run build","has_dockerfile":false,"default_port":3000}]});
        fs::write(
            dir.join("selected-work/state.json"),
            serde_json::to_string(&state).unwrap(),
        )
        .unwrap();
        Self { dir }
    }
    fn run(&self, relative_root: bool, args: &[&str]) -> String {
        let root = if relative_root {
            PathBuf::from("project")
        } else {
            self.dir.join("project")
        };
        let output = Command::new(env!("CARGO_BIN_EXE_ved"))
            .env_clear()
            .current_dir(&self.dir)
            .arg("-d")
            .arg(root)
            .args([
                "--config-dir",
                "selected-config",
                "--work-dir",
                "selected-work",
            ])
            .args(if args.contains(&"--output-dir") {
                vec![]
            } else {
                vec!["--output-dir", "selected-output"]
            })
            .args(args)
            .env("VED_CONFIG_DIR", self.dir.join("wrong-config"))
            .env("VED_WORK_DIR", self.dir.join("wrong-work"))
            .env("VED_OUTPUT_DIR", self.dir.join("wrong-output"))
            .env(
                "PATH",
                format!("{}:{}", self.dir.join("bin").display(), "/usr/bin:/bin"),
            )
            .env("TEST_TRACE", self.dir.join("trace"))
            .env_remove("DEPLOYMENT_TARGET")
            .env_remove("TARGET")
            .env_remove("COMPOSE_PATH")
            .env_remove("INGRESS_SUFFIX")
            .env_remove("K8S_APP_SLOT")
            .env_remove("NPM_BIN")
            .env_remove("VED_AUTO_BUILD")
            .env_remove("OTEL_ENABLED")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
    fn trace(&self) -> String {
        fs::read_to_string(self.dir.join("trace")).unwrap()
    }
    fn assert_paths(&self, record: &str, cwd: &Path) {
        for (key, path) in [
            ("CWD", cwd.to_path_buf()),
            ("CONFIG", self.dir.join("selected-config")),
            ("WORK", self.dir.join("selected-work")),
            ("OUTPUT", self.dir.join("selected-output")),
        ] {
            assert!(
                record
                    .lines()
                    .any(|line| line == format!("{key}={}", path.display())),
                "missing {key}: {record}"
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn compose_commands_use_cli_context_for_absolute_and_relative_roots() {
    for relative in [false, true] {
        let f = Fixture::new();
        f.run(relative, &["compile"]);
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            &fs::read_to_string(f.dir.join("selected-output/docker-compose.yml")).unwrap(),
        )
        .unwrap();
        assert!(
            serde_yaml::to_string(&yaml["services"]["web"]["labels"])
                .unwrap()
                .contains("web.local.example"),
            "{yaml:?}"
        );
        for args in [
            vec!["up"],
            vec!["down"],
            vec!["ps"],
            vec!["reset"],
            vec!["test"],
            vec!["build", "--module", "stored-module"],
        ] {
            f.run(relative, &args);
        }
        let trace = f.trace();
        let records: Vec<_> = trace
            .split("BEGIN\n")
            .filter(|s| !s.trim().is_empty())
            .collect();
        assert_eq!(records.len(), 6, "{trace}");
        for record in &records {
            let build = record.contains("EXE=fake-build");
            f.assert_paths(
                record,
                &f.dir.join(if build { "project/module" } else { "project" }),
            );
            if !build {
                assert!(record.contains(&format!(
                    "ARG={}\n",
                    f.dir.join("selected-output/docker-compose.yml").display()
                )));
                assert!(record.contains(&format!(
                    "ARG={}\n",
                    f.dir.join("selected-config/.env").display()
                )));
                assert!(record.contains(&format!(
                    "ARG={}\n",
                    f.dir.join("selected-config/.local.env").display()
                )));
            }
        }
        assert!(records[0].contains("PROFILES=core,otel"));
        assert!(trace.contains("ARG=run\nARG=--rm\nARG=smoke-test"));
        assert!(trace.contains("EXE=fake-build"));
        assert!(!f.dir.join("wrong-output").exists());
        assert!(!f.dir.join("project/.ved/state.json").exists());
    }
}

#[test]
fn kubernetes_commands_use_selected_context_and_existing_k8s_output_root() {
    let f = Fixture::new();
    // The artifact-layout special case must be used by deployment too.
    let out = f.dir.join("selected-output/k8s");
    for args in [
        vec!["compile", "-t", "k8s"],
        vec!["up", "-t", "k8s"],
        vec!["down", "-t", "k8s"],
        vec!["ps", "-t", "k8s"],
        vec!["reset", "-t", "k8s"],
        vec!["test", "-t", "k8s"],
    ] {
        let mut actual = vec!["--output-dir", out.to_str().unwrap()];
        actual.extend(args);
        f.run(true, &actual);
    }
    let trace = f.trace();
    assert!(
        trace.contains(&format!(
            "ARG={}\n",
            out.join("overlays/local/ns-7").display()
        )),
        "{trace}"
    );
    assert!(!trace.contains("k8s/k8s"));
    assert!(trace.contains("ARG=ns-7"));
    for record in trace.split("BEGIN\n").filter(|s| !s.trim().is_empty()) {
        assert!(record.contains("EXE=kubectl"));
        assert!(record.contains(&format!("CWD={}\n", f.dir.join("project").display())));
        assert!(record.contains(&format!(
            "CONFIG={}\n",
            f.dir.join("selected-config").display()
        )));
        assert!(record.contains(&format!("WORK={}\n", f.dir.join("selected-work").display())));
        assert!(record.contains(&format!("OUTPUT={}\n", out.display())));
    }
}
