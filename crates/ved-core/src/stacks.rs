use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, VedError};
use crate::model::AppSpec;
use crate::resources::{LoadOptions, LoadSession};
pub use crate::vcs::{VcsDriver, VcsError, VcsOutput};

/// Locate the stacks catalog directory using hermetic workspace context.
pub fn resolve_stacks_dir() -> PathBuf {
    resolve_stacks_dir_with_context(&crate::WorkspaceContext::new("."))
}

/// A regular file copied from an activated stack catalog into the workspace.
/// Loading a spec returns these assets without writing to the workspace; a
/// publisher can include them in the same transaction as generated artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackAsset {
    pub destination: PathBuf,
    pub bytes: Vec<u8>,
}

/// Exact source bytes observed while loading a dependency. Callers should hash
/// bytes before writing metadata; paths and file contents stay in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedDependency {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// Result of evaluating stack overlays without publishing their config files.
#[derive(Debug, Clone)]
pub struct StackApplication {
    pub spec: AppSpec,
    pub dependencies: Vec<LoadedDependency>,
    pub assets: Vec<StackAsset>,
}

pub(crate) fn run_git_op<R: VcsDriver + ?Sized>(
    runner: &R,
    op: &'static str,
    target: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<()> {
    let safe_target = redact_url_credentials(target);
    match runner.execute(args, cwd) {
        Ok(output) => {
            let success = output.exit_code == Some(0);
            if success {
                Ok(())
            } else {
                let details = match output.exit_code {
                    Some(code) => format!("git command exited with status {code}"),
                    None => "git process terminated by signal".to_string(),
                };
                Err(VedError::stacks_sync(op, safe_target, details))
            }
        }
        Err(VcsError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => Err(
            VedError::stacks_sync(op, safe_target, "git executable not found in PATH"),
        ),
        Err(VcsError::Io(err)) => Err(VedError::stacks_sync(op, safe_target, err.to_string())),
        Err(VcsError::Process(source)) => Err(VedError::ProcessOperation {
            operation: op.into(),
            resource: safe_target,
            source,
        }),
        Err(VcsError::Custom(msg)) => Err(VedError::stacks_sync(op, safe_target, msg)),
    }
}

fn redact_url_credentials(value: &str) -> String {
    let Some(scheme_end) = value.find("://") else {
        return value.to_string();
    };
    let sanitized = value
        .find(['?', '#'])
        .map(|index| &value[..index])
        .unwrap_or(value);
    let authority_start = scheme_end + 3;
    let authority_end = sanitized[authority_start..]
        .find('/')
        .map(|offset| authority_start + offset)
        .unwrap_or(sanitized.len());
    let authority = &sanitized[authority_start..authority_end];
    let Some(at) = authority.rfind('@') else {
        return sanitized.to_string();
    };
    format!(
        "{}[redacted]@{}{}",
        &sanitized[..authority_start],
        &authority[at + 1..],
        &sanitized[authority_end..]
    )
}

/// Options and fluent builder for synchronizing the stacks catalog.
#[derive(Default)]
pub struct StackSyncOptions<'a> {
    pub context: Option<&'a crate::WorkspaceContext>,
    pub driver: Option<&'a dyn VcsDriver>,
    pub cancellation: Option<ved_process::CancellationToken>,
    pub stacks_dir: Option<PathBuf>,
    pub repo_url: Option<String>,
}

impl<'a> StackSyncOptions<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn context(mut self, context: &'a crate::WorkspaceContext) -> Self {
        self.context = Some(context);
        self
    }

    pub fn driver(mut self, driver: &'a dyn VcsDriver) -> Self {
        self.driver = Some(driver);
        self
    }

    pub fn cancellation(mut self, token: ved_process::CancellationToken) -> Self {
        self.cancellation = Some(token);
        self
    }

    pub fn stacks_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.stacks_dir = Some(dir.into());
        self
    }

    pub fn repo_url(mut self, url: impl Into<String>) -> Self {
        self.repo_url = Some(url.into());
        self
    }

    pub fn sync(self) -> Result<()> {
        sync_stacks_catalog_with_options(self)
    }
}

pub fn sync_stacks_catalog_with_options(options: StackSyncOptions<'_>) -> Result<()> {
    let default_ctx;
    let context = match options.context {
        Some(ctx) => ctx,
        None => {
            default_ctx = crate::WorkspaceContext::new(".");
            &default_ctx
        }
    };
    let directory = match options.stacks_dir {
        Some(dir) => dir,
        None => {
            let dir = resolve_stacks_dir_with_context(context);
            if context
                .config()
                .get_prop_opt(crate::r::keys::VED_STACKS_PATH)
                .is_none()
                && context.config().interpolation_value("HOME").is_none()
                && !dir.exists()
            {
                return Err(VedError::config("HOME environment variable is not set"));
            }
            dir
        }
    };
    let repository = match options.repo_url {
        Some(url) => url,
        None => context.config().get_prop(
            crate::r::keys::VED_STACKS_REPO,
            "https://github.com/velos-io/stacks.git",
        ),
    };
    let default_runner;
    let driver: &dyn VcsDriver = match options.driver {
        Some(d) => d,
        None => {
            let token = options.cancellation.unwrap_or_default();
            default_runner = crate::vcs::HostGitRunner::new(token);
            &default_runner
        }
    };
    sync_stacks_catalog_internal(driver, &directory, &repository)
}

/// Sync the stacks catalog from remote if needed.
pub fn sync_stacks_catalog() -> Result<()> {
    StackSyncOptions::new().sync()
}

/// Sync the catalog using the host application's cancellation policy.
pub fn sync_stacks_catalog_with_cancellation(token: ved_process::CancellationToken) -> Result<()> {
    StackSyncOptions::new().cancellation(token).sync()
}

/// Entry point supporting injected runners for tests and host integration.
#[allow(dead_code)]
pub fn sync_stacks_catalog_with_runner<R: VcsDriver>(runner: &R) -> Result<()> {
    StackSyncOptions::new().driver(runner).sync()
}

pub fn sync_stacks_catalog_with_driver<D: VcsDriver>(
    driver: &D,
    stacks_dir: &Path,
    repo_url: &str,
) -> Result<()> {
    StackSyncOptions::new()
        .driver(driver)
        .stacks_dir(stacks_dir)
        .repo_url(repo_url)
        .sync()
}

pub(crate) fn sync_stacks_catalog_internal<R: VcsDriver + ?Sized>(
    runner: &R,
    stacks_dir: &Path,
    repo_url: &str,
) -> Result<()> {
    if stacks_dir.join(".git").exists() {
        let stacks_dir_str = stacks_dir.to_string_lossy();
        return run_git_op(
            runner,
            "pull",
            stacks_dir_str.as_ref(),
            &["-C", stacks_dir_str.as_ref(), "pull", "--quiet"],
            None,
        );
    }

    if stacks_dir.exists() {
        return Ok(());
    }

    if let Some(parent) = stacks_dir.parent() {
        fs::create_dir_all(parent)?;
    }
    let target_str = stacks_dir.to_string_lossy();
    run_git_op(
        runner,
        "clone",
        repo_url,
        &["clone", repo_url, target_str.as_ref()],
        None,
    )
}

/// Apply named catalog stacks after validating every input and filesystem destination.
pub fn apply_stacks(stack_names: &[String], spec: &mut AppSpec, project_dir: &Path) -> Result<()> {
    if stack_names.is_empty() {
        return Ok(());
    }
    apply_stacks_from_catalog(stack_names, spec, project_dir, &resolve_stacks_dir())
}

/// Resolve a catalog using the execution snapshot and project-root fallback paths.
pub fn resolve_stacks_dir_with_context(context: &crate::WorkspaceContext) -> PathBuf {
    if let Some(custom) = context
        .config()
        .get_prop_opt(crate::r::keys::VED_STACKS_PATH)
    {
        let path = context.resolve(custom);
        if path.exists() {
            return path;
        }
    }
    let home = context
        .config()
        .interpolation_value("HOME")
        .map(PathBuf::from);
    if let Some(home) = &home {
        let path = home.join(".ved/stacks");
        if path.exists() {
            return path;
        }
    }
    for relative in ["stacks", "../stacks", "../../stacks", "../../../stacks"] {
        let path = context.root().join(relative);
        if path.exists() {
            return path;
        }
    }
    home.map(|home| home.join(".ved/stacks"))
        .unwrap_or_else(|| context.work_dir().join("stacks"))
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PassiveVcsDriver;

impl VcsDriver for PassiveVcsDriver {
    fn execute(
        &self,
        _args: &[&str],
        _cwd: Option<&Path>,
    ) -> std::result::Result<VcsOutput, VcsError> {
        Err(VcsError::Custom(
            "host VCS driver required to synchronize remote stack catalog".into(),
        ))
    }
}

pub fn sync_stacks_catalog_with_context(context: &crate::WorkspaceContext) -> Result<()> {
    StackSyncOptions::new().context(context).sync()
}

/// Sync the contextual catalog using the host application's cancellation policy.
#[allow(dead_code)]
pub fn sync_stacks_catalog_with_context_and_cancellation(
    context: &crate::WorkspaceContext,
    token: ved_process::CancellationToken,
) -> Result<()> {
    StackSyncOptions::new()
        .context(context)
        .cancellation(token)
        .sync()
}

pub fn sync_stacks_catalog_with_context_and_driver<D: VcsDriver>(
    context: &crate::WorkspaceContext,
    driver: &D,
) -> Result<()> {
    StackSyncOptions::new()
        .context(context)
        .driver(driver)
        .sync()
}
pub fn apply_stacks_with_context(
    stack_names: &[String],
    spec: &mut AppSpec,
    context: &crate::WorkspaceContext,
) -> Result<()> {
    let options = LoadOptions::from_config(context.config())?;
    apply_stacks_with_context_and_options(stack_names, spec, context, &options)
}
/// Apply stacks with one finite budget shared across every catalogue evaluation.
pub fn apply_stacks_with_context_and_options(
    stack_names: &[String],
    spec: &mut AppSpec,
    context: &crate::WorkspaceContext,
    options: &LoadOptions,
) -> Result<()> {
    let mut session = LoadSession::new(options)?;
    apply_stacks_with_context_in_session(stack_names, spec, context, &mut session)
}
pub(crate) fn apply_stacks_with_context_in_session(
    stack_names: &[String],
    spec: &mut AppSpec,
    context: &crate::WorkspaceContext,
    session: &mut LoadSession,
) -> Result<()> {
    if stack_names.is_empty() {
        return Ok(());
    }
    apply_stacks_from_catalog_with_context(
        stack_names,
        spec,
        context.root(),
        &resolve_stacks_dir_with_context(context),
        Some(context),
        true,
        session,
    )
    .map(|_| ())
}
/// Evaluate activated stacks without writing copied config files.
pub fn load_stacks_with_context(
    stack_names: &[String],
    spec: &AppSpec,
    context: &crate::WorkspaceContext,
) -> Result<StackApplication> {
    let options = LoadOptions::from_config(context.config())?;
    load_stacks_with_context_and_options(stack_names, spec, context, &options)
}
/// Load stacks without publication, sharing explicit finite limits across all sources.
pub fn load_stacks_with_context_and_options(
    stack_names: &[String],
    spec: &AppSpec,
    context: &crate::WorkspaceContext,
    options: &LoadOptions,
) -> Result<StackApplication> {
    let mut session = LoadSession::new(options)?;
    load_stacks_with_context_in_session(stack_names, spec, context, &mut session)
}
pub(crate) fn load_stacks_with_context_in_session(
    stack_names: &[String],
    spec: &AppSpec,
    context: &crate::WorkspaceContext,
    session: &mut LoadSession,
) -> Result<StackApplication> {
    if stack_names.is_empty() {
        return Ok(StackApplication {
            spec: spec.clone(),
            dependencies: Vec::new(),
            assets: Vec::new(),
        });
    }
    let mut result = spec.clone();
    let (dependencies, assets) = apply_stacks_from_catalog_with_context(
        stack_names,
        &mut result,
        context.root(),
        &resolve_stacks_dir_with_context(context),
        Some(context),
        false,
        session,
    )?;
    Ok(StackApplication {
        spec: result,
        dependencies,
        assets,
    })
}
fn apply_stacks_from_catalog(
    stack_names: &[String],
    spec: &mut AppSpec,
    project_dir: &Path,
    catalog_dir: &Path,
) -> Result<()> {
    let context = crate::WorkspaceContext::new(project_dir);
    let options = LoadOptions::from_config(context.config())?;
    let mut session = LoadSession::new(&options)?;
    apply_stacks_from_catalog_with_context(
        stack_names,
        spec,
        project_dir,
        catalog_dir,
        None,
        true,
        &mut session,
    )
    .map(|_| ())
}
#[allow(clippy::too_many_arguments)]
fn apply_stacks_from_catalog_with_context(
    stack_names: &[String],
    spec: &mut AppSpec,
    project_dir: &Path,
    catalog_dir: &Path,
    context: Option<&crate::WorkspaceContext>,
    publish_assets: bool,
    session: &mut LoadSession,
) -> Result<(Vec<LoadedDependency>, Vec<StackAsset>)> {
    use crate::filesystem::{validate_identifier, ConfinedDir};

    crate::validated::ValidatedSpec::validate_fields(spec)?;
    for name in stack_names {
        validate_identifier("stack", name)?;
    }
    let catalog = ConfinedDir::open_existing(catalog_dir).map_err(|e| {
        contextual_stack_error(e, "catalog traversal", catalog_dir, "<catalog>", vec![])
    })?;
    let destination = match context {
        Some(context) if context.work_dir().exists() => {
            Some(ConfinedDir::open_existing(context.work_dir()).map_err(|e| {
                contextual_stack_error(
                    e,
                    "destination traversal",
                    context.work_dir(),
                    "<catalog>",
                    vec![],
                )
            })?)
        }
        Some(_) => None,
        None => Some(ConfinedDir::open_existing(project_dir).map_err(|e| {
            contextual_stack_error(e, "destination traversal", project_dir, "<catalog>", vec![])
        })?),
    };
    let mut candidate = spec.clone();
    let mut assets = Vec::new();
    let mut dependencies: Vec<LoadedDependency> = Vec::new();
    for stack_name in stack_names {
        let stack_path = Path::new(stack_name);
        let spec_file = stack_path.join(crate::r::files::VEDFILE);
        let spec_source = PathBuf::from("<catalog>").join(&spec_file);
        catalog.preflight(&spec_file).map_err(|e| {
            contextual_stack_error(e, "catalog traversal", &spec_source, stack_name, vec![])
        })?;
        if !catalog.exists(&spec_file).map_err(|e| {
            contextual_stack_error(e, "catalog traversal", &spec_source, stack_name, vec![])
        })? {
            return Err(VedError::stack(
                stack_name,
                format!(
                    "Stack '{}' is missing Vedfile in {}",
                    stack_name,
                    catalog_dir.join(stack_name).display()
                ),
            ));
        }
        let absolute_source = catalog_dir.join(&spec_file);
        session.catalogue_source(&absolute_source)?;
        let content = catalog
            .read_bounded(&spec_file, session.source_limit())
            .map_err(|e| {
                contextual_stack_error(e, "catalog read", &spec_source, stack_name, vec![])
            })?;
        session.check_source(&absolute_source, &content)?;
        dependencies.push(LoadedDependency {
            path: canonical_dependency(&catalog_dir.join(&spec_file)),
            bytes: content.clone(),
        });
        let content = std::str::from_utf8(&content)
            .map_err(|e| VedError::stack(stack_name, format!("Stack Vedfile is not UTF-8: {e}")))?;
        let fallback_context;
        let evaluation_context = match context {
            Some(context) => context,
            None => {
                fallback_context = crate::WorkspaceContext::new(
                    absolute_source.parent().unwrap_or(Path::new(".")),
                );
                &fallback_context
            }
        };
        let stack_services = crate::starlark::evaluate_stack_contents_with_context_in_session(
            content,
            &absolute_source,
            evaluation_context,
            session,
        )
        .map_err(|error| {
            contextual_stack_error(error, "catalog evaluate", &spec_source, stack_name, vec![])
        })?;
        // Check identifiers and typed fields even for overridden catalog workloads.
        // Resolve dependencies against the final merged candidate below.
        let mut stack_spec = AppSpec::default();
        stack_spec.services.extend(stack_services.clone());
        crate::validated::ValidatedSpec::validate_fields(&stack_spec)?;

        let src_config = stack_path.join(crate::r::dirs::CONFIG_DIR);
        let config_source = PathBuf::from("<catalog>").join(&src_config);
        let dest_config = if context.is_some() {
            PathBuf::new()
        } else {
            PathBuf::from(crate::r::dirs::VED_DIR)
        }
        .join(crate::r::dirs::STACKS_DIR)
        .join(stack_name)
        .join(crate::r::dirs::CONFIG_DIR);
        if let Some(destination) = &destination {
            destination.preflight(&dest_config).map_err(|e| {
                let root = context
                    .map(crate::WorkspaceContext::work_dir)
                    .unwrap_or(project_dir);
                contextual_stack_error(
                    e,
                    "destination traversal",
                    root.join(&dest_config),
                    stack_name,
                    vec![],
                )
            })?;
        }
        catalog.preflight(&src_config).map_err(|e| {
            contextual_stack_error(e, "catalog traversal", &config_source, stack_name, vec![])
        })?;
        if catalog.exists(&src_config).map_err(|e| {
            contextual_stack_error(e, "catalog traversal", &config_source, stack_name, vec![])
        })? {
            let mut file_names = catalog.file_names(&src_config).map_err(|e| {
                contextual_stack_error(e, "catalog traversal", &config_source, stack_name, vec![])
            })?;
            file_names.sort();
            for file_name in file_names {
                let dest_file = dest_config.join(&file_name);
                if let Some(destination) = &destination {
                    destination.preflight(&dest_file).map_err(|e| {
                        let root = context
                            .map(crate::WorkspaceContext::work_dir)
                            .unwrap_or(project_dir);
                        contextual_stack_error(
                            e,
                            "destination traversal",
                            root.join(&dest_file),
                            stack_name,
                            vec![],
                        )
                    })?;
                }
                let source_path = config_source.join(&file_name);
                let bytes = catalog.read(src_config.join(&file_name)).map_err(|e| {
                    contextual_stack_error(
                        e,
                        "catalog read",
                        source_path.clone(),
                        stack_name,
                        vec![],
                    )
                })?;
                dependencies.push(LoadedDependency {
                    path: canonical_dependency(
                        &catalog_dir
                            .join(stack_path)
                            .join(crate::r::dirs::CONFIG_DIR)
                            .join(&file_name),
                    ),
                    bytes: bytes.clone(),
                });
                assets.push((stack_name.clone(), dest_file, bytes));
            }
        }

        for (svc_name, mut svc_spec) in stack_services {
            let mut rewritten_mounts = Vec::new();
            for m in &svc_spec.mounts {
                let parts: Vec<&str> = m.split(':').collect();
                if parts.len() >= 2 {
                    let src = parts[0];
                    let dst = parts[1];
                    let mode = parts.get(2).map(|s| format!(":{s}")).unwrap_or_default();
                    let clean_src = src.trim_start_matches("./");
                    if clean_src.starts_with("config/") {
                        let sub = clean_src.trim_start_matches("config/");
                        let new_src = match context {
                            Some(context) if context.work_dir() != context.root().join(".ved") => {
                                context
                                    .work_dir()
                                    .join("stacks")
                                    .join(stack_name)
                                    .join("config")
                                    .join(sub)
                                    .to_string_lossy()
                                    .into_owned()
                            }
                            _ => format!(".ved/stacks/{stack_name}/config/{sub}"),
                        };
                        rewritten_mounts.push(format!("{new_src}:{dst}{mode}"));
                    } else {
                        rewritten_mounts.push(m.clone());
                    }
                } else {
                    rewritten_mounts.push(m.clone());
                }
            }
            svc_spec.mounts = rewritten_mounts;
            if !candidate.services.contains_key(&svc_name) {
                candidate.services.insert(svc_name, svc_spec);
            }
        }
    }
    candidate.validate()?;
    session.check_elapsed(catalog_dir)?;
    let destination = match destination {
        Some(destination) => destination,
        None if publish_assets => {
            let work_dir = context.expect("contextual destination").work_dir();
            ConfinedDir::create(work_dir).map_err(|e| {
                contextual_stack_error(e, "destination creation", work_dir, "<catalog>", vec![])
            })?
        }
        None => {
            let root = context.expect("contextual destination").work_dir();
            *spec = candidate;
            return Ok((
                dependencies,
                assets
                    .into_iter()
                    .map(|(_, relative, bytes)| StackAsset {
                        destination: root.join(relative),
                        bytes,
                    })
                    .collect(),
            ));
        }
    };
    let destination_root = context
        .map(crate::WorkspaceContext::work_dir)
        .unwrap_or(project_dir);
    for (stack_name, relative, _) in &assets {
        destination.preflight(relative).map_err(|e| {
            contextual_stack_error(
                e,
                "destination traversal",
                destination_root.join(relative),
                stack_name,
                vec![],
            )
        })?;
    }
    if !publish_assets {
        *spec = candidate;
        return Ok((
            dependencies,
            assets
                .into_iter()
                .map(|(_, relative, bytes)| StackAsset {
                    destination: destination_root.join(relative),
                    bytes,
                })
                .collect(),
        ));
    }
    session.check_elapsed(catalog_dir)?;
    let mut completed = Vec::new();
    for (stack_name, relative, bytes) in assets {
        if let Some(parent) = relative.parent() {
            destination.create_dir_all(parent).map_err(|e| {
                contextual_stack_error(
                    e,
                    "destination traversal",
                    destination_root.join(parent),
                    &stack_name,
                    completed.clone(),
                )
            })?;
        }
        if let Err(e) = destination.write(&relative, &bytes) {
            return Err(contextual_stack_error(
                e,
                "catalog write",
                destination_root.join(&relative),
                &stack_name,
                completed,
            ));
        }
        completed.push(destination_root.join(relative).display().to_string());
    }
    *spec = candidate;
    Ok((dependencies, Vec::new()))
}

fn canonical_dependency(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn contextual_stack_error(
    error: VedError,
    operation: &'static str,
    path: impl AsRef<Path>,
    stack: &str,
    completed: Vec<String>,
) -> VedError {
    match error {
        VedError::Io(source) => VedError::stack_io(stack, operation, path, source, completed),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct StackFixture(PathBuf);

    impl StackFixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "ved-contained-stacks-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("project")).unwrap();
            fs::create_dir_all(root.join("catalog")).unwrap();
            Self(root)
        }
        fn stack(&self, name: &str, workload: &str) {
            let dir = self.0.join("catalog").join(name);
            fs::create_dir_all(dir.join("config")).unwrap();
            fs::write(
                dir.join("Vedfile"),
                format!("services = [Service(name={workload:?}, image=\"alpine\")]\n"),
            )
            .unwrap();
            fs::write(dir.join("config/settings.txt"), "settings").unwrap();
        }
        fn apply(&self, names: &[&str], spec: &mut AppSpec) -> Result<()> {
            let names = names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
            apply_stacks_from_catalog(
                &names,
                spec,
                &self.0.join("project"),
                &self.0.join("catalog"),
            )
        }
    }
    impl Drop for StackFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn token_aware_git_sync_cancels_without_spawning_or_stops_active_git() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        const CHILD_ENV: &str = "VED_TEST_GIT_CANCELLATION_CHILD";
        if let Ok(mode) = std::env::var(CHILD_ENV) {
            let fixture_root = PathBuf::from(std::env::var("VED_TEST_GIT_FIXTURE").unwrap());
            let root = fixture_root.join("project");
            let locator = crate::config::ConfigLocator::legacy(&root);
            let snapshot = crate::config::ConfigSnapshot::with_environment(
                &locator,
                std::collections::BTreeMap::from([(
                    "VED_STACKS_PATH".into(),
                    fixture_root.join("catalog").to_string_lossy().into_owned(),
                )]),
            );
            let context = crate::WorkspaceContext::with_snapshot(
                root.clone(),
                root.join("config"),
                root.join(".ved"),
                None,
                locator.generated_env_file(),
                snapshot,
            );
            let token = ved_process::CancellationToken::new();
            let ready = fixture_root.join("spawned");
            let canceller = if mode == "active" {
                let token = token.clone();
                let ready = ready.clone();
                Some(std::thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(3);
                    while !ready.exists() && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    token.cancel();
                }))
            } else {
                token.cancel();
                None
            };
            let result = sync_stacks_catalog_with_context_and_cancellation(&context, token);
            if let Some(canceller) = canceller {
                canceller.join().unwrap();
            }
            assert!(matches!(
                result,
                Err(VedError::ProcessOperation {
                    source: ved_process::RunError::Cancelled { .. },
                    ..
                })
            ));
            assert_eq!(ready.exists(), mode == "active");
            return;
        }

        for mode in ["pre", "active"] {
            let fixture = StackFixture::new();
            fs::create_dir_all(fixture.0.join("catalog/.git")).unwrap();
            let bin = fixture.0.join("bin");
            fs::create_dir_all(&bin).unwrap();
            let git = bin.join("git");
            fs::write(
                &git,
                "#!/bin/sh\nprintf ready > \"$VED_TEST_GIT_FIXTURE/spawned\"\nexec /bin/sleep 30\n",
            )
            .unwrap();
            fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "stacks::tests::token_aware_git_sync_cancels_without_spawning_or_stops_active_git", "--nocapture"])
                .env(CHILD_ENV, mode)
                .env("VED_TEST_GIT_FIXTURE", &fixture.0)
                .env("PATH", &bin)
                .spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "{mode} cancellation fixture failed");
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("{mode} cancellation fixture exceeded deadline");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[test]
    fn contextual_stacks_use_snapshot_and_custom_work_directory() {
        let fixture = StackFixture::new();
        fixture.stack("good", "api");
        fs::write(fixture.0.join("catalog/good/Vedfile"), "services = [Service(name='api', image=env.secret('STACK_IMAGE'), mounts=['config/settings.txt:/etc/settings:ro'])]\n").unwrap();
        let root = fixture.0.join("project");
        let work = fixture.0.join("custom-work");
        let locator = crate::config::ConfigLocator::legacy(&root);
        let snapshot = crate::config::ConfigSnapshot::with_environment(
            &locator,
            std::collections::BTreeMap::from([
                (
                    "VED_STACKS_PATH".into(),
                    fixture.0.join("catalog").to_string_lossy().into_owned(),
                ),
                ("STACK_IMAGE".into(), "snapshot-image".into()),
            ]),
        );
        let context = crate::WorkspaceContext::with_snapshot(
            root.clone(),
            root.join("config"),
            work.clone(),
            None,
            locator.generated_env_file(),
            snapshot,
        );
        let mut spec = AppSpec::default();
        assert!(apply_stacks_with_context(&["missing".into()], &mut spec, &context).is_err());
        assert!(!work.exists());
        apply_stacks_with_context(&["good".into()], &mut spec, &context).unwrap();
        assert_eq!(
            spec.services["api"].image.as_deref(),
            Some("snapshot-image")
        );
        assert_eq!(
            spec.services["api"].mounts,
            vec![format!(
                "{}:/etc/settings:ro",
                work.join("stacks/good/config/settings.txt").display()
            )]
        );
        assert_eq!(
            fs::read_to_string(work.join("stacks/good/config/settings.txt")).unwrap(),
            "settings"
        );
        assert!(!root.join(".ved").exists());
    }

    #[test]
    fn deferred_stack_load_returns_assets_without_creating_workdir() {
        let fixture = StackFixture::new();
        fixture.stack("good", "api");
        fs::write(
            fixture.0.join("catalog/good/Vedfile"),
            "services = [Service(name='api', image='alpine', mounts=['config/settings.txt:/etc/settings:ro'])]\n",
        ).unwrap();
        let root = fixture.0.join("project");
        let work = fixture.0.join("not-created-yet");
        let locator = crate::config::ConfigLocator::legacy(&root);
        let snapshot = crate::config::ConfigSnapshot::with_environment(
            &locator,
            std::collections::BTreeMap::from([(
                "VED_STACKS_PATH".into(),
                fixture.0.join("catalog").to_string_lossy().into_owned(),
            )]),
        );
        let context = crate::WorkspaceContext::with_snapshot(
            root.clone(),
            root.join("config"),
            work.clone(),
            None,
            locator.generated_env_file(),
            snapshot,
        );
        let input = AppSpec {
            app: "example".into(),
            version: "1.0.0".into(),
            ..Default::default()
        };
        let loaded = load_stacks_with_context(&["good".into()], &input, &context).unwrap();
        assert!(loaded.spec.services.contains_key("api"));
        assert_eq!(
            loaded.spec.services["api"].mounts,
            vec![format!(
                "{}:/etc/settings:ro",
                work.join("stacks/good/config/settings.txt").display()
            )]
        );
        assert!(!work.exists());
        assert_eq!(loaded.assets.len(), 1);
        assert_eq!(
            loaded.assets[0].destination,
            work.join("stacks/good/config/settings.txt")
        );
        assert_eq!(loaded.assets[0].bytes, b"settings");
        assert_eq!(loaded.dependencies.len(), 2);
        assert!(loaded
            .dependencies
            .iter()
            .any(|dependency| dependency.path.ends_with("Vedfile")));
        assert!(loaded
            .dependencies
            .iter()
            .any(|dependency| dependency.path.ends_with("settings.txt")));
    }

    #[test]
    fn contained_stacks_validate_all_names_before_copying() {
        let fixture = StackFixture::new();
        fixture.stack("good", "api");
        let mut spec = AppSpec::default();
        assert!(fixture.apply(&["good", "../escaped"], &mut spec).is_err());
        assert!(!fixture.0.join("project/.ved").exists());
        assert!(spec.services.is_empty());
    }

    #[test]
    fn stack_io_context_keeps_cause_and_completed_write_list() {
        let path = Path::new("project/.ved/stacks/good/config/second.txt");
        let error = contextual_stack_error(
            VedError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "controlled failure",
            )),
            "catalog write",
            path,
            "good",
            vec!["project/.ved/stacks/good/config/first.txt".into()],
        );
        assert!(error
            .to_string()
            .contains("Stack 'good' catalog write failed"));
        assert!(error.to_string().contains("first.txt"));
        assert_eq!(
            std::error::Error::source(&error).unwrap().to_string(),
            "controlled failure"
        );
    }

    #[test]
    fn contained_stacks_validate_all_services_before_copying() {
        let fixture = StackFixture::new();
        fixture.stack("good", "api");
        fixture.stack("bad", "../escaped");
        let mut spec = AppSpec::default();
        assert!(fixture.apply(&["good", "bad"], &mut spec).is_err());
        assert!(!fixture.0.join("project/.ved").exists());
        assert!(spec.services.is_empty());
    }

    #[test]
    fn contained_stacks_copy_regular_files_and_allow_missing_config() {
        let fixture = StackFixture::new();
        fixture.stack("good", "api");
        fs::create_dir_all(fixture.0.join("catalog/good/config/nested")).unwrap();
        let mut spec = AppSpec::default();
        fixture.apply(&["good"], &mut spec).unwrap();
        assert!(spec.services.contains_key("api"));
        assert_eq!(
            fs::read_to_string(
                fixture
                    .0
                    .join("project/.ved/stacks/good/config/settings.txt")
            )
            .unwrap(),
            "settings"
        );
        assert!(!fixture
            .0
            .join("project/.ved/stacks/good/config/nested")
            .exists());
        fs::remove_dir_all(fixture.0.join("catalog/good/config")).unwrap();
        fixture.apply(&["good"], &mut spec).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn contained_stacks_reject_source_and_destination_symlinks() {
        use std::os::unix::fs::symlink;
        for location in [
            "stack",
            "spec",
            "config",
            "asset",
            "destination",
            "destination-file",
        ] {
            let fixture = StackFixture::new();
            fixture.stack("good", "api");
            let outside = fixture.0.join("outside");
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("sentinel"), "unchanged").unwrap();
            let catalog = fixture.0.join("catalog/good");
            match location {
                "stack" => {
                    fs::rename(&catalog, outside.join("stack")).unwrap();
                    symlink(outside.join("stack"), &catalog).unwrap();
                }
                "spec" => {
                    fs::rename(catalog.join("Vedfile"), outside.join("Vedfile")).unwrap();
                    symlink(outside.join("Vedfile"), catalog.join("Vedfile")).unwrap();
                }
                "config" => {
                    fs::rename(catalog.join("config"), outside.join("config")).unwrap();
                    symlink(outside.join("config"), catalog.join("config")).unwrap();
                }
                "asset" => {
                    symlink(outside.join("sentinel"), catalog.join("config/linked.txt")).unwrap();
                }
                "destination" => {
                    symlink(&outside, fixture.0.join("project/.ved")).unwrap();
                }
                _ => {
                    let dest = fixture.0.join("project/.ved/stacks/good/config");
                    fs::create_dir_all(&dest).unwrap();
                    symlink(outside.join("sentinel"), dest.join("settings.txt")).unwrap();
                }
            }
            let mut spec = AppSpec::default();
            assert!(fixture.apply(&["good"], &mut spec).is_err(), "{}", location);
            assert!(spec.services.is_empty());
            assert_eq!(
                fs::read_to_string(outside.join("sentinel")).unwrap(),
                "unchanged"
            );
        }
    }

    struct MockGitRunner {
        calls: Mutex<Vec<Vec<String>>>,
        result: std::result::Result<VcsOutput, std::io::ErrorKind>,
    }

    impl MockGitRunner {
        fn success() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Ok(VcsOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: Some(0),
                }),
            }
        }

        fn failure(exit_code: Option<i32>, stdout: &str, stderr: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Ok(VcsOutput {
                    stdout: stdout.to_string(),
                    stderr: stderr.to_string(),
                    exit_code,
                }),
            }
        }

        fn io_error(kind: std::io::ErrorKind) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Err(kind),
            }
        }

        fn recorded_calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl VcsDriver for MockGitRunner {
        fn execute(
            &self,
            args: &[&str],
            _cwd: Option<&Path>,
        ) -> std::result::Result<VcsOutput, VcsError> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            match &self.result {
                Ok(output) => Ok(output.clone()),
                Err(kind) => Err(VcsError::Io(std::io::Error::new(*kind, "mock io error"))),
            }
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ved-test-stacks-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn test_sync_pull_success() {
        let dir = test_dir("pull-success");
        fs::create_dir_all(dir.join(".git")).unwrap();

        let runner = MockGitRunner::success();
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        assert!(res.is_ok());
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec!["-C", dir.to_str().unwrap(), "pull", "--quiet"]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_pull_failure_reports_status_without_stderr() {
        let dir = test_dir("pull-fail");
        fs::create_dir_all(dir.join(".git")).unwrap();

        let runner =
            MockGitRunner::failure(Some(1), "", "fatal: unable to access: Connection timed out");
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        match res {
            Err(VedError::StacksSyncError {
                operation,
                repo,
                details,
            }) => {
                assert_eq!(operation, "pull");
                assert_eq!(repo, dir.to_str().unwrap());
                assert_eq!(details, "git command exited with status 1");
            }
            other => panic!("Expected StacksSyncError, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_clone_success() {
        let dir = test_dir("clone-success");
        assert!(!dir.exists());

        let runner = MockGitRunner::success();
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        assert!(res.is_ok());
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec![
                "clone",
                "https://example.com/stacks.git",
                dir.to_str().unwrap()
            ]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_clone_failure_redacts_captured_output() {
        let dir = test_dir("clone-fail");
        assert!(!dir.exists());

        let runner = MockGitRunner::failure(Some(128), "", "fatal: credential=secret-marker");
        let res = sync_stacks_catalog_internal(
            &runner,
            &dir,
            "https://user:password-marker@example.com/stacks.git",
        );

        match res {
            Err(VedError::StacksSyncError {
                operation,
                repo,
                details,
            }) => {
                assert_eq!(operation, "clone");
                assert_eq!(repo, "https://[redacted]@example.com/stacks.git");
                assert_eq!(details, "git command exited with status 128");
                assert!(!details.contains("secret-marker"));
                assert!(!repo.contains("password-marker"));
            }
            other => panic!("Expected StacksSyncError, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repo_credential_redaction_preserves_noncredential_targets() {
        assert_eq!(
            redact_url_credentials("https://user:secret@example.test/repo.git"),
            "https://[redacted]@example.test/repo.git"
        );
        assert_eq!(
            redact_url_credentials("/home/user/stacks"),
            "/home/user/stacks"
        );
        assert_eq!(
            redact_url_credentials("https://example.test/repo.git"),
            "https://example.test/repo.git"
        );
        assert_eq!(
            redact_url_credentials("https://example.test/repo.git?token=secret"),
            "https://example.test/repo.git"
        );
    }

    #[test]
    fn test_sync_git_not_found_in_path() {
        let dir = test_dir("not-found");
        assert!(!dir.exists());

        let runner = MockGitRunner::io_error(std::io::ErrorKind::NotFound);
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        match res {
            Err(VedError::StacksSyncError {
                operation,
                repo,
                details,
            }) => {
                assert_eq!(operation, "clone");
                assert_eq!(repo, "https://example.com/stacks.git");
                assert_eq!(details, "git executable not found in PATH");
            }
            other => panic!("Expected StacksSyncError, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_local_override_skips_git() {
        let dir = test_dir("local-override");
        fs::create_dir_all(&dir).unwrap();

        let runner = MockGitRunner::success();
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        assert!(res.is_ok());
        assert!(
            runner.recorded_calls().is_empty(),
            "Git runner must not be invoked for local override without .git"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_signal_termination() {
        let dir = test_dir("signal-term");
        fs::create_dir_all(dir.join(".git")).unwrap();

        let runner = MockGitRunner::failure(None, "", "");
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        match res {
            Err(VedError::StacksSyncError { details, .. }) => {
                assert_eq!(details, "git process terminated by signal");
            }
            other => panic!("Expected StacksSyncError, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_stdout_failure_is_redacted() {
        let dir = test_dir("fallback-stdout");
        fs::create_dir_all(dir.join(".git")).unwrap();

        let runner = MockGitRunner::failure(Some(1), "credential=secret-marker", "");
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        match res {
            Err(VedError::StacksSyncError { details, .. }) => {
                assert_eq!(details, "git command exited with status 1");
                assert!(!details.contains("secret-marker"));
            }
            other => panic!("Expected StacksSyncError, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_empty_stderr_and_stdout_falls_back_to_exit_code() {
        let dir = test_dir("fallback-exit-code");
        fs::create_dir_all(dir.join(".git")).unwrap();

        let runner = MockGitRunner::failure(Some(127), "", "");
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        match res {
            Err(VedError::StacksSyncError { details, .. }) => {
                assert_eq!(details, "git command exited with status 127");
            }
            other => panic!("Expected StacksSyncError, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sync_other_io_error_propagates_details() {
        let dir = test_dir("io-error");
        assert!(!dir.exists());

        let runner = MockGitRunner::io_error(std::io::ErrorKind::PermissionDenied);
        let res = sync_stacks_catalog_internal(&runner, &dir, "https://example.com/stacks.git");

        match res {
            Err(VedError::StacksSyncError { details, .. }) => {
                assert_eq!(details, "mock io error");
            }
            other => panic!("Expected StacksSyncError, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_stack_sync_options_builder() {
        let dir = test_dir("builder-sync");
        let runner = MockGitRunner::success();
        let res = StackSyncOptions::new()
            .driver(&runner)
            .stacks_dir(&dir)
            .repo_url("https://example.com/stacks.git")
            .sync();
        assert!(res.is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
