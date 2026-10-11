mod support;
use std::fs;

#[test]
fn test_cli_slot_flag_validation_and_rejection() {
    let tmp = std::env::temp_dir().join(format!("ved-slot-validation-{}", std::process::id()));
    let ved_bin = env!("CARGO_BIN_EXE_ved");

    let output = support::command(ved_bin, &tmp)
        .args(["ps", "--slot", "UpperSlot"])
        .output()
        .expect("exec ved ps");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid character 'U' in slot") || stderr.contains("invalid value"));

    let output = support::command(ved_bin, &tmp)
        .args(["ps", "--slot", "-bad"])
        .output()
        .expect("exec ved ps");
    assert_eq!(output.status.code(), Some(2));

    let output = support::command(ved_bin, &tmp)
        .args(["ps", "--slot", "slot@1"])
        .output()
        .expect("exec ved ps");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid character '@' in slot") || stderr.contains("invalid value"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_cli_validate_with_spec_file_flag() {
    let tmp = std::env::temp_dir().join(format!("test_cli_spec_flag_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let spec_content = r#"
app = App(name = "demo-app", version = "1.0.0")
app.services.add(Service(name = "web", image = "nginx:alpine"))
"#;
    let custom_spec = tmp.join("custom_spec.ved");
    fs::write(&custom_spec, spec_content).unwrap();

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let output = support::command(ved_bin, &tmp)
        .args(["validate", "-f", custom_spec.to_str().unwrap()])
        .output()
        .expect("exec ved validate");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("is valid"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_cli_compile_with_typed_options() {
    let tmp = std::env::temp_dir().join(format!("test_cli_compile_typed_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let spec_content = r#"
app = App(name = "demo-compile", version = "1.0.0")
app.services.add(Service(name = "api", image = "api:v1", ports = [Port(8080, host = 8080)]))
"#;
    let vedfile = tmp.join("Vedfile");
    fs::write(&vedfile, spec_content).unwrap();

    let ved_bin = env!("CARGO_BIN_EXE_ved");
    let out_dir = tmp.join("generated_output");

    let output = support::command(ved_bin, &tmp)
        .current_dir(&tmp)
        .args([
            "compile",
            "-t",
            "compose",
            "-o",
            out_dir.to_str().unwrap(),
            "--org",
            "testorg",
        ])
        .output()
        .expect("exec ved compile");

    assert!(output.status.success());
    assert!(out_dir.join("docker-compose.yml").exists());

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_cli_smoke_test_alias_and_workload_flag() {
    let tmp = std::env::temp_dir().join(format!("test_cli_smoke_alias_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);

    let spec_content = r#"
app = App(name = "demo-test", version = "1.0.0")
app.services.add(Service(name = "srv", image = "srv:v1"))
"#;
    let vedfile = tmp.join("Vedfile");
    fs::write(&vedfile, spec_content).unwrap();

    let ved_bin = env!("CARGO_BIN_EXE_ved");

    let output = support::command(ved_bin, &tmp)
        .args(["smoke-test", "--help"])
        .output()
        .expect("exec ved smoke-test --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Run test workload against active stack"));
    assert!(stdout.contains("[WORKLOAD]"));

    let _ = fs::remove_dir_all(&tmp);
}
