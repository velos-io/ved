use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use ved_core::{AppSpec, ConfigLocator, ConfigSnapshot, Result, VedCompiler, WorkspaceContext};

pub fn context(root: &Path, environment: BTreeMap<String, String>) -> WorkspaceContext {
    let locator = ConfigLocator::legacy(root);
    // Snapshot only the committed base configuration, never developer overrides.
    let isolated = TempConfig::new();
    let isolated_locator = ConfigLocator::legacy(&isolated.0);
    std::fs::create_dir_all(&isolated_locator.config_dir).expect("create isolated config");
    if locator.env_file().is_file() {
        std::fs::copy(locator.env_file(), isolated_locator.env_file())
            .expect("copy base configuration");
    }
    let snapshot = ConfigSnapshot::with_environment(&isolated_locator, environment);
    WorkspaceContext::with_snapshot(
        root.to_path_buf(),
        root.join("config"),
        root.join(".ved"),
        None,
        locator.generated_env_file(),
        snapshot,
    )
}

#[allow(dead_code)]
pub fn load(path: impl AsRef<Path>, environment: BTreeMap<String, String>) -> Result<AppSpec> {
    let path = path.as_ref();
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap().join(path)
    };
    VedCompiler::with_context(context(path.parent().unwrap(), environment)).load_spec(&path)
}

#[allow(dead_code)]
pub fn compose(
    spec: &ved_core::AppSpec,
    environment: BTreeMap<String, String>,
) -> ved_core::Result<String> {
    let root = TempConfig::new();
    VedCompiler::with_context(context(&root.0, environment)).compile_compose(spec)
}

struct TempConfig(PathBuf);

impl TempConfig {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "ved-test-config-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create isolated config: {error}"),
            }
        }
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
