//! Immutable workspace paths and configuration shared by compilation and execution.
use crate::config::{ConfigLocator, ConfigSnapshot};
use crate::model::AppSpec;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Failure to resolve a workspace path against an absolute working directory.
#[cfg(feature = "host")]
#[derive(Debug)]
pub struct WorkspaceResolutionError {
    path: PathBuf,
    source: std::io::Error,
}
#[cfg(feature = "host")]
impl WorkspaceResolutionError {
    /// The workspace path that required working-directory resolution.
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// The original working-directory error, or an invalid provider result.
    pub fn io_error(&self) -> &std::io::Error {
        &self.source
    }
}
#[cfg(feature = "host")]
impl std::fmt::Display for WorkspaceResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cannot resolve workspace path '{}' against the current directory: {}",
            self.path.display(),
            self.source
        )
    }
}
#[cfg(feature = "host")]
impl std::error::Error for WorkspaceResolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceContext {
    root: PathBuf,
    config_dir: PathBuf,
    work_dir: PathBuf,
    output_dir: Option<PathBuf>,
    generated_env: PathBuf,
    config: ConfigSnapshot,
}
impl WorkspaceContext {
    /// Construct explicit absolute paths without reading configuration or checking existence.
    pub fn from_explicit_paths(
        root: impl Into<PathBuf>,
        config_dir: impl Into<PathBuf>,
        config: ConfigSnapshot,
    ) -> crate::Result<Self> {
        let root = root.into();
        let config_dir = config_dir.into();
        let context = Self {
            work_dir: root.join(".ved"),
            generated_env: config_dir.join(".generated.env"),
            root,
            config_dir,
            output_dir: None,
            config,
        };
        context.validate_absolute_paths()?;
        Ok(context)
    }
    pub(crate) fn validate_absolute_paths(&self) -> crate::Result<()> {
        for path in [
            Some(self.root()),
            Some(self.config_dir()),
            Some(self.work_dir()),
            self.output_dir(),
            Some(self.generated_env_file()),
        ]
        .into_iter()
        .flatten()
        {
            if !path.is_absolute() {
                return Err(crate::VedError::config(format!(
                    "Explicit workspace path must be absolute: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    /// Resolve the root fallibly before reading its configuration.
    #[cfg(feature = "host")]
    pub fn try_new(root: impl Into<PathBuf>) -> Result<Self, WorkspaceResolutionError> {
        let root = resolve_paths([root.into()], std::env::current_dir)?.remove(0);
        Self::try_with_paths(
            root.clone(),
            root.join("config"),
            root.join(".ved"),
            None,
            root.join("config/.generated.env"),
        )
    }
    /// Resolve all paths using one working-directory snapshot before reading configuration.
    #[cfg(feature = "host")]
    pub fn try_with_paths(
        root: PathBuf,
        config_dir: PathBuf,
        work_dir: PathBuf,
        output_dir: Option<PathBuf>,
        generated_env: PathBuf,
    ) -> Result<Self, WorkspaceResolutionError> {
        let context = Self::try_with_snapshot(
            root,
            config_dir,
            work_dir,
            output_dir,
            generated_env,
            ConfigSnapshot::from_values(BTreeMap::new()),
        )?;
        let config = ConfigSnapshot::capture(&context.locator());
        Ok(Self { config, ..context })
    }
    /// Resolve paths fallibly without reading configuration or checking path existence.
    #[cfg(feature = "host")]
    pub fn try_with_snapshot(
        root: PathBuf,
        config_dir: PathBuf,
        work_dir: PathBuf,
        output_dir: Option<PathBuf>,
        generated_env: PathBuf,
        config: ConfigSnapshot,
    ) -> Result<Self, WorkspaceResolutionError> {
        Self::try_with_snapshot_using(
            root,
            config_dir,
            work_dir,
            output_dir,
            generated_env,
            config,
            std::env::current_dir,
        )
    }
    #[cfg(feature = "host")]
    #[allow(clippy::too_many_arguments)]
    fn try_with_snapshot_using(
        root: PathBuf,
        config_dir: PathBuf,
        work_dir: PathBuf,
        output_dir: Option<PathBuf>,
        generated_env: PathBuf,
        config: ConfigSnapshot,
        cwd: impl FnOnce() -> std::io::Result<PathBuf>,
    ) -> Result<Self, WorkspaceResolutionError> {
        let has_output = output_dir.is_some();
        let mut all_paths = vec![root, config_dir, work_dir, generated_env];
        all_paths.extend(output_dir);
        let mut resolved = resolve_paths(all_paths, cwd)?.into_iter();
        let (Some(root), Some(config_dir), Some(work_dir), Some(generated_env)) = (
            resolved.next(),
            resolved.next(),
            resolved.next(),
            resolved.next(),
        ) else {
            unreachable!("resolve_paths preserves input path count");
        };
        let output_dir = if has_output { resolved.next() } else { None };
        Ok(Self {
            root,
            config_dir,
            work_dir,
            generated_env,
            output_dir,
            config,
        })
    }
    /// Compatibility constructor; cwd failures retain the legacy relative-path fallback.
    /// Prefer [`Self::try_new`] when failures must be reported.
    #[cfg(feature = "host")]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = absolute(root.into());
        Self::with_paths(
            root.clone(),
            root.join("config"),
            root.join(".ved"),
            None,
            root.join("config/.generated.env"),
        )
    }
    /// Compatibility constructor with legacy cwd fallback; prefer [`Self::try_with_paths`].
    #[cfg(feature = "host")]
    pub fn with_paths(
        root: PathBuf,
        config_dir: PathBuf,
        work_dir: PathBuf,
        output_dir: Option<PathBuf>,
        generated_env: PathBuf,
    ) -> Self {
        let config_dir = absolute(config_dir);
        let generated_env = absolute(generated_env);
        let config = ConfigSnapshot::capture(&ConfigLocator::new(
            config_dir.clone(),
            generated_env.clone(),
        ));
        Self::with_snapshot(
            root,
            config_dir,
            work_dir,
            output_dir,
            generated_env,
            config,
        )
    }
    /// Compatibility constructor with legacy cwd fallback; prefer [`Self::try_with_snapshot`].
    #[cfg(feature = "host")]
    pub fn with_snapshot(
        root: PathBuf,
        config_dir: PathBuf,
        work_dir: PathBuf,
        output_dir: Option<PathBuf>,
        generated_env: PathBuf,
        config: ConfigSnapshot,
    ) -> Self {
        Self {
            root: absolute(root),
            config_dir: absolute(config_dir),
            work_dir: absolute(work_dir),
            output_dir: output_dir.map(absolute),
            generated_env: absolute(generated_env),
            config,
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }
    pub fn output_dir(&self) -> Option<&Path> {
        self.output_dir.as_deref()
    }
    pub fn generated_env_file(&self) -> &Path {
        &self.generated_env
    }
    pub fn config(&self) -> &ConfigSnapshot {
        &self.config
    }
    pub fn compose_file_path(&self) -> PathBuf {
        if let Some(ref out) = self.output_dir {
            out.join(crate::r::files::COMPOSE_FILE)
        } else {
            let comp_prop = self.config.get_prop(
                crate::r::keys::COMPOSE_PATH,
                crate::r::defaults::COMPOSE_PATH,
            );
            let p = Path::new(&comp_prop);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.root.join(p)
            }
        }
    }
    pub fn locator(&self) -> ConfigLocator {
        ConfigLocator::new(self.config_dir.clone(), self.generated_env.clone())
    }
    pub fn resolve(&self, path: impl AsRef<Path>) -> PathBuf {
        let p = path.as_ref();
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        }
    }
    pub fn child_environment(&self) -> BTreeMap<String, String> {
        let mut env = self.config.effective_environment();
        env.insert(
            "VED_CONFIG_DIR".into(),
            self.config_dir.to_string_lossy().into_owned(),
        );
        env.insert(
            "VED_WORK_DIR".into(),
            self.work_dir.to_string_lossy().into_owned(),
        );
        env.insert("VED_OUTPUT_DIR".into(), String::new());
        if let Some(out) = &self.output_dir {
            env.insert("VED_OUTPUT_DIR".into(), out.to_string_lossy().into_owned());
        }
        env
    }
    pub fn prepare_spec(&self, spec: &AppSpec) -> AppSpec {
        let mut spec = spec.clone();
        for (_name, service) in &mut spec.services {
            service.enabled = Some(service.enabled.unwrap_or_else(|| self.enabled(_name)));
            service.mounts = root_anchor_mounts(&service.mounts, &self.root);
            if let Some(ref b) = service.build {
                service.build = Some(crate::compose::root_anchor_build_context(b, &self.root));
            }
        }
        for (_name, job) in &mut spec.jobs {
            job.enabled = Some(job.enabled.unwrap_or_else(|| self.enabled(_name)));
            job.mounts = root_anchor_mounts(&job.mounts, &self.root);
        }
        for profile in spec.profiles.values_mut() {
            for (_name, service) in &mut profile.services {
                service.enabled = Some(service.enabled.unwrap_or_else(|| self.enabled(_name)));
                service.mounts = root_anchor_mounts(&service.mounts, &self.root);
                if let Some(ref b) = service.build {
                    service.build = Some(crate::compose::root_anchor_build_context(b, &self.root));
                }
            }
            for (_name, job) in &mut profile.jobs {
                job.enabled = Some(job.enabled.unwrap_or_else(|| self.enabled(_name)));
                job.mounts = root_anchor_mounts(&job.mounts, &self.root);
            }
        }
        let current_cfg_rel = self
            .config_dir
            .strip_prefix(&self.root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| self.config_dir.to_string_lossy().into_owned());
        let current_cfg_clean = if current_cfg_rel.is_empty() {
            ".".to_string()
        } else {
            current_cfg_rel
        };
        if self.config_dir != self.root.join("config")
            && self.config_dir != self.root.join(&spec.config_dir)
        {
            spec.config_dir = current_cfg_clean.clone();
        }

        if self.config_dir != self.root.join("config")
            && spec.env_files == ["config/.env", "config/.local.env"]
        {
            let base = if spec.config_dir != "." && !spec.config_dir.is_empty() {
                &spec.config_dir
            } else {
                &current_cfg_clean
            };
            if base != "." && !base.is_empty() {
                spec.env_files = vec![format!("{base}/.env"), format!("{base}/.local.env")];
            } else {
                spec.env_files = vec![
                    self.config_dir.join(".env").to_string_lossy().into_owned(),
                    self.config_dir
                        .join(".local.env")
                        .to_string_lossy()
                        .into_owned(),
                ];
            }
        } else if spec.config_dir != "." && !spec.config_dir.is_empty() {
            let prefix = format!("{}/", spec.config_dir);
            for f in &mut spec.env_files {
                if !f.starts_with('/') && !f.starts_with(&prefix) {
                    *f = format!("{prefix}{f}");
                }
            }
        }
        spec
    }
    pub fn enabled(&self, name: &str) -> bool {
        let key = format!("{}_ENABLED", name.replace('-', "_").to_uppercase());
        let value = self.config.get_prop(&key, "").to_lowercase();
        !matches!(value.as_str(), "0" | "false" | "no" | "off")
    }
}

/// Rewrites relative host-path mounts to be anchored at the workspace root.
/// Skips named volumes, PVC-style sources, and ConfigMap references.
pub fn root_anchor_mounts(mounts: &[String], root: &Path) -> Vec<String> {
    mounts
        .iter()
        .map(|mount| {
            let trimmed = mount.trim();
            if trimmed.starts_with("cm:") || trimmed.starts_with("configmap:") {
                return mount.clone();
            }

            if let Some(rest) = trimmed.strip_prefix("long:") {
                let parts: Vec<&str> = rest.split(':').collect();
                if parts.len() >= 3 {
                    let v_type = parts[0];
                    let src = parts[1];
                    let dst = parts[2];
                    let mode = parts.get(3).map(|s| format!(":{s}")).unwrap_or_default();
                    let is_host_path = v_type == "bind"
                        || src.starts_with("./")
                        || src.starts_with("../")
                        || src.starts_with('/')
                        || src.contains('/');
                    if is_host_path && !Path::new(src).is_absolute() {
                        let rel = src.trim_start_matches("./");
                        let anchored = if rel.is_empty() || rel == "." {
                            root.to_path_buf()
                        } else {
                            root.join(rel)
                        };
                        return format!("long:{}:{}:{}{}", v_type, anchored.display(), dst, mode);
                    }
                }
                return mount.clone();
            }

            let parts: Vec<&str> = trimmed.split(':').collect();
            if parts.len() < 2 || parts.len() > 3 {
                return mount.clone();
            }

            let src = parts[0];
            let dst = parts[1];
            let mode = parts.get(2).copied();

            if src.is_empty() || dst.is_empty() {
                return mount.clone();
            }

            // Host paths start with ./, ../, /, or contain a / separator.
            // Pure tokens without slashes are named volumes or PVCs.
            let is_host_path = src.starts_with("./")
                || src.starts_with("../")
                || src.starts_with('/')
                || src.contains('/');

            if !is_host_path || Path::new(src).is_absolute() {
                return mount.clone();
            }

            let rel = src.trim_start_matches("./");
            let anchored = if rel.is_empty() || rel == "." {
                root.to_path_buf()
            } else {
                root.join(rel)
            };

            let anchored_src = anchored.to_string_lossy();
            if let Some(m) = mode {
                format!("{anchored_src}:{dst}:{m}")
            } else {
                format!("{anchored_src}:{dst}")
            }
        })
        .collect()
}

#[cfg(feature = "host")]
fn resolve_paths(
    paths: impl IntoIterator<Item = PathBuf>,
    cwd: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<Vec<PathBuf>, WorkspaceResolutionError> {
    let paths: Vec<_> = paths.into_iter().collect();
    let Some(relative) = paths.iter().find(|path| !path.is_absolute()) else {
        return Ok(paths);
    };
    let directory = cwd().map_err(|source| WorkspaceResolutionError {
        path: relative.clone(),
        source,
    })?;
    if !directory.is_absolute() {
        return Err(WorkspaceResolutionError {
            path: relative.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "current directory provider returned a relative path",
            ),
        });
    }
    Ok(paths
        .into_iter()
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                directory.join(path)
            }
        })
        .collect())
}

#[cfg(feature = "host")]
fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;
    use crate::VedCompiler;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ved-context-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("config")).unwrap();
            Self(path)
        }
        fn context(&self, environment: BTreeMap<String, String>) -> WorkspaceContext {
            let locator = ConfigLocator::legacy(&self.0);
            let config = ConfigSnapshot::with_environment(&locator, environment);
            WorkspaceContext::with_snapshot(
                self.0.clone(),
                self.0.join("config"),
                self.0.join(".ved"),
                None,
                locator.generated_env_file(),
                config,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn explicit_root() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            r"C:\workspace"
        } else {
            "/workspace"
        })
    }
    #[test]
    fn fallible_resolution_retains_io_error_and_path_context() {
        use std::error::Error;
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied,
        ] {
            let error = resolve_paths([PathBuf::from("relative")], || {
                Err(std::io::Error::new(kind, "cwd unavailable"))
            })
            .unwrap_err();
            assert_eq!(error.path(), Path::new("relative"));
            assert_eq!(error.io_error().kind(), kind);
            assert_eq!(
                error
                    .source()
                    .unwrap()
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .kind(),
                kind
            );
            assert!(error.to_string().contains("relative"));
            assert!(error.to_string().contains("cwd unavailable"));
        }
    }
    #[test]
    fn fallible_resolution_rejects_relative_cwd() {
        let error = resolve_paths([PathBuf::from("root")], || {
            Ok(PathBuf::from("relative-cwd"))
        })
        .unwrap_err();
        assert_eq!(error.io_error().kind(), std::io::ErrorKind::InvalidInput);
    }
    #[test]
    fn fallible_snapshot_resolves_mixed_paths_with_one_cwd_capture() {
        let calls = std::cell::Cell::new(0);
        let cwd = explicit_root();
        let absolute_config = cwd.join("absolute-config");
        let config = ConfigSnapshot::from_values(BTreeMap::from([("KEY".into(), "value".into())]));
        let context = WorkspaceContext::try_with_snapshot_using(
            "root".into(),
            absolute_config.clone(),
            "work".into(),
            Some("output".into()),
            "generated".into(),
            config.clone(),
            || {
                calls.set(calls.get() + 1);
                Ok(cwd.clone())
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(context.root(), cwd.join("root"));
        assert_eq!(context.config_dir(), absolute_config);
        assert_eq!(context.work_dir(), cwd.join("work"));
        assert_eq!(context.output_dir(), Some(cwd.join("output").as_path()));
        assert_eq!(context.generated_env_file(), cwd.join("generated"));
        assert_eq!(context.config(), &config);
        context.validate_absolute_paths().unwrap();
    }
    #[test]
    fn fallible_absolute_snapshot_never_consults_cwd() {
        let root = explicit_root();
        let context = WorkspaceContext::try_with_snapshot_using(
            root.clone(),
            root.join("config"),
            root.join("work"),
            Some(root.join("output")),
            root.join("generated"),
            ConfigSnapshot::from_values(BTreeMap::new()),
            || panic!("absolute inputs must not consult cwd"),
        )
        .unwrap();
        context.validate_absolute_paths().unwrap();
        assert_eq!(context.root(), root);
    }
    #[test]
    fn snapshot_preserves_precedence_raw_interpolation_and_stability() {
        let fixture = Fixture::new();
        fs::write(
            fixture.0.join("config/.env"),
            "VALUE=base\nFALLBACK=base\nBASE=base\nDUP=first\nDUP=second\n",
        )
        .unwrap();
        fs::write(
            fixture.0.join("config/.local.env"),
            "VALUE=local\nLOCAL=local\n",
        )
        .unwrap();
        fs::write(
            fixture.0.join("config/.generated.env"),
            "VALUE=generated\nGENERATED=generated\n",
        )
        .unwrap();
        let context = fixture.context(BTreeMap::from([
            ("VALUE".into(), " raw value ".into()),
            ("FALLBACK".into(), "  ".into()),
            ("SECRET".into(), "sensitive-marker".into()),
        ]));
        assert_eq!(context.config().get_prop("VALUE", ""), "raw value");
        assert_eq!(
            context.config().interpolation_value("VALUE").as_deref(),
            Some(" raw value ")
        );
        assert_eq!(context.config().get_prop("FALLBACK", ""), "base");
        assert_eq!(context.child_environment()["FALLBACK"], "base");
        assert_eq!(context.config().get_prop("DUP", ""), "first");
        assert_eq!(context.config().get_prop("LOCAL", ""), "local");
        assert_eq!(context.config().get_prop("GENERATED", ""), "generated");
        fs::write(
            fixture.0.join("config/.env"),
            "VALUE=changed\nBASE=changed\n",
        )
        .unwrap();
        assert_eq!(context.config().get_prop("BASE", ""), "base");
        assert!(!format!("{context:?}").contains("sensitive-marker"));
        assert_eq!(context.child_environment()["VED_OUTPUT_DIR"], "");
    }
    #[test]
    fn concurrent_roots_compile_yaml_and_starlark_independently() {
        let before = std::env::current_dir().unwrap();
        let fixtures = [Fixture::new(), Fixture::new()];
        let contexts=fixtures.iter().enumerate().map(|(index,fixture)| {
            fs::write(fixture.0.join("config/.env"),format!("IMAGE=image-{index}\nWEB_ENABLED={}\nINGRESS_SUFFIX=domain-{index}\n",if index==0 {"true"} else {"false"})).unwrap();
            fs::write(fixture.0.join("app.yaml"),"app: dual\nversion: '1'\nservices:\n  web:\n    image: ${IMAGE}\n").unwrap();
            fs::write(fixture.0.join("Vedfile"),"app = App(name='dual', version='1')\napp.services.add(Service('web', image=env.secret('IMAGE')))\n").unwrap();
            fixture.context(BTreeMap::new())
        }).collect::<Vec<_>>();
        std::thread::scope(|scope| {
            let handles = contexts
                .iter()
                .enumerate()
                .map(|(index, context)| {
                    scope.spawn(move || {
                        let compiler = VedCompiler::with_context(context.clone());
                        for file in ["app.yaml", "Vedfile"] {
                            let spec = compiler.load_validated_spec(file).unwrap();
                            assert_eq!(
                                spec.as_spec().services["web"].image,
                                Some(format!("image-{index}"))
                            );
                            assert_eq!(spec.as_spec().services["web"].enabled, Some(index == 0));
                            let compose = compiler.compile_compose_validated(&spec).unwrap();
                            assert_eq!(compose.contains(&format!("image-{index}")), index == 0);
                        }
                    })
                })
                .collect::<Vec<_>>();
            for handle in handles {
                handle.join().unwrap();
            }
        });
        assert_eq!(std::env::current_dir().unwrap(), before);
    }
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    #[test]
    fn explicit_context_rejects_relative_paths_without_host_resolution() {
        let config = ConfigSnapshot::from_values(BTreeMap::new());
        assert!(WorkspaceContext::from_explicit_paths("relative", "config", config).is_err());
    }

    #[test]
    fn explicit_context_uses_supplied_values_and_paths() {
        let root = PathBuf::from(if cfg!(windows) {
            r"C:\explicit\workspace"
        } else {
            "/explicit/workspace"
        });
        let config =
            ConfigSnapshot::from_values(BTreeMap::from([("WEB_ENABLED".into(), " false ".into())]));
        let context =
            WorkspaceContext::from_explicit_paths(&root, root.join("config"), config).unwrap();
        assert_eq!(context.root(), root);
        assert!(!context.enabled("web"));
        assert_eq!(context.resolve("app.yaml"), root.join("app.yaml"));
    }
}
