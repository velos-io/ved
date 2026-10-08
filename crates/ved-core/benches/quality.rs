use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ved_core::{ManifestDocBuilder, YamlNode};

fn quality(c: &mut Criterion) {
    let text = format!(
        "App sample 1.0.0\n{}",
        (0..32)
            .map(|i| format!("Service web{i} nginx {}\n", 8000 + i))
            .collect::<String>()
    );
    c.bench_function("parse_shorthand_32_services", |b| {
        b.iter(|| ved_core::parse_shorthand(black_box(&text)).unwrap())
    });
    let spec = ved_core::parse_shorthand(&text).unwrap();
    c.bench_function("emit_compose_32_services", |b| {
        b.iter(|| ved_core::generate_compose(black_box(&spec)).unwrap())
    });
    c.bench_function("emit_typed_manifest_32_fields", |b| {
        b.iter(|| {
            ManifestDocBuilder::new("v1", "ConfigMap", "sample")
                .with_field(
                    "data",
                    YamlNode::Mapping(
                        (0..32)
                            .map(|i| {
                                (
                                    format!("KEY_{i}"),
                                    YamlNode::scalar("quotes: \" # 世界\nline"),
                                )
                            })
                            .collect(),
                    ),
                )
                .render()
                .unwrap()
        })
    });
    scheduler(c);
}

#[cfg(unix)]
fn scheduler(c: &mut Criterion) {
    use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, time::Duration};
    use ved_core::{
        ConfigLocator, ConfigSnapshot, DetectedModule, ModuleKind, WorkspaceContext, WorkspaceState,
    };
    let root = std::env::temp_dir().join(format!("ved-quality-bench-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let fake = root.join("fake-build");
    fs::write(
        &fake,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> ./invocations.log\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let modules = (0..8)
        .map(|i| {
            let name = format!("module{i}");
            fs::create_dir_all(root.join(&name)).unwrap();
            DetectedModule {
                name: name.clone(),
                path: name,
                kinds: vec![ModuleKind::Go],
                build_cmd: String::new(),
                has_dockerfile: false,
                default_port: 8080,
            }
        })
        .collect();
    ved_core::save_workspace_state(
        &WorkspaceState {
            updated_at: String::new(),
            root: root.display().to_string(),
            modules,
        },
        &root.join(".ved"),
    )
    .unwrap();
    let locator = ConfigLocator::new(root.join("config"), root.join("config/generated.env"));
    let config = ConfigSnapshot::with_environment(
        &locator,
        BTreeMap::from([("GO_BIN".into(), fake.display().to_string())]),
    );
    let context = WorkspaceContext::with_snapshot(
        root.clone(),
        root.join("config"),
        root.join(".ved"),
        None,
        root.join("config/generated.env"),
        config,
    );
    c.bench_function("schedule_8_fake_builds_4_workers", |b| {
        b.iter(|| {
            ved_core::build::build_all_concurrent_with_context_and_sink_and_control(
                &context,
                &root,
                None,
                4,
                &ved_core::SilentSink,
                Some(Duration::from_secs(5)),
                ved_process::CancellationToken::new(),
            )
            .unwrap();
        })
    });
    for i in 0..8 {
        assert!(
            fs::read_to_string(root.join(format!("module{i}/invocations.log")))
                .unwrap()
                .lines()
                .all(|line| line == "build ./...")
        );
    }
}
#[cfg(not(unix))]
fn scheduler(_: &mut Criterion) {}
criterion_group! { name = benches; config = Criterion::default().sample_size(10).warm_up_time(std::time::Duration::from_secs(1)).measurement_time(std::time::Duration::from_secs(2)); targets = quality }
criterion_main!(benches);
