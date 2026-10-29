use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ved_core::{AppSpec, KubernetesRoutingPlan, LoadedSpec, VedCompiler};

use crate::artifact_publisher::{ArtifactLease, ArtifactPublisher, ScopeUpdate};
use crate::cli::{CompileArgs, CompileTarget, Slot};
use crate::context::WorkspaceContext;
use crate::error::{CliError, Result};
use crate::output::{Outputter, TerminalOutputter};
use crate::process::default_compose_path;
use crate::spec::resolve_target_with_ctx;

pub(crate) fn handle_compile_with_output(
    ctx: &WorkspaceContext,
    args: CompileArgs,
    output: &dyn Outputter,
) -> Result<()> {
    let tgt = resolve_target_with_ctx(ctx, args.target)?;
    let _ = run_compile_with_output(ctx, args.file, tgt, &args.org, output)?;
    Ok(())
}

pub(crate) fn run_compile_with_ctx(
    ctx: &WorkspaceContext,
    file: Option<PathBuf>,
    target: CompileTarget,
    _org: &str,
) -> Result<AppSpec> {
    run_compile_with_output(ctx, file, target, _org, &TerminalOutputter)
}

pub(crate) fn run_compile_with_output(
    ctx: &WorkspaceContext,
    file: Option<PathBuf>,
    target: CompileTarget,
    _org: &str,
    output: &dyn Outputter,
) -> Result<AppSpec> {
    let spec_path = ctx.spec_path(file.as_deref());
    if !spec_path.exists() {
        return Err(CliError::spec_not_found(spec_path));
    }
    let loaded = ved_core::load_any_spec_with_context_and_dependencies(&spec_path, ctx.core())?;
    let updated_ctx = ctx.with_spec(&loaded.spec)?;
    let ctx = &updated_ctx;
    let layout = artifact_layout(ctx);
    let roots = trusted_roots(ctx, &layout);
    let lease = ArtifactPublisher::new(publisher_root(ctx)).lease(&roots)?;
    run_compile_with_lease_and_output(ctx, file, target, &lease, output)
}

pub(crate) fn run_compile_with_lease_and_output(
    ctx: &WorkspaceContext,
    file: Option<PathBuf>,
    target: CompileTarget,
    lease: &ArtifactLease,
    output: &dyn Outputter,
) -> Result<AppSpec> {
    let spec_path = ctx.spec_path(file.as_deref());
    if !spec_path.exists() {
        return Err(CliError::spec_not_found(spec_path));
    }
    let loaded = ved_core::load_any_spec_with_context_and_dependencies(&spec_path, ctx.core())?;
    let updated_ctx = ctx.with_spec(&loaded.spec)?;
    let ctx = &updated_ctx;
    let spec = loaded.spec.clone();
    let locator = ctx.config();
    let comp_path_str = locator.get_prop(ved_core::r::keys::COMPOSE_PATH, default_compose_path());
    let layout = ctx.artifact_layout(&comp_path_str);
    let plan = if matches!(target, CompileTarget::Kubernetes | CompileTarget::All) {
        let slot = Slot(ctx.config().get_prop(ved_core::r::keys::K8S_APP_SLOT, "1"));
        Some(KubernetesRoutingPlan::build(
            &spec,
            ctx.namespace_resolver(&slot),
        )?)
    } else {
        None
    };
    let compiler = VedCompiler::with_context(ctx.core().clone());
    let generated_env = generated_env_content(
        ctx,
        &spec,
        if matches!(target, CompileTarget::Compose | CompileTarget::All) {
            Some(&comp_path_str)
        } else {
            None
        },
    );
    let mut updates = BTreeMap::new();
    let mut fingerprints = BTreeMap::new();

    if matches!(target, CompileTarget::Compose | CompileTarget::All) {
        let bytes = compiler.compile_compose(&spec)?.into_bytes();
        insert_compose_artifacts(ctx, &mut updates, &layout.compose_file, bytes)?;
        insert_single(
            &mut updates,
            "generated-env",
            &ctx.paths.generated_env_file(),
            generated_env.clone(),
        )?;
    }
    if matches!(target, CompileTarget::Kubernetes | CompileTarget::All) {
        let plan = plan.as_ref().expect("Kubernetes plan was selected");
        let files = render_k8s(ctx, &spec, plan)?;
        let mut files = files;
        files.insert(
            PathBuf::from(".ved-routing-plan"),
            plan.fingerprint().into_bytes(),
        );
        updates.insert(
            "k8s".into(),
            ScopeUpdate {
                root: layout.k8s_dir.clone(),
                files,
            },
        );
        insert_single(
            &mut updates,
            "generated-env",
            &ctx.paths.generated_env_file(),
            generated_env.clone(),
        )?;
    }
    if matches!(target, CompileTarget::Helm | CompileTarget::All) {
        let bytes = compiler.compile_helm(&spec)?.into_bytes();
        insert_single(&mut updates, "helm", &layout.helm_values, bytes)?;
    }
    add_stack_assets(ctx, &loaded, &mut updates)?;

    match target {
        CompileTarget::Compose => {
            fingerprints.insert(
                "compose".into(),
                fingerprint(ctx, &loaded, "compose", &layout, None)?,
            );
        }
        CompileTarget::Kubernetes => {
            fingerprints.insert(
                "k8s".into(),
                fingerprint(ctx, &loaded, "k8s", &layout, plan.as_ref())?,
            );
        }
        CompileTarget::Helm => {
            fingerprints.insert(
                "helm".into(),
                fingerprint(ctx, &loaded, "helm", &layout, None)?,
            );
        }
        CompileTarget::All => {
            fingerprints.insert(
                "compose".into(),
                fingerprint(ctx, &loaded, "compose", &layout, None)?,
            );
            fingerprints.insert(
                "k8s".into(),
                fingerprint(ctx, &loaded, "k8s", &layout, plan.as_ref())?,
            );
            fingerprints.insert(
                "helm".into(),
                fingerprint(ctx, &loaded, "helm", &layout, None)?,
            );
        }
    }
    lease.publish(updates, fingerprints)?;
    match target {
        CompileTarget::Compose => {
            output.println(&format!("compile: wrote {}", layout.compose_file.display()))
        }
        CompileTarget::Kubernetes => output.println(&format!(
            "compile: wrote k8s manifests to {}",
            layout.k8s_dir.display()
        )),
        CompileTarget::Helm => {
            output.println(&format!("compile: wrote {}", layout.helm_values.display()))
        }
        CompileTarget::All => output.println("compile: wrote all targets"),
    }
    output.println("compile: done");
    Ok(spec)
}

/// Implicit Kubernetes deployment compilation uses the same transactional publisher.
pub(crate) fn run_compile_k8s_with_lease_and_output(
    ctx: &WorkspaceContext,
    loaded: &LoadedSpec,
    plan: &KubernetesRoutingPlan,
    lease: &ArtifactLease,
    output: &dyn Outputter,
) -> Result<()> {
    let spec = &loaded.spec;
    let locator = ctx.config();
    let comp_path_str = locator.get_prop(ved_core::r::keys::COMPOSE_PATH, default_compose_path());
    let layout = ctx.artifact_layout(&comp_path_str);
    let mut updates = BTreeMap::new();
    let mut files = render_k8s(ctx, spec, plan)?;
    files.insert(
        PathBuf::from(".ved-routing-plan"),
        plan.fingerprint().into_bytes(),
    );
    updates.insert(
        "k8s".into(),
        ScopeUpdate {
            root: layout.k8s_dir.clone(),
            files,
        },
    );
    insert_single(
        &mut updates,
        "generated-env",
        &ctx.paths.generated_env_file(),
        generated_env_content(ctx, spec, None),
    )?;
    add_stack_assets(ctx, loaded, &mut updates)?;
    let cache = BTreeMap::from([(
        "k8s".into(),
        fingerprint(ctx, loaded, "k8s", &layout, Some(plan))?,
    )]);
    lease.publish(updates, cache)?;
    output.println(&format!(
        "compile: wrote k8s manifests to {}",
        layout.k8s_dir.display()
    ));
    Ok(())
}

fn generated_env_content(
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    comp_path: Option<&str>,
) -> Vec<u8> {
    let locator = ctx.config();
    let env_files_str = spec.env_files.join(" ");
    let ingress_suffix = locator.resolve_ingress_suffix(spec.ingress.as_deref());
    let k8s_namespace = locator.get_prop(
        "K8S_NAMESPACE",
        &locator.get_prop(ved_core::r::keys::VED_K8S_NAMESPACE, "default"),
    );
    let text = if let Some(comp_path) = comp_path {
        format!(
            "{}={}\nAPP_VERSION={}\nK8S_NAMESPACE={}\nVED_ENV_FILES=\"{}\"\n{}={}\n",
            ved_core::r::keys::INGRESS_SUFFIX,
            ingress_suffix,
            spec.version,
            k8s_namespace,
            env_files_str,
            ved_core::r::keys::COMPOSE_PATH,
            comp_path
        )
    } else {
        format!(
            "{}={}\nAPP_VERSION={}\nK8S_NAMESPACE={}\nVED_ENV_FILES=\"{}\"\n",
            ved_core::r::keys::INGRESS_SUFFIX,
            ingress_suffix,
            spec.version,
            k8s_namespace,
            env_files_str
        )
    };
    text.into_bytes()
}

fn insert_compose_artifacts(
    ctx: &WorkspaceContext,
    updates: &mut BTreeMap<String, ScopeUpdate>,
    destination: &Path,
    bytes: Vec<u8>,
) -> Result<()> {
    let root = destination
        .parent()
        .ok_or_else(|| {
            CliError::validation_failed(destination, "artifact has no parent directory")
        })?
        .to_path_buf();
    let relative = destination
        .file_name()
        .ok_or_else(|| CliError::validation_failed(destination, "artifact has no file name"))?;
    let mut files = BTreeMap::from([(PathBuf::from(relative), bytes)]);
    let project_env = ctx.root().join(ved_core::r::files::ENV_FILE);
    let target_env = root.join(ved_core::r::files::ENV_FILE);
    if root != ctx.root()
        && target_env != project_env
        && target_env != ctx.config_locator().env_file()
        && fs::symlink_metadata(&project_env).is_ok_and(|meta| meta.file_type().is_file())
    {
        files.insert(
            PathBuf::from(ved_core::r::files::ENV_FILE),
            fs::read(&project_env)?,
        );
    }
    updates.insert("compose".into(), ScopeUpdate { root, files });
    Ok(())
}

fn insert_single(
    updates: &mut BTreeMap<String, ScopeUpdate>,
    name: &str,
    destination: &Path,
    bytes: Vec<u8>,
) -> Result<()> {
    let root = destination
        .parent()
        .ok_or_else(|| {
            CliError::validation_failed(destination, "artifact has no parent directory")
        })?
        .to_path_buf();
    let relative = destination
        .file_name()
        .ok_or_else(|| CliError::validation_failed(destination, "artifact has no file name"))?;
    updates.insert(
        name.into(),
        ScopeUpdate {
            root,
            files: BTreeMap::from([(PathBuf::from(relative), bytes)]),
        },
    );
    Ok(())
}

fn render_k8s(
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    plan: &KubernetesRoutingPlan,
) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    Ok(ved_core::render_k8s_validated_with_plan(
        &ved_core::ValidatedSpec::try_from(spec)?,
        ctx.core(),
        plan,
    )?)
}

fn add_stack_assets(
    ctx: &WorkspaceContext,
    loaded: &LoadedSpec,
    updates: &mut BTreeMap<String, ScopeUpdate>,
) -> Result<()> {
    let root = ctx.work_dir().to_path_buf();
    let mut files = BTreeMap::new();
    for asset in &loaded.stack_assets {
        let relative = asset.destination.strip_prefix(&root).map_err(|_| {
            CliError::validation_failed(
                &asset.destination,
                "stack asset destination is outside the work directory",
            )
        })?;
        files.insert(relative.to_path_buf(), asset.bytes.clone());
    }
    updates.insert("stack-assets".into(), ScopeUpdate { root, files });
    Ok(())
}

pub(crate) fn trusted_roots(
    ctx: &WorkspaceContext,
    layout: &crate::paths::ArtifactLayout,
) -> BTreeMap<String, PathBuf> {
    // Explicit output is the writable build root. When callers supply only a work
    // directory, keep generated deployment artifacts there rather than under sources.
    let compose_root = layout.compose_file.parent().unwrap_or(Path::new("."));
    let generated_env_file = ctx.paths.generated_env_file();
    let generated_env_root = generated_env_file.parent().unwrap_or(Path::new("."));
    BTreeMap::from([
        ("compose".into(), compose_root.to_path_buf()),
        ("k8s".into(), layout.k8s_dir.clone()),
        (
            "helm".into(),
            layout
                .helm_values
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf(),
        ),
        ("generated-env".into(), generated_env_root.to_path_buf()),
        ("stack-assets".into(), ctx.work_dir().to_path_buf()),
    ])
}
pub(crate) fn artifact_layout(ctx: &WorkspaceContext) -> crate::paths::ArtifactLayout {
    let compose_path = ctx
        .config()
        .get_prop(ved_core::r::keys::COMPOSE_PATH, default_compose_path());
    ctx.artifact_layout(&compose_path)
}
pub(crate) fn artifact_lease(ctx: &WorkspaceContext) -> Result<ArtifactLease> {
    let layout = artifact_layout(ctx);
    ArtifactPublisher::new(publisher_root(ctx)).lease(&trusted_roots(ctx, &layout))
}
pub(crate) fn publisher_root(ctx: &WorkspaceContext) -> PathBuf {
    if !ctx.paths.work_dir_explicit {
        if let Some(output) = &ctx.paths.output_dir {
            return output.join(ved_core::r::dirs::VED_DIR);
        }
    }
    ctx.work_dir().to_path_buf()
}

pub(crate) fn fingerprint(
    ctx: &WorkspaceContext,
    loaded: &LoadedSpec,
    target: &str,
    layout: &crate::paths::ArtifactLayout,
    plan: Option<&KubernetesRoutingPlan>,
) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(target.as_bytes());
    let generated_env = ctx.paths.generated_env_file();
    for path in [
        &layout.compose_file,
        &layout.k8s_dir,
        &layout.helm_values,
        &generated_env,
    ] {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(&[0]);
    }
    if let Some(plan) = plan {
        hasher.update(plan.fingerprint().as_bytes());
    }
    for dependency in &loaded.dependencies {
        // The evaluator snapshot includes the generated env file. Hash its raw
        // inputs below instead to avoid making an output an input to its own cache.
        if dependency.path == Path::new("<config-snapshot>") {
            continue;
        }
        hasher.update(dependency.path.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        hasher.update(&dependency.bytes);
        hasher.update(&[0]);
    }
    let project_env = ctx.root().join(ved_core::r::files::ENV_FILE);
    for path in loaded
        .spec
        .env_files
        .iter()
        .map(|env_file| {
            let path = Path::new(env_file);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                ctx.root().join(path)
            }
        })
        .chain(std::iter::once(project_env))
    {
        hasher.update(path.to_string_lossy().as_bytes());
        match fs::read(&path) {
            Ok(bytes) => {
                hasher.update(b"present");
                hasher.update(&bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"missing");
            }
            Err(error) => return Err(error.into()),
        }
        hasher.update(&[0]);
    }
    if let Ok(environment) = serde_json::to_vec(ctx.core().config().environment()) {
        hasher.update(&environment);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[allow(dead_code)]
pub(crate) fn run_compile_in(
    root: &Path,
    file: Option<PathBuf>,
    target: CompileTarget,
    output_dir: &Path,
) -> Result<AppSpec> {
    let out = if output_dir == root || output_dir == Path::new(".") {
        None
    } else {
        Some(output_dir.to_path_buf())
    };
    let paths = crate::paths::WorkspacePaths {
        root: root.to_path_buf(),
        config_dir: root.join(ved_core::r::dirs::CONFIG_DIR),
        config_dir_explicit: false,
        work_dir: root.join(ved_core::r::dirs::VED_DIR),
        work_dir_explicit: false,
        output_dir: out,
    };
    let ctx = WorkspaceContext::with_paths(paths)?;
    run_compile_with_ctx(&ctx, file, target, "local")
}
#[allow(dead_code)]
pub(crate) fn run_compile(
    file: Option<PathBuf>,
    target: CompileTarget,
    output_dir: &Path,
) -> Result<AppSpec> {
    run_compile_in(Path::new("."), file, target, output_dir)
}
