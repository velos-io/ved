#![cfg(feature = "host")]
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use ved_core::{LoadOptions, ResourceBudget, ResourceLimitKind};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ved-eval-budget-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn limit(&self, source: &str, budget: ResourceBudget, expected: ResourceLimitKind) {
        let path = self.0.join("test.star");
        fs::write(&path, source).unwrap();
        let context = support::context(&self.0, BTreeMap::new());
        let options = LoadOptions::new(budget).unwrap();
        let error = ved_core::load_any_spec_with_context_and_dependencies_and_options(
            &path, &context, &options,
        )
        .unwrap_err();
        let limit = error
            .resource_limit_error()
            .expect("typed resource limit retained");
        assert_eq!(limit.kind(), expected, "{error:?}");
        assert_eq!(limit.source_path(), path);
        assert!(error.is_validation_error());
        assert!(
            !self.0.join(".ved").exists(),
            "failed evaluation published assets"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn evaluators_enforce_cooperative_budgets_in_bounded_child() {
    const CHILD: &str = "VED_EVALUATION_BUDGET_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = ved_process::Runner::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "evaluators_enforce_cooperative_budgets_in_bounded_child",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .capture()
            .capture_limits(64 * 1024, 64 * 1024)
            .timeout(Duration::from_secs(15))
            .check_status(false)
            .run()
            .expect("bounded evaluator test child must finish");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = Fixture::new();
    let normal = "app = App(name='sample', version='1')\n";
    // Short evaluations return before the upstream 1000-tick periodic check.
    // The final usage check must still reject them.
    fixture.limit(
        normal,
        ResourceBudget {
            max_ticks: 1,
            ..Default::default()
        },
        ResourceLimitKind::Ticks,
    );
    fixture.limit(
        normal,
        ResourceBudget {
            max_heap_bytes: 1,
            ..Default::default()
        },
        ResourceLimitKind::HeapBytes,
    );
    let looping = "app = App(name='sample', version='1')\nx = [i for i in range(1000000)]\n";
    fixture.limit(
        looping,
        ResourceBudget {
            max_ticks: 100,
            ..Default::default()
        },
        ResourceLimitKind::Ticks,
    );
    let mut deep = String::new();
    for i in 0..20 {
        deep.push_str(&format!("def f{i}():\n    return f{}()\n", i + 1));
    }
    deep.push_str("def f20():\n    return 1\napp = App(name='sample', version='1')\nf0()\n");
    fixture.limit(
        &deep,
        ResourceBudget {
            max_callstack_frames: 8,
            ..Default::default()
        },
        ResourceLimitKind::CallstackFrames,
    );
    fixture.limit(
        normal,
        ResourceBudget {
            max_elapsed: Duration::from_nanos(1),
            ..Default::default()
        },
        ResourceLimitKind::Elapsed,
    );
}

#[test]
fn configured_legacy_loader_limits_are_enforced() {
    let fixture = Fixture::new();
    let path = fixture.0.join("test.star");
    fs::write(&path, "app = App(name='sample', version='1')\n").unwrap();
    let context = support::context(
        &fixture.0,
        BTreeMap::from([("VED_MAX_TICKS".into(), "1".into())]),
    );
    let error = ved_core::VedCompiler::with_context(context)
        .load_spec(&path)
        .unwrap_err();
    assert_eq!(
        error.resource_limit_error().unwrap().kind(),
        ResourceLimitKind::Ticks
    );
    assert!(!fixture.0.join(".ved").exists());
}
