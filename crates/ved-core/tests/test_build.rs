#![cfg(feature = "host")]
mod support;
use std::collections::BTreeMap;
use std::fs;
use ved_core::build::{build_all_with_context, resolve_module_command};
use ved_core::{scan_workspace, DetectedModule, ModuleKind};

#[test]
fn test_build_execution_with_mock_scripts() {
    let tmp = std::env::temp_dir().join(format!("ved-build-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::create_dir_all(&tmp);

    let node_dir = tmp.join("web");
    fs::create_dir_all(&node_dir).unwrap();
    fs::write(
        node_dir.join("package.json"),
        r#"{"name":"web","scripts":{"build":"echo ok"}}"#,
    )
    .unwrap();

    let bin_dir = tmp.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let mock_npm = bin_dir.join("mock-npm.sh");
    fs::write(
        &mock_npm,
        "#!/bin/sh\necho 'mock npm built successfully'\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mock_npm).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_npm, perms).unwrap();
    }

    let context = support::context(
        &tmp,
        BTreeMap::from([("NPM_BIN".into(), mock_npm.to_string_lossy().into_owned())]),
    );

    let state = scan_workspace(&tmp);
    assert!(state.modules.iter().any(|m| m.name == "web"));

    let res = build_all_with_context(&context, &tmp, Some("web"));
    assert!(res.is_ok(), "build_all failed: {res:?}");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_build_failure_propagation() {
    let tmp = std::env::temp_dir().join(format!("ved-build-fail-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::create_dir_all(&tmp);

    let go_dir = tmp.join("api");
    fs::create_dir_all(&go_dir).unwrap();
    fs::write(go_dir.join("go.mod"), "module example.com/api\ngo 1.22\n").unwrap();

    let bin_dir = tmp.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let mock_go = bin_dir.join("mock-go.sh");
    fs::write(&mock_go, "#!/bin/sh\necho 'compile error' >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mock_go).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_go, perms).unwrap();
    }

    let context = support::context(
        &tmp,
        BTreeMap::from([("GO_BIN".into(), mock_go.to_string_lossy().into_owned())]),
    );

    let res = build_all_with_context(&context, &tmp, Some("api"));
    assert!(res.is_err(), "build_all should have failed");
    let err_str = res.unwrap_err().to_string();
    assert!(
        err_str.contains("failed with exit code"),
        "Error: {err_str}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_resolve_wrapper_precedence() {
    let tmp = std::env::temp_dir().join(format!("ved-build-wrapper-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::create_dir_all(&tmp);

    let gradle_dir = tmp.join("backend");
    fs::create_dir_all(&gradle_dir).unwrap();
    fs::write(gradle_dir.join("build.gradle.kts"), "").unwrap();
    let gradlew = gradle_dir.join("gradlew");
    fs::write(&gradlew, "#!/bin/sh\nexit 0\n").unwrap();

    let module = DetectedModule {
        name: "backend".to_string(),
        path: "./backend".to_string(),
        kinds: vec![ModuleKind::Gradle],
        build_cmd: "gradle build".to_string(),
        has_dockerfile: false,
        default_port: 8080,
    };

    let (prog, _) = resolve_module_command(&module, &gradle_dir);
    assert_eq!(prog, "./gradlew");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_get_build_prop_delegation() {
    let tmp = std::env::temp_dir().join(format!("ved-build-prop-{}", std::process::id()));
    let context = support::context(
        &tmp,
        BTreeMap::from([(
            "TEST_TOOLCHAIN_BIN".into(),
            "/usr/local/bin/custom-tool".into(),
        )]),
    );
    assert_eq!(
        context.config().get_prop_opt("TEST_TOOLCHAIN_BIN"),
        Some("/usr/local/bin/custom-tool".to_string())
    );
    assert_eq!(
        context.config().get_prop_opt("TEST_TOOLCHAIN_NON_EXISTENT"),
        None
    );
}

#[test]
fn test_build_all_concurrent_execution() {
    let tmp = std::env::temp_dir().join(format!("ved-build-all-conc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::create_dir_all(&tmp);

    let mod_a = tmp.join("app-a");
    fs::create_dir_all(&mod_a).unwrap();
    fs::write(mod_a.join("build.gradle"), "").unwrap();
    let gradlew_a = mod_a.join("gradlew");
    fs::write(&gradlew_a, "#!/bin/sh\nexit 0\n").unwrap();

    let mod_b = tmp.join("app-b");
    fs::create_dir_all(&mod_b).unwrap();
    fs::write(mod_b.join("build.gradle"), "").unwrap();
    let gradlew_b = mod_b.join("gradlew");
    fs::write(&gradlew_b, "#!/bin/sh\nexit 0\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms_a = fs::metadata(&gradlew_a).unwrap().permissions();
        perms_a.set_mode(0o755);
        fs::set_permissions(&gradlew_a, perms_a).unwrap();

        let mut perms_b = fs::metadata(&gradlew_b).unwrap().permissions();
        perms_b.set_mode(0o755);
        fs::set_permissions(&gradlew_b, perms_b).unwrap();
    }

    let res = ved_core::build::build_all_concurrent(&tmp, None, 4);
    assert!(res.is_ok(), "build_all_concurrent failed: {res:?}");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_build_all_concurrent_multi_failure() {
    let tmp = std::env::temp_dir().join(format!("ved-build-all-fail-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::create_dir_all(&tmp);

    let mod_a = tmp.join("fail-a");
    fs::create_dir_all(&mod_a).unwrap();
    fs::write(mod_a.join("build.gradle"), "").unwrap();
    let gradlew_a = mod_a.join("gradlew");
    fs::write(&gradlew_a, "#!/bin/sh\necho 'fail a' >&2\nexit 1\n").unwrap();

    let mod_b = tmp.join("fail-b");
    fs::create_dir_all(&mod_b).unwrap();
    fs::write(mod_b.join("build.gradle"), "").unwrap();
    let gradlew_b = mod_b.join("gradlew");
    fs::write(&gradlew_b, "#!/bin/sh\necho 'fail b' >&2\nexit 1\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms_a = fs::metadata(&gradlew_a).unwrap().permissions();
        perms_a.set_mode(0o755);
        fs::set_permissions(&gradlew_a, perms_a).unwrap();

        let mut perms_b = fs::metadata(&gradlew_b).unwrap().permissions();
        perms_b.set_mode(0o755);
        fs::set_permissions(&gradlew_b, perms_b).unwrap();
    }

    let res = ved_core::build::build_all_concurrent(&tmp, None, 2);
    assert!(
        res.is_err(),
        "build_all_concurrent should fail on multiple errors"
    );
    let err_str = res.unwrap_err().to_string();
    assert!(err_str.contains("Failed to build"), "Error was: {err_str}");

    let _ = fs::remove_dir_all(&tmp);
}

#[cfg(unix)]
#[test]
fn concurrent_build_failures_are_reported_in_module_order() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = std::env::temp_dir().join(format!("ved-build-order-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    for (name, script) in [
        ("fail-a", "sleep 0.2\necho 'failure-a' >&2\nexit 1"),
        ("fail-b", "echo 'failure-b' >&2\nexit 1"),
    ] {
        let module = tmp.join(name);
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("build.gradle"), "").unwrap();
        let wrapper = module.join("gradlew");
        fs::write(&wrapper, format!("#!/bin/sh\n{script}\n")).unwrap();
        let mut permissions = fs::metadata(&wrapper).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(wrapper, permissions).unwrap();
    }

    let error = ved_core::build::build_all_concurrent(&tmp, None, 2).unwrap_err();
    let display = error.to_string();
    assert!(display.contains("[fail-a]"), "{display}");
    assert!(display.contains("[fail-b]"), "{display}");
    assert!(display.find("[fail-a]").unwrap() < display.find("[fail-b]").unwrap());
    let failures = error
        .build_failure_sources()
        .expect("aggregate keeps typed module errors")
        .collect::<Vec<_>>();
    assert_eq!(
        failures.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["fail-a", "fail-b"]
    );
    assert!(failures
        .iter()
        .all(|(_, source)| matches!(source, ved_core::VedError::Build { .. })));
    let _ = fs::remove_dir_all(&tmp);
}

#[cfg(unix)]
#[test]
fn cancelling_build_batch_stops_active_and_unscheduled_modules() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let tmp = std::env::temp_dir().join(format!("ved-build-cancel-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    for name in ["a-running", "b-pending", "c-pending"] {
        let module = tmp.join(name);
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("build.gradle"), "").unwrap();
        let script = if name == "a-running" {
            "touch started\nwhile :; do sleep 1; done"
        } else {
            "touch started\nexit 0"
        };
        let wrapper = module.join("gradlew");
        fs::write(&wrapper, format!("#!/bin/sh\n{script}\n")).unwrap();
        let mut permissions = fs::metadata(&wrapper).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(wrapper, permissions).unwrap();
    }

    let token = ved_process::CancellationToken::new();
    let cancel = token.clone();
    let root = tmp.clone();
    let handle = std::thread::spawn(move || {
        let context = ved_core::WorkspaceContext::new(&root);
        ved_core::build::build_all_concurrent_with_context_and_sink_and_control(
            &context,
            &root,
            None,
            1,
            &ved_core::SilentSink,
            None,
            cancel,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !tmp.join("a-running/started").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        tmp.join("a-running/started").exists(),
        "build did not start"
    );
    token.cancel();
    let error = handle.join().unwrap().unwrap_err();
    let display = error.to_string();
    assert!(display.contains("a-running"), "{display}");
    assert!(display.contains("b-pending"), "{display}");
    assert!(display.contains("c-pending"), "{display}");
    assert!(!tmp.join("b-pending/started").exists());
    assert!(!tmp.join("c-pending/started").exists());
    let _ = fs::remove_dir_all(&tmp);
}
