#![cfg(feature = "host")]
mod support;

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use ved_core::{SpecLoadPhase, VedError};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ved-load-causes-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn error(&self, name: &str, contents: &str) -> VedError {
        let path = self.0.join(name);
        fs::write(&path, contents).unwrap();
        let context = support::context(&self.0, BTreeMap::new());
        ved_core::VedCompiler::with_context(context)
            .load_spec(&path)
            .unwrap_err()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_file_retains_concrete_io_source_and_read_context() {
    let fixture = Fixture::new();
    let path = fixture.0.join("missing.yaml");
    let context = support::context(&fixture.0, BTreeMap::new());
    let error = ved_core::load_any_spec_with_context(&path, &context).unwrap_err();
    let load = error.spec_load_error().unwrap();
    assert_eq!(load.path(), path);
    assert_eq!(load.phase(), SpecLoadPhase::Read);
    assert_eq!(
        load.source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(error.is_validation_error());
}

#[test]
fn yaml_parse_retains_concrete_source_but_does_not_display_secret_value() {
    let fixture = Fixture::new();
    let error = fixture.error(
        "invalid.yaml",
        "app: example\nversion: 1\nservices:\n  web:\n    enabled: private-secret-value\n",
    );
    let load = error.spec_load_error().unwrap();
    assert_eq!(load.path(), fixture.0.join("invalid.yaml"));
    assert_eq!(load.phase(), SpecLoadPhase::Parse);
    let original = load
        .source()
        .unwrap()
        .downcast_ref::<serde_yaml::Error>()
        .unwrap();
    assert!(original.location().is_some());
    assert!(original.to_string().contains("private-secret-value"));
    assert!(!error.to_string().contains("private-secret-value"));
    assert!(!format!("{load:?}").contains("private-secret-value"));
}

#[test]
fn semantic_validation_keeps_file_and_typed_field_context() {
    let fixture = Fixture::new();
    let error = fixture.error(
        "invalid.yaml",
        "app: example\nversion: 1\nservices:\n  web:\n    ports: [\"0:8080\"]\n",
    );
    assert_eq!(
        error.spec_load_error().unwrap().phase(),
        SpecLoadPhase::Validate
    );
    let validation = error
        .validation_error()
        .expect("nested typed validation source");
    assert_eq!(validation.field(), "ports[0]");
    assert!(error.is_validation_error());
}

#[test]
fn shorthand_failure_retains_original_syntax_error() {
    let fixture = Fixture::new();
    let error = fixture.error("app.ved", "Service web nginx not-a-port\n");
    let load = error.spec_load_error().unwrap();
    assert_eq!(load.phase(), SpecLoadPhase::Parse);
    assert!(load.source().unwrap().downcast_ref::<VedError>().is_some());
    assert!(!error.to_string().contains("not-a-port"));
}

#[test]
fn starlark_parse_and_evaluation_retain_supported_upstream_diagnostics() {
    let fixture = Fixture::new();
    for (contents, phase, secret) in [
        (
            "private_parse_secret = @\n",
            SpecLoadPhase::Parse,
            "private_parse_secret",
        ),
        (
            "app = App(name=\"sample\", version=\"1\")\nfail(\"private_evaluation_secret\")\n",
            SpecLoadPhase::Evaluate,
            "private_evaluation_secret",
        ),
    ] {
        let error = fixture.error("bad.star", contents);
        let load = error.spec_load_error().unwrap();
        assert_eq!(load.path(), fixture.0.join("bad.star"));
        assert_eq!(load.phase(), phase);
        assert!(
            load.source().unwrap().to_string().contains(secret),
            "{}",
            load.source().unwrap()
        );
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{load:?}").contains(secret));
    }
}

#[test]
fn starlark_wire_conversion_retains_json_source_without_raw_value() {
    let fixture = Fixture::new();
    let error = fixture.error("bad.star", "app = App(name=\"sample\", version=\"1\")\napp.services.add(Service(name=\"web\", image=\"nginx\", env=\"private_conversion_secret\"))\n");
    let load = error.spec_load_error().unwrap();
    assert_eq!(load.phase(), SpecLoadPhase::Convert);
    assert!(load
        .source()
        .unwrap()
        .downcast_ref::<serde_json::Error>()
        .is_some());
    assert!(load
        .source()
        .unwrap()
        .to_string()
        .contains("private_conversion_secret"));
    assert!(!error.to_string().contains("private_conversion_secret"));
}
