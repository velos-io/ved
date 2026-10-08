#[cfg(feature = "host")]
use std::path::PathBuf;
use std::{collections::BTreeMap, fs, path::Path};
use ved_core::{
    AppSpec, CompilationTarget, ConfigSnapshot, PureCompiler, ValidatedSpec, WorkspaceContext,
};

fn context(root: &Path) -> WorkspaceContext {
    WorkspaceContext::from_explicit_paths(
        root,
        root.join("config"),
        ConfigSnapshot::from_values(BTreeMap::from([
            ("WEB_ENABLED".into(), "false".into()),
            ("PROFILE_WEB_ENABLED".into(), "false".into()),
            ("K8S_APP_SLOT".into(), "7".into()),
            ("K8S_INFRA_NAMESPACE".into(), "custom-infra".into()),
            ("INGRESS_SUFFIX".into(), "example.test".into()),
            ("POSTGRES_PASSWORD".into(), "private-secret-marker".into()),
        ])),
    )
    .unwrap()
}
fn spec() -> ValidatedSpec {
    let raw: AppSpec = serde_yaml::from_str(
        r#"
app: parity
version: '1'
services:
  web:
    image: disabled-marker
  api:
    image: api:v1
    ports: ['8080:8080']
    ingress: api
    env:
      TOKEN: private-secret-marker
  database:
    image: postgres:16
    group: infra
jobs:
  migrate:
    image: migrate:v1
    command: ['run', 'migration']
profiles:
  debug:
    when: DEBUG_ENABLED
    services:
      profile-web:
        image: disabled-profile-marker
"#,
    )
    .unwrap();
    ValidatedSpec::try_from(raw).unwrap()
}
#[cfg(feature = "host")]
fn read_tree(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            read_tree(root, &path, files);
        } else {
            files.insert(
                path.strip_prefix(root).unwrap().into(),
                fs::read(path).unwrap(),
            );
        }
    }
}
#[test]
fn hermetic_compilation_is_deterministic_and_matches_legacy_outputs() {
    let root = std::env::temp_dir().join(format!("ved-pure-parity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let context = context(&root);
    let compiler = PureCompiler::new(context.clone()).unwrap();
    let spec = spec();
    let bundle = compiler.compile(&spec, CompilationTarget::All).unwrap();
    assert_eq!(
        bundle,
        compiler.compile(&spec, CompilationTarget::All).unwrap()
    );
    assert!(!root.exists(), "pure compilation created the workspace");
    assert!(!format!("{context:?} {bundle:?}").contains("private-secret-marker"));
    for (path, bytes) in bundle.files() {
        assert!(path.is_relative());
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("disabled-marker"));
        assert!(!text.contains("disabled-profile-marker"));
    }
    assert!(bundle
        .files()
        .contains_key(Path::new("k8s/overlays/local/ns-7/kustomization.yaml")));
    assert!(bundle.files().contains_key(Path::new(
        "k8s/overlays/local/custom-infra/kustomization.yaml"
    )));
    #[cfg(feature = "host")]
    {
        let legacy = ved_core::VedCompiler::with_context(context.clone());
        assert_eq!(
            bundle.files()[Path::new("docker-compose.yml")],
            legacy
                .compile_compose_validated(&spec)
                .unwrap()
                .into_bytes()
        );
        assert_eq!(
            bundle.files()[Path::new("helm/values.yaml")],
            legacy.compile_helm_validated(&spec).unwrap().into_bytes()
        );
        // Freeze activation before constructing either routing plan.
        let prepared = spec.with_context(&context);
        let plan = ved_core::KubernetesRoutingPlan::from_context(prepared.as_spec(), &context, "7")
            .unwrap();
        legacy
            .compile_k8s_validated_with_plan(&prepared, root.join("persisted"), &plan)
            .unwrap();
        let mut persisted = BTreeMap::new();
        read_tree(
            &root.join("persisted"),
            &root.join("persisted"),
            &mut persisted,
        );
        let expected = bundle
            .files()
            .iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix("k8s")
                    .ok()
                    .map(|path| (path.to_path_buf(), bytes.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(persisted, expected);
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn explicit_paths_reject_relative_inputs_and_snapshot_preserves_raw_values() {
    let snapshot = ConfigSnapshot::from_values(BTreeMap::from([("RAW".into(), " raw ".into())]));
    assert_eq!(
        snapshot.interpolation_value("RAW").as_deref(),
        Some(" raw ")
    );
    assert_eq!(snapshot.get_prop("RAW", ""), "raw");
    let root = std::env::temp_dir();
    assert!(WorkspaceContext::from_explicit_paths(
        "relative",
        root.join("config"),
        snapshot.clone()
    )
    .is_err());
    assert!(WorkspaceContext::from_explicit_paths(root, "relative", snapshot).is_err());
}
#[test]
fn malformed_spec_cannot_enter_the_pure_facade() {
    let raw: AppSpec = serde_yaml::from_str("app: invalid\nversion: '1'\nservices:\n  web:\n    image: web:v1\n    ports: ['not-a-port']\n").unwrap();
    assert!(ValidatedSpec::try_from(raw).is_err());
}

#[test]
fn explicit_configuration_ignores_ambient_environment_and_files() {
    const CHILD: &str = "VED_PURE_POISON_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "explicit_configuration_ignores_ambient_environment_and_files",
            ])
            .env(CHILD, "1")
            .env("API_ENABLED", "false")
            .env("K8S_APP_SLOT", "99")
            .env("INGRESS_SUFFIX", "poison.test")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let root = std::env::temp_dir().join(format!("ved-pure-poison-{}", std::process::id()));
    let context = context(&root);
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/.env"),
        "API_ENABLED=false\nK8S_APP_SLOT=99\nINGRESS_SUFFIX=file-poison.test\n",
    )
    .unwrap();
    let bundle = PureCompiler::new(context)
        .unwrap()
        .compile(&spec(), CompilationTarget::All)
        .unwrap();
    let compose = String::from_utf8_lossy(&bundle.files()[Path::new("docker-compose.yml")]);
    assert!(
        compose.contains("api:v1"),
        "unspecified activation must use explicit snapshot defaults"
    );
    assert!(bundle
        .files()
        .contains_key(Path::new("k8s/base/agent/api.yaml")));
    assert!(bundle
        .files()
        .contains_key(Path::new("k8s/overlays/local/ns-7/kustomization.yaml")));
    for bytes in bundle.files().values() {
        assert!(!String::from_utf8_lossy(bytes).contains("poison.test"));
    }
    fs::remove_dir_all(root).unwrap();
}
