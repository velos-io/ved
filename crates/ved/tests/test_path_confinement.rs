mod support;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "ved-cli-confinement-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn compile(&self, name: &str, content: &str) -> std::process::Output {
        let path = self.0.join(name);
        fs::write(&path, content).unwrap();
        support::command(env!("CARGO_BIN_EXE_ved"), &self.0)
            .args([
                "-d",
                self.0.to_str().unwrap(),
                "compile",
                "-f",
                path.to_str().unwrap(),
                "-t",
                "k8s",
                "--output-dir",
                self.0.join("out").to_str().unwrap(),
            ])
            .env_remove("VED_CONFIG_DIR")
            .env_remove("VED_WORK_DIR")
            .env_remove("VED_OUTPUT_DIR")
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_rejects_yaml_and_starlark_traversal_without_spec_side_effects() {
    let yaml = Fixture::new();
    let result = yaml.compile("spec.yaml", "app: audit\nversion: '1'\nservices:\n  web:\n    image: nginx\n    group: ../../../escaped\n");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid group"));
    assert!(!yaml.0.join("escaped").exists());
    assert!(!yaml.0.join("out/k8s").exists());
    assert!(!yaml.0.join("config").exists());

    let starlark = Fixture::new();
    let result = starlark.compile("Vedfile", "app = App(name='audit', version='1', network='audit')\nStack.add('../escape')\napp.services.add(Service(name='web', image='nginx'))\n");
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("Vedfile"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!starlark.0.join(".ved/stacks").exists());
    assert!(!starlark.0.join("config").exists());
}

#[cfg(unix)]
#[test]
fn cli_rejects_symlinked_k8s_output_boundary() {
    let fixture = Fixture::new();
    let outside = fixture.0.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(fixture.0.join("out")).unwrap();
    fs::write(outside.join("sentinel"), "untouched").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.0.join("out/k8s")).unwrap();
    let result = fixture.compile(
        "spec.yaml",
        "app: audit\nversion: '1'\nenv_files: []\nservices:\n  web:\n    image: nginx\n",
    );
    assert!(!result.status.success());
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).unwrap(),
        "untouched"
    );
    assert_eq!(fs::read_dir(outside).unwrap().count(), 1);
}
