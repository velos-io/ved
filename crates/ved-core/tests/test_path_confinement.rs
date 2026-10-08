#![cfg(feature = "host")]
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use ved_core::{AppSpec, JobSpec, ServiceSpec};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ved-path-confinement-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn spec(name: &str, group: &str) -> AppSpec {
    let mut spec = AppSpec::default();
    spec.services.insert(
        name.into(),
        ServiceSpec {
            image: Some("nginx:alpine".into()),
            group: group.into(),
            ..Default::default()
        },
    );
    spec
}

#[test]
fn direct_k8s_rejects_all_identifiers_before_creating_output() {
    let fixture = Fixture::new();
    for bad in [
        "../escape",
        "a/b",
        "a\\b",
        "/absolute",
        "C:drive",
        ".",
        "..",
        "",
        "a\nname",
    ] {
        let output = fixture.path("out");
        assert!(ved_core::generate_k8s(&spec(bad, "agent"), &output).is_err());
        assert!(!output.exists(), "invalid workload created output: {bad:?}");
        assert!(ved_core::generate_k8s(&spec("web", bad), &output).is_err() || bad.is_empty());
        if bad.is_empty() {
            fs::remove_dir_all(output).unwrap(); // Empty group intentionally means agent.
        } else {
            assert!(!output.exists(), "invalid group created output: {bad:?}");
        }
    }
    let mut invalid = spec("valid", "agent");
    invalid.services.insert(
        "../disabled".into(),
        ServiceSpec {
            enabled: Some(false),
            ..Default::default()
        },
    );
    assert!(ved_core::generate_k8s(&invalid, &fixture.path("out")).is_err());
    invalid.services.shift_remove("../disabled");
    invalid.jobs.insert(
        "../job".into(),
        serde_json::from_value::<JobSpec>(serde_json::json!({"image":"alpine"})).unwrap(),
    );
    assert!(ved_core::generate_k8s(&invalid, &fixture.path("out")).is_err());
    assert!(!fixture.path("out").exists());
}

#[test]
fn yaml_starlark_and_shorthand_reject_traversal_before_stack_application() {
    let fixture = Fixture::new();
    let yaml = fixture.path("spec.yaml");
    fs::write(&yaml, "app: audit\nversion: '1'\nservices:\n  web:\n    image: nginx\n    group: ../../../escaped\n").unwrap();
    assert!(ved_core::load_spec(&yaml).is_err());
    let vedfile = fixture.path("Vedfile");
    fs::write(&vedfile, "app = App(name='audit', version='1', network='audit')\nStack.add('../escape')\napp.services.add(Service(name='web', image='nginx'))\n").unwrap();
    let error = ved_core::load_starlark_spec(&vedfile).unwrap_err();
    assert_eq!(
        error.spec_load_error().unwrap().phase(),
        ved_core::SpecLoadPhase::Validate
    );
    assert_eq!(
        error.validation_error().unwrap().reason(),
        ved_core::ValidationReason::InvalidIdentifier
    );
    assert!(!error.to_string().contains("../escape"));
    fs::write(&vedfile, "app = App(name='audit', version='1', network='audit')\napp.services.add(Service(name='../escape', image='nginx'))\n").unwrap();
    assert!(ved_core::load_starlark_spec(&vedfile).is_err());
    let shorthand = fixture.path("spec.ved");
    fs::write(
        &shorthand,
        "App audit 1\nStack ../escape\nService web nginx 80\n",
    )
    .unwrap();
    assert!(ved_core::load_shorthand_spec(&shorthand).is_err());
    assert!(!fixture.path(".ved").exists());
    assert!(!fixture.path("escape").exists());
}

#[test]
fn profile_identifiers_are_validated_even_when_not_selected() {
    let fixture = Fixture::new();
    let yaml = fixture.path("spec.yaml");
    fs::write(&yaml, "app: audit\nversion: '1'\nprofiles:\n  debug:\n    services:\n      '../escape':\n        image: nginx\n").unwrap();
    assert!(ved_core::load_spec(&yaml).is_err());
}

#[test]
fn valid_absolute_output_and_legacy_group_names_generate_safely() {
    let fixture = Fixture::new();
    for group in ["", "default", "  agent  "] {
        let output = fixture.path("absolute-output");
        assert!(output.is_absolute());
        ved_core::generate_k8s(&spec("API_v1.2", group), &output).unwrap();
        let normalized = ved_core::resolve_group(group, "API_v1.2");
        assert!(output
            .join(format!("base/{normalized}/API_v1.2.yaml"))
            .exists());
    }
    let output = fixture.path("invalid-namespace-output");
    assert!(ved_core::generate_k8s(&spec("API_v1.2", "Team_1.v2"), &output).is_err());
    assert!(!output.exists());
}

#[cfg(unix)]
#[test]
fn k8s_rejects_directory_file_and_dangling_symlinks_before_any_write() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = fixture.path("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("sentinel"), "untouched").unwrap();
    for (i, link) in [
        "base",
        "base/agent",
        "base/agent/web.yaml",
        "base/infra/secrets.yaml",
        "overlays/local/ns-1",
        "unused/dangling",
    ]
    .iter()
    .enumerate()
    {
        let output = fixture.path(&format!("out-{i}"));
        let path = output.join(link);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = if link.ends_with(".yaml") {
            outside.join("sentinel")
        } else if link.contains("dangling") {
            outside.join("absent")
        } else {
            outside.clone()
        };
        symlink(target, &path).unwrap();
        assert!(
            ved_core::generate_k8s(&spec("web", "agent"), &output).is_err(),
            "{link}"
        );
        assert!(!output.join("base/kustomization.yaml").exists());
        assert_eq!(
            fs::read_to_string(outside.join("sentinel")).unwrap(),
            "untouched"
        );
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    }
    let output_link = fixture.path("linked-root");
    symlink(&outside, &output_link).unwrap();
    assert!(ved_core::generate_k8s(&spec("web", "agent"), &output_link).is_err());
}

#[cfg(unix)]
#[test]
fn k8s_replaces_hardlinked_outputs_without_mutating_outside_aliases() {
    let fixture = Fixture::new();
    let sentinel = fixture.path("sentinel");
    fs::write(&sentinel, "untouched").unwrap();
    let output = fixture.path("out");
    fs::create_dir_all(output.join("base/agent")).unwrap();
    fs::hard_link(&sentinel, output.join("base/agent/web.yaml")).unwrap();
    ved_core::generate_k8s(&spec("web", "agent"), &output).unwrap();
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "untouched");
    assert!(fs::read_to_string(output.join("base/agent/web.yaml"))
        .unwrap()
        .contains("Deployment"));
}
