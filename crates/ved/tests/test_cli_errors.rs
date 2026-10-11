mod support;
use std::fs;

#[test]
fn test_missing_spec_file_returns_exit_code_2() {
    let tmp = std::env::temp_dir().join(format!("test_missing_spec_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("compile")
        .arg("--file")
        .arg("nonexistent-spec.ved")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved compile");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr,
        format!(
            "error: Specification file not found: {}\n",
            fs::canonicalize(&tmp)
                .unwrap()
                .join("nonexistent-spec.ved")
                .display()
        )
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_missing_spec_file_with_dir_flag_returns_exit_code_2() {
    let tmp = std::env::temp_dir().join(format!("test_missing_spec_dir_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("--dir")
        .arg(&tmp)
        .arg("compile")
        .output()
        .expect("Failed to execute ved compile");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr,
        format!(
            "error: Specification file not found: {}\n",
            tmp.join("Vedfile").display()
        )
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_invalid_target_in_env_returns_exit_code_2() {
    let tmp = std::env::temp_dir().join(format!("test_invalid_target_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("compile")
        .env("DEPLOYMENT_TARGET", "invalid-target-value")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved compile");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr,
        "error: Invalid DEPLOYMENT_TARGET: 'invalid-target-value'. Valid values are 'compose', 'k8s', 'helm', 'all'\n"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_invalid_target_fallback_in_env_returns_exit_code_2() {
    let tmp = std::env::temp_dir().join(format!(
        "test_invalid_target_fallback_{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("compile")
        .env_remove("DEPLOYMENT_TARGET")
        .env("TARGET", "invalid-target-value")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved compile");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr,
        "error: Invalid TARGET: 'invalid-target-value'. Valid values are 'compose', 'k8s', 'helm', 'all'\n"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_missing_compose_file_returns_exit_code_2() {
    let tmp = std::env::temp_dir().join(format!("test_missing_compose_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("init")
        .arg("--from-compose")
        .arg("does-not-exist.yml")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved init");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr,
        "error: Specified compose file does not exist: does-not-exist.yml\n"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_compose_discovery_failed_returns_exit_code_2() {
    let tmp = std::env::temp_dir().join(format!("test_compose_discovery_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("init")
        .arg("--compose")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved init --compose");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr,
        format!("error: No docker-compose file found in {} (checked docker-compose.yml, docker-compose.yaml, compose.yml, compose.yaml)\n", fs::canonicalize(&tmp).unwrap().display())
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_validation_failed_returns_exit_code_2() {
    let tmp = std::env::temp_dir().join(format!("test_validate_failed_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    fs::write(
        tmp.join("Vedfile"),
        "this is not valid starlark or syntax ((( ;;",
    )
    .unwrap();

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("validate")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved validate");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with(&format!(
        "error: Validation failed for {}:",
        fs::canonicalize(&tmp).unwrap().join("Vedfile").display()
    )));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_validation_failed_for_malformed_mount_in_yaml() {
    let tmp = std::env::temp_dir().join(format!("test_validate_mount_yaml_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let invalid_spec = r#"
app: test-invalid-mount
version: 1.0.0
services:
  web:
    image: nginx:alpine
    mounts:
      - bad-mount-syntax
"#;
    fs::write(tmp.join("ved.yaml"), invalid_spec).unwrap();

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("validate")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved validate");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&format!(
        "Validation failed for {}",
        fs::canonicalize(&tmp).unwrap().join("ved.yaml").display()
    )));
    assert!(stderr.contains("Service 'web': invalid mounts[0]"));
    assert!(!stderr.contains("bad-mount-syntax"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_validation_failed_for_malformed_mount_in_starlark() {
    let tmp = std::env::temp_dir().join(format!(
        "test_validate_mount_starlark_{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);

    let invalid_vedfile = r#"
app = App(name="test-invalid-mount", version="1.0.0")
app.services.add(
    Service(
        name="web",
        image="nginx:alpine",
        mounts=["Invalid_PVC!:/var/data"],
    )
)
"#;
    fs::write(tmp.join("Vedfile"), invalid_vedfile).unwrap();

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .arg("validate")
        .current_dir(&tmp)
        .output()
        .expect("Failed to execute ved validate");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&format!(
        "Validation failed for {}",
        fs::canonicalize(&tmp).unwrap().join("Vedfile").display()
    )));
    assert!(stderr.contains("Service 'web': invalid mounts[0]"));
    assert!(!stderr.contains("Invalid_PVC!"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn validate_redacts_interpolated_yaml_and_starlark_diagnostics() {
    let root =
        std::env::temp_dir().join(format!("ved-structured-redaction-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    for (file, content) in [
        (
            "bad.yaml",
            "app: safe\nversion: '1'\nservices:\n  web:\n    port: ${SECRET_PORT}\n",
        ),
        ("bad.star", "fail('SECRET-STARLARK-CREDENTIAL')\n"),
    ] {
        fs::write(root.join(file), content).unwrap();
        let output = support::command(env!("CARGO_BIN_EXE_ved"), &root)
            .arg("validate")
            .arg("--file")
            .arg(file)
            .current_dir(&root)
            .env("SECRET_PORT", "SECRET-YAML-CREDENTIAL")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(file), "{stderr}");
        assert!(!stderr.contains("SECRET-YAML-CREDENTIAL"), "{stderr}");
        assert!(!stderr.contains("SECRET-STARLARK-CREDENTIAL"), "{stderr}");
        assert!(output.stdout.is_empty());
    }
    fs::remove_dir_all(root).unwrap();
}
