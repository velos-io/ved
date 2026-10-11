mod support;
use std::fs;

#[test]
fn compile_uses_external_writable_artifact_root_for_read_only_sources() {
    let temp = std::env::temp_dir().join(format!("ved-cli-readonly-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    let source = temp.join("source");
    let output = temp.join("output");
    fs::create_dir_all(source.join("config")).unwrap();
    fs::create_dir_all(&output).unwrap();
    fs::write(
        source.join("ved.yaml"),
        "app: readonly\nversion: '1'\nservices: {}\n",
    )
    .unwrap();
    let result = support::command(env!("CARGO_BIN_EXE_ved"), &temp)
        .args([
            "--dir",
            source.to_str().unwrap(),
            "--output-dir",
            output.to_str().unwrap(),
            "compile",
            "--target",
            "compose",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(output.join("docker-compose.yml").is_file());
    assert!(output.join(".generated.env").is_file());
    assert!(output.join(".ved/artifact-manifest.json").is_file());
    assert!(!source.join(".ved").exists());
    assert!(!source.join("config/.generated.env").exists());
    assert!(!source.join("config/.env").exists());
    let _ = fs::remove_dir_all(&temp);
}
