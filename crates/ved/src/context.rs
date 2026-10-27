use crate::cli::Slot;
use crate::paths::{ArtifactLayout, CliDirs, WorkspacePaths};
use std::path::{Path, PathBuf};
use ved_core::{ConfigLocator, ConfigSnapshot};

/// Encapsulates workspace paths and execution context for commands.
#[derive(Debug, Clone)]
pub(crate) struct WorkspaceContext {
    pub(crate) paths: WorkspacePaths,
    core: ved_core::WorkspaceContext,
    cancellation: ved_process::CancellationToken,
}

impl WorkspaceContext {
    pub(crate) fn new(root: PathBuf) -> crate::error::Result<Self> {
        let paths = WorkspacePaths::resolve(
            &CliDirs {
                dir: Some(root.clone()),
                ..Default::default()
            },
            &crate::resolve_cwd(std::env::current_dir)?,
        );
        Self::with_paths(paths)
    }

    pub(crate) fn with_paths(paths: WorkspacePaths) -> crate::error::Result<Self> {
        let locator = ConfigLocator::new(paths.config_dir.clone(), paths.generated_env_file());
        let mut snapshot = ConfigSnapshot::capture(&locator);
        if paths.config_dir_explicit {
            snapshot = snapshot.with_property(
                "VED_CONFIG_DIR",
                paths.config_dir.to_string_lossy().into_owned(),
            );
        }
        let core = ved_core::WorkspaceContext::try_with_snapshot(
            paths.root.clone(),
            paths.config_dir.clone(),
            paths.work_dir.clone(),
            paths.output_dir.clone(),
            paths.generated_env_file(),
            snapshot,
        )?;
        Ok(Self {
            paths,
            core,
            cancellation: ved_process::CancellationToken::new(),
        })
    }

    pub(crate) fn with_cancellation_token(mut self, token: ved_process::CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    pub(crate) fn with_spec(&self, spec: &ved_core::AppSpec) -> crate::error::Result<Self> {
        if self.paths.config_dir_explicit {
            return Ok(self.clone());
        }
        let target_dir = if spec.config_dir != "." && !spec.config_dir.is_empty() {
            self.root().join(&spec.config_dir)
        } else {
            return Ok(self.clone());
        };
        if target_dir == self.paths.config_dir {
            return Ok(self.clone());
        }
        let updated_paths = self.paths.with_config_dir(target_dir);
        let mut new_ctx = Self::with_paths(updated_paths)?;
        new_ctx.cancellation = self.cancellation.clone();
        Ok(new_ctx)
    }

    pub(crate) fn cancellation_token(&self) -> ved_process::CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn core(&self) -> &ved_core::WorkspaceContext {
        &self.core
    }
    pub(crate) fn config(&self) -> &ved_core::ConfigSnapshot {
        self.core.config()
    }
    pub(crate) fn namespace_resolver(&self, slot: &Slot) -> ved_core::NamespaceResolver {
        ved_core::NamespaceResolver::from_context(self.core(), slot.as_str())
    }

    #[allow(dead_code)]
    pub(crate) fn paths(&self) -> &WorkspacePaths {
        &self.paths
    }

    pub(crate) fn root(&self) -> &Path {
        &self.paths.root
    }

    pub(crate) fn config_dir(&self) -> &Path {
        &self.paths.config_dir
    }

    pub(crate) fn work_dir(&self) -> &Path {
        &self.paths.work_dir
    }

    #[allow(dead_code)]
    pub(crate) fn output_dir(&self) -> Option<&Path> {
        self.paths.output_dir.as_deref()
    }

    pub(crate) fn config_locator(&self) -> ConfigLocator {
        ConfigLocator::new(
            self.paths.config_dir.clone(),
            self.paths.generated_env_file(),
        )
    }

    pub(crate) fn artifact_layout(&self, compose_path_prop: &str) -> ArtifactLayout {
        ArtifactLayout::from_paths(&self.paths, compose_path_prop)
    }

    #[allow(dead_code)]
    pub(crate) fn resolve(&self, path: impl AsRef<Path>) -> PathBuf {
        let p = path.as_ref();
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root().join(p)
        }
    }

    pub(crate) fn resolve_slot(&self, slot: Option<Slot>) -> Slot {
        slot.unwrap_or_else(|| Slot(self.config().get_prop("K8S_APP_SLOT", "1")))
    }

    pub(crate) fn effective_dir(&self, dir: &Path) -> PathBuf {
        if dir == Path::new(".") {
            self.root().to_path_buf()
        } else if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            self.root().join(dir)
        }
    }

    pub(crate) fn spec_path(&self, file: Option<&Path>) -> PathBuf {
        crate::spec::resolve_spec_file_in(self.root(), file.map(Path::to_path_buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effective_dir_resolution() {
        let ctx = WorkspaceContext::new(PathBuf::from("/workspace/project")).unwrap();

        assert_eq!(
            ctx.effective_dir(Path::new(".")),
            PathBuf::from("/workspace/project")
        );
        assert_eq!(
            ctx.effective_dir(Path::new("dist")),
            PathBuf::from("/workspace/project/dist")
        );
        assert_eq!(
            ctx.effective_dir(Path::new("sub/dir")),
            PathBuf::from("/workspace/project/sub/dir")
        );
        assert_eq!(
            ctx.effective_dir(Path::new("/tmp/output")),
            PathBuf::from("/tmp/output")
        );
    }

    #[test]
    fn test_resolve_slot_delegation() {
        let tmp = std::env::temp_dir().join(format!("ctx_slot_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);

        let ctx = WorkspaceContext::new(tmp.clone()).unwrap();
        let slot = ctx.resolve_slot(Some(Slot("custom-slot".to_string())));
        assert_eq!(slot.as_str(), "custom-slot");

        let default_slot = ctx.resolve_slot(None);
        assert_eq!(default_slot.as_str(), "1");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_spec_path_resolution() {
        let tmp = std::env::temp_dir().join(format!("ctx_spec_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);

        let ctx = WorkspaceContext::new(tmp.clone()).unwrap();

        let vedfile = tmp.join("Vedfile");
        std::fs::write(&vedfile, "app = App(name = 'test')\n").unwrap();
        assert_eq!(ctx.spec_path(None), vedfile);

        let custom_file = Path::new("custom.yaml");
        assert_eq!(ctx.spec_path(Some(custom_file)), tmp.join("custom.yaml"));

        let abs_file = Path::new("/var/ved.yaml");
        assert_eq!(
            ctx.spec_path(Some(abs_file)),
            PathBuf::from("/var/ved.yaml")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
