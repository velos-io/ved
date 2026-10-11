#[test]
fn test_version_output_includes_timestamp() {
    let ved_bin = env!("CARGO_BIN_EXE_ved");

    let out_cmd = std::process::Command::new(ved_bin)
        .env_clear()
        .arg("version")
        .output()
        .expect("Failed to execute ved version");
    assert!(out_cmd.status.success());
    let stdout_cmd = String::from_utf8_lossy(&out_cmd.stdout);
    let expected_prefix = format!("ved {} (built ", env!("CARGO_PKG_VERSION"));
    assert!(stdout_cmd.starts_with(&expected_prefix));
    assert!(stdout_cmd.trim().ends_with(')'));

    let out_flag = std::process::Command::new(ved_bin)
        .env_clear()
        .arg("--version")
        .output()
        .expect("Failed to execute ved --version");
    assert!(out_flag.status.success());
    let stdout_flag = String::from_utf8_lossy(&out_flag.stdout);
    assert!(stdout_flag.starts_with(&expected_prefix));
    assert!(stdout_flag.trim().ends_with(')'));
}
