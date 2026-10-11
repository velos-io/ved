mod support;
use std::fs;

fn get_ved_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ved")
}

fn create_sample_spec(dir: &std::path::Path) {
    let spec_content = r#"
from ved import App, Port, Service

app = App(
    name = "decouple-app",
    version = "1.0.0",
)

app.services.add(
    Service(
        name = "api",
        image = "nginx:alpine",
        ports = [Port(8080)],
    )
)
"#;
    fs::write(dir.join("Vedfile"), spec_content).expect("write Vedfile");
    let config_dir = dir.join("config");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join(".env"), "APP_ENV=test\n").expect("write .env");
}

#[test]
fn test_ac2_unified_output_directory() {
    let tmp = std::env::temp_dir().join(format!("test_ac2_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let root = tmp.join("app");
    let out = tmp.join("dist");
    fs::create_dir_all(&root).unwrap();
    create_sample_spec(&root);

    let status = support::command(get_ved_bin(), &tmp)
        .arg("-d")
        .arg(&root)
        .arg("-o")
        .arg(&out)
        .args(["compile", "-t", "all"])
        .status()
        .expect("run compile");

    assert!(status.success(), "compile with -o failed");

    assert!(
        out.join("docker-compose.yml").exists(),
        "docker-compose.yml must exist in unified output"
    );
    assert!(
        out.join("k8s").exists(),
        "k8s directory must exist in unified output"
    );
    assert!(
        out.join("values.yaml").exists(),
        "values.yaml must exist in unified output"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_ac3_legacy_output_fallback_when_output_dir_omitted() {
    let tmp = std::env::temp_dir().join(format!("test_ac3_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let root = tmp.join("app");
    fs::create_dir_all(&root).unwrap();
    create_sample_spec(&root);

    let status = support::command(get_ved_bin(), &tmp)
        .arg("-d")
        .arg(&root)
        .args(["compile", "-t", "all"])
        .status()
        .expect("run compile");

    assert!(status.success(), "compile without -o failed");

    assert!(
        root.join(".ved/docker-compose.yml").exists(),
        "legacy compose path must exist"
    );
    assert!(
        root.join("installer/k8s").exists(),
        "legacy k8s path must exist"
    );
    assert!(
        root.join("installer/values.yaml").exists(),
        "legacy helm path must exist"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_ac1_and_ac5_precedence_and_fail_fast_config_dir() {
    let tmp = std::env::temp_dir().join(format!("test_ac1_ac5_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let root = tmp.join("app");
    fs::create_dir_all(&root).unwrap();
    create_sample_spec(&root);

    let out_fail = support::command(get_ved_bin(), &tmp)
        .arg("-d")
        .arg(&root)
        .arg("--config-dir")
        .arg(tmp.join("does-not-exist"))
        .arg("compile")
        .output()
        .expect("run compile with missing config");

    assert_eq!(out_fail.status.code(), Some(1), "must exit with code 1");
    let stderr = String::from_utf8_lossy(&out_fail.stderr);
    assert!(
        stderr.contains("config directory not found"),
        "expected config directory error, got: {stderr}"
    );

    let custom_work = tmp.join("auto_created_work");
    let custom_out = tmp.join("auto_created_out");
    assert!(!custom_work.exists());
    assert!(!custom_out.exists());

    let status = support::command(get_ved_bin(), &tmp)
        .arg("-d")
        .arg(&root)
        .arg("--work-dir")
        .arg(&custom_work)
        .arg("-o")
        .arg(&custom_out)
        .args(["compile", "-t", "compose"])
        .status()
        .expect("run compile");

    assert!(status.success());
    assert!(
        custom_work.exists(),
        "work-dir must be automatically created"
    );
    assert!(
        custom_out.exists(),
        "output-dir must be automatically created"
    );

    let cli_work = tmp.join("cli_work");
    let env_work = tmp.join("env_work");

    let status_precedence = support::command(get_ved_bin(), &tmp)
        .arg("-d")
        .arg(&root)
        .arg("--work-dir")
        .arg(&cli_work)
        .env("VED_WORK_DIR", &env_work)
        .args(["compile", "-t", "compose"])
        .status()
        .expect("run compile with env and flag");

    assert!(status_precedence.success());
    assert!(cli_work.exists(), "CLI work dir should be created");
    assert!(
        !env_work.exists(),
        "Env work dir should not be created because CLI takes precedence"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[cfg(unix)]
#[test]
fn test_ac6_read_only_root_execution() {
    use std::os::unix::fs::PermissionsExt;

    let is_root = rustix::process::geteuid().is_root();
    if is_root {
        return;
    }

    let tmp = std::env::temp_dir().join(format!("test_ac6_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let root = tmp.join("readonly_app");
    let work = tmp.join("writable_work");
    let out = tmp.join("writable_out");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&out).unwrap();

    create_sample_spec(&root);

    // Make root read-only (r-xr-xr-x = 0o555)
    let ro_perms = fs::Permissions::from_mode(0o555);
    fs::set_permissions(&root, ro_perms).unwrap();

    let output = support::command(get_ved_bin(), &tmp)
        .arg("-d")
        .arg(&root)
        .arg("--work-dir")
        .arg(&work)
        .arg("-o")
        .arg(&out)
        .args(["compile", "-t", "all"])
        .output()
        .expect("run compile on read-only root");

    // Restore write permissions so tmp can be cleaned up
    let rw_perms = fs::Permissions::from_mode(0o755);
    let _ = fs::set_permissions(&root, rw_perms);

    assert!(
        output.status.success(),
        "Compile should succeed on read-only root. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.join("docker-compose.yml").exists());
    assert!(out.join("k8s").exists());
    assert!(out.join("values.yaml").exists());
    assert!(work.join(".generated.env").exists());

    let _ = fs::remove_dir_all(&tmp);
}
