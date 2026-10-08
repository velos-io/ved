#![cfg(all(feature = "host", unix))]
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use ved_core::build::{build_workspace, BuildOptions};
use ved_core::{SilentSink, VedError};
use ved_process::CancellationToken;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ved-options-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn module(&self, name: &str, script: &str) {
        let dir = self.0.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("build.gradle"), "").unwrap();
        let wrapper = dir.join("gradlew");
        fs::write(&wrapper, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn run(&self, options: &BuildOptions) -> ved_core::Result<()> {
        let context = support::context(&self.0, BTreeMap::new());
        build_workspace(&context, &self.0, options, &SilentSink)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn defaults_are_fresh_and_clones_share_cancellation() {
    let first = BuildOptions::default();
    let second = BuildOptions::new();
    assert!(first.selected_module().is_none());
    assert_eq!(
        first.concurrency_limit(),
        ved_core::build::default_concurrency()
    );
    assert!(first.timeout_override().is_none());
    let cloned = first.clone();
    cloned.cancellation().cancel();
    assert!(first.cancellation().is_cancelled());
    assert!(!second.cancellation().is_cancelled());
    let explicit = CancellationToken::new();
    let options = second
        .module("web")
        .concurrency(2)
        .unwrap()
        .timeout(Duration::from_secs(2))
        .unwrap()
        .cancellation_token(explicit.clone());
    assert_eq!(options.selected_module(), Some("web"));
    assert_eq!(options.concurrency_limit(), 2);
    assert_eq!(options.timeout_override(), Some(Duration::from_secs(2)));
    explicit.cancel();
    assert!(options.cancellation().is_cancelled());
    let reset = options.all_modules().configured_timeout();
    assert!(reset.selected_module().is_none());
    assert!(reset.timeout_override().is_none());
}

#[test]
fn invalid_options_fail_before_discovery_or_execution() {
    // Neither setter accepts a workspace, so validation cannot scan it or launch a tool.
    assert!(BuildOptions::new().concurrency(0).is_err());
    assert!(BuildOptions::new().timeout(Duration::ZERO).is_err());
}

#[test]
fn selection_by_name_path_missing_and_all_modules() {
    let fixture = Fixture::new();
    fixture.module("a", "touch built");
    fixture.module("b", "touch built");
    fixture.run(&BuildOptions::new().module("a")).unwrap();
    assert!(fixture.0.join("a/built").exists());
    assert!(!fixture.0.join("b/built").exists());
    fixture.run(&BuildOptions::new().module("./b")).unwrap();
    assert!(fixture.0.join("b/built").exists());
    fs::remove_file(fixture.0.join("a/built")).unwrap();
    fs::remove_file(fixture.0.join("b/built")).unwrap();
    assert!(fixture.run(&BuildOptions::new().module("missing")).is_err());
    assert!(!fixture.0.join("a/built").exists());
    assert!(!fixture.0.join("b/built").exists());
    fixture
        .run(&BuildOptions::new().module("missing").all_modules())
        .unwrap();
    assert!(fixture.0.join("a/built").exists());
    assert!(fixture.0.join("b/built").exists());
}

// A scoped worker always gets cancellation and a finite child timeout, including when
// an assertion panics. This guard cancels before the scoped thread is joined.
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
fn wait_for(path: &std::path::Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    path.exists()
}

#[test]
fn concurrency_limit_holds_pending_modules_until_a_slot_opens() {
    let fixture = Fixture::new();
    for name in ["a", "b", "c"] {
        fixture.module(
            name,
            "if mkdir ../slot1 2>/dev/null; then slot=../slot1; elif mkdir ../slot2 2>/dev/null; then slot=../slot2; else touch ../overflow; exit 9; fi\ntouch started\nwhile [ ! -f ../release ]; do sleep 0.02; done\nrmdir \"$slot\"\ntouch built",
        );
    }
    let options = BuildOptions::new()
        .concurrency(2)
        .unwrap()
        .timeout(Duration::from_secs(8))
        .unwrap();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| fixture.run(&options));
        let _cleanup = CancelOnDrop(options.cancellation().clone());
        assert!(wait_for(&fixture.0.join("a/started")));
        assert!(wait_for(&fixture.0.join("b/started")));
        assert!(!fixture.0.join("c/started").exists());
        fs::write(fixture.0.join("release"), "").unwrap();
        worker.join().unwrap().unwrap();
    });
    assert!(fixture.0.join("c/built").exists());
    assert!(!fixture.0.join("overflow").exists());
}

#[test]
fn cancellation_stops_active_and_pending_options_builds() {
    let fixture = Fixture::new();
    fixture.module("a", "touch started\nwhile :; do sleep 1; done");
    fixture.module("b", "touch started");
    let options = BuildOptions::new()
        .concurrency(1)
        .unwrap()
        .timeout(Duration::from_secs(8))
        .unwrap();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| fixture.run(&options));
        let _cleanup = CancelOnDrop(options.cancellation().clone());
        assert!(wait_for(&fixture.0.join("a/started")));
        options.cancellation().cancel();
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error.build_failure_sources().unwrap().count(), 2);
    });
    assert!(!fixture.0.join("b/started").exists());
    fs::remove_file(fixture.0.join("a/started")).unwrap();
    assert!(fixture.run(&options).is_err());
    assert!(!fixture.0.join("a/started").exists());
}

#[test]
fn explicit_timeout_preserves_typed_process_failure() {
    let fixture = Fixture::new();
    fixture.module("slow", "touch started\nwhile :; do sleep 1; done");
    let start = Instant::now();
    let error = fixture
        .run(
            &BuildOptions::new()
                .timeout(Duration::from_millis(150))
                .unwrap(),
        )
        .unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(
        matches!(
            error,
            VedError::ProcessOperation {
                source: ved_process::RunError::TimedOut { .. },
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn failures_follow_module_order_despite_reversed_completion() {
    let fixture = Fixture::new();
    fixture.module(
        "a",
        "while [ ! -f ../b/finished ]; do sleep 0.02; done\nexit 1",
    );
    fixture.module("b", "touch finished\nexit 2");
    let options = BuildOptions::new()
        .concurrency(2)
        .unwrap()
        .timeout(Duration::from_secs(5))
        .unwrap();
    let error = fixture.run(&options).unwrap_err();
    let failures: Vec<_> = error.build_failure_sources().unwrap().collect();
    assert_eq!(
        failures.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(failures
        .iter()
        .all(|(_, error)| matches!(error, VedError::Build { .. })));
}

#[test]
fn legacy_zero_concurrency_and_zero_timeout_remain_compatible() {
    use ved_core::build::build_all_concurrent_with_context_and_sink_and_control;
    let fixture = Fixture::new();
    fixture.module("a", "touch built");
    fixture.module("b", "touch built");
    let context = support::context(&fixture.0, BTreeMap::new());
    build_all_concurrent_with_context_and_sink_and_control(
        &context,
        &fixture.0,
        None,
        0,
        &SilentSink,
        None,
        CancellationToken::new(),
    )
    .unwrap();
    assert!(fixture.0.join("a/built").exists());
    assert!(fixture.0.join("b/built").exists());
    fixture.module("slow", "while :; do sleep 1; done");
    let start = Instant::now();
    let error = build_all_concurrent_with_context_and_sink_and_control(
        &context,
        &fixture.0,
        Some("slow"),
        0,
        &SilentSink,
        Some(Duration::ZERO),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(
        matches!(
            error,
            VedError::ProcessOperation {
                source: ved_process::RunError::TimedOut { .. },
                ..
            }
        ),
        "{error:?}"
    );
}
