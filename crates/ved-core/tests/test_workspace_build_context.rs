#![cfg(feature = "host")]
#![cfg(unix)]
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use ved_core::{
    save_workspace_state, ConfigLocator, ConfigSnapshot, DetectedModule, ModuleKind, SilentSink,
    WorkspaceContext, WorkspaceState,
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parallel_builds_use_selected_snapshot_toolchains_state_and_child_paths() {
    let root = std::env::temp_dir().join(format!("ved-build-context-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let _guard = Fixture(root.clone());
    let before_cwd = std::env::current_dir().unwrap();
    let before_env: BTreeMap<_, _> = std::env::vars_os().collect();
    let mut contexts = Vec::new();
    for name in ["first", "second"] {
        let project = root.join(name);
        let config = project.join("chosen config");
        let work = project.join("chosen work");
        let out = project.join("chosen output");
        let module = project.join("service");
        for dir in [&config, &work, &module] {
            fs::create_dir_all(dir).unwrap();
        }
        let fake = project.join("fake npm");
        fs::write(&fake, "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$VED_CONFIG_DIR\" \"$VED_WORK_DIR\" \"$VED_OUTPUT_DIR\" \"$SNAPSHOT_VALUE\" \"$@\" > \"$BUILD_LOG\"\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let log = project.join("build.log");
        fs::write(
            config.join(".env"),
            format!(
                "NPM_BIN={}\nBUILD_LOG={}\nSNAPSHOT_VALUE={name}\n",
                fake.display(),
                log.display()
            ),
        )
        .unwrap();
        let state = WorkspaceState {
            updated_at: "test".into(),
            root: project.to_string_lossy().into_owned(),
            modules: vec![DetectedModule {
                name: "service".into(),
                path: "service".into(),
                kinds: vec![ModuleKind::Node],
                build_cmd: String::new(),
                has_dockerfile: false,
                default_port: 80,
            }],
        };
        save_workspace_state(&state, &work).unwrap();
        // State in the default work dir must not win.
        fs::create_dir_all(project.join(".ved")).unwrap();
        fs::write(project.join(".ved/state.json"), "invalid ignored state").unwrap();
        let generated = work.join(".generated.env");
        let locator = ConfigLocator::new(config.clone(), generated.clone());
        let snapshot = ConfigSnapshot::with_environment(&locator, BTreeMap::new());
        let context = WorkspaceContext::with_snapshot(
            project.clone(),
            config.clone(),
            work.clone(),
            Some(out.clone()),
            generated,
            snapshot,
        );
        // Files changing after context capture cannot select another executable.
        fs::write(
            config.join(".env"),
            "NPM_BIN=definitely-not-a-tool\nSNAPSHOT_VALUE=changed\n",
        )
        .unwrap();
        contexts.push((context, log, vec![module, config, work, out], name));
    }
    std::thread::scope(|scope| {
        for (ctx, _, _, _) in &contexts {
            scope.spawn(move || {
                ved_core::build::build_all_concurrent_with_context_and_sink(
                    ctx,
                    ctx.root(),
                    None,
                    2,
                    &SilentSink,
                )
                .unwrap()
            });
        }
    });
    for (_, log, paths, name) in contexts {
        let mut paths = paths;
        paths[0] = paths[0].canonicalize().unwrap();
        let expected = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .chain([name.to_string(), "run".into(), "build".into()])
            .collect::<Vec<_>>();
        let actual = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
    assert_eq!(std::env::current_dir().unwrap(), before_cwd);
    assert_eq!(std::env::vars_os().collect::<BTreeMap<_, _>>(), before_env);
}
