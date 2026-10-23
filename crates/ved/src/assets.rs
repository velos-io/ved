use std::fs;
use std::path::{Path, PathBuf};

use ved_core::{env_file_in, local_env_file_in, AppSpec, KubernetesRoutingPlan};

use crate::cli::Slot;
use crate::cli::{CompileTarget, DeployTarget};
use crate::error::{CliError, Result};
use crate::output::{Outputter, TerminalOutputter};
use crate::process::default_compose_path;

pub(crate) fn get_latest_input_mtime_in(
    root: &Path,
    spec_path: &Path,
) -> Option<std::time::SystemTime> {
    let mut latest = fs::metadata(spec_path).and_then(|m| m.modified()).ok();
    for p in &[local_env_file_in(root), env_file_in(root)] {
        if let Ok(mtime) = fs::metadata(p).and_then(|m| m.modified()) {
            latest = match latest {
                Some(prev) => Some(prev.max(mtime)),
                None => Some(mtime),
            };
        }
    }
    latest
}

#[allow(dead_code)]
pub(crate) fn get_latest_input_mtime(spec_path: &Path) -> Option<std::time::SystemTime> {
    get_latest_input_mtime_in(Path::new("."), spec_path)
}

use crate::context::WorkspaceContext;

pub(crate) fn assets_up_to_date_with_ctx(
    ctx: &WorkspaceContext,
    spec_path: &Path,
    target: &str,
) -> bool {
    assets_up_to_date_with_plan(ctx, spec_path, target, None)
}

fn assets_up_to_date_with_plan(
    ctx: &WorkspaceContext,
    spec_path: &Path,
    target: &str,
    plan: Option<&KubernetesRoutingPlan>,
) -> bool {
    let loaded = match ved_core::load_any_spec_with_context_and_dependencies(spec_path, ctx.core())
    {
        Ok(loaded) => loaded,
        Err(_) => return false,
    };
    let comp_path_str = ctx
        .config()
        .get_prop(ved_core::r::keys::COMPOSE_PATH, default_compose_path());
    let layout = ctx.artifact_layout(&comp_path_str);

    let fingerprint =
        match crate::commands::compile::fingerprint(ctx, &loaded, target, &layout, plan) {
            Ok(fingerprint) => fingerprint,
            Err(_) => return false,
        };
    crate::artifact_publisher::ArtifactPublisher::new(crate::commands::compile::publisher_root(ctx))
        .is_fresh(
            target,
            &fingerprint,
            &crate::commands::compile::trusted_roots(ctx, &layout),
        )
}

pub(crate) fn assets_up_to_date_in(root: &Path, spec_path: &Path, target: &str) -> bool {
    let Ok(ctx) = WorkspaceContext::new(root.to_path_buf()) else {
        return false;
    };
    assets_up_to_date_with_ctx(&ctx, spec_path, target)
}

#[allow(dead_code)]
pub(crate) fn assets_up_to_date(spec_path: &Path, target: &str) -> bool {
    assets_up_to_date_in(Path::new("."), spec_path, target)
}

pub(crate) fn ensure_assets_with_ctx(
    ctx: &WorkspaceContext,
    file: Option<PathBuf>,
    target: DeployTarget,
) -> Result<AppSpec> {
    ensure_assets_with_output(ctx, file, target, &TerminalOutputter)
}

pub(crate) fn ensure_assets_with_output(
    ctx: &WorkspaceContext,
    file: Option<PathBuf>,
    target: DeployTarget,
    output: &dyn Outputter,
) -> Result<AppSpec> {
    let spec_path = ctx.spec_path(file.as_deref());
    if !spec_path.exists() {
        return Err(CliError::spec_not_found(spec_path));
    }

    let loaded = ved_core::load_any_spec_with_context_and_dependencies(&spec_path, ctx.core())?;
    let updated_ctx = ctx.with_spec(&loaded.spec)?;
    let ctx = &updated_ctx;
    let lease = crate::commands::compile::artifact_lease(ctx)?;
    let layout = crate::commands::compile::artifact_layout(ctx);
    let fingerprint =
        crate::commands::compile::fingerprint(ctx, &loaded, target.as_str(), &layout, None)?;
    if !lease.is_fresh(target.as_str(), &fingerprint) {
        output.println(&format!(
            "hint: deployment assets missing or outdated for target '{}'; compiling implicitly",
            target.as_str()
        ));
        let compile_target = match target {
            DeployTarget::Compose => CompileTarget::Compose,
            DeployTarget::Kubernetes => CompileTarget::Kubernetes,
        };
        crate::commands::compile::run_compile_with_lease_and_output(
            ctx,
            file,
            compile_target,
            &lease,
            output,
        )
    } else {
        Ok(loaded.spec)
    }
}

pub(crate) fn ensure_k8s_assets_with_plan_and_output(
    ctx: &WorkspaceContext,
    file: Option<PathBuf>,
    slot: &Slot,
    output: &dyn Outputter,
) -> Result<(AppSpec, KubernetesRoutingPlan)> {
    let spec_path = ctx.spec_path(file.as_deref());
    if !spec_path.exists() {
        return Err(CliError::spec_not_found(spec_path));
    }
    let loaded = ved_core::load_any_spec_with_context_and_dependencies(&spec_path, ctx.core())?;
    let updated_ctx = ctx.with_spec(&loaded.spec)?;
    let ctx = &updated_ctx;
    let lease = crate::commands::compile::artifact_lease(ctx)?;
    let spec = loaded.spec.clone();
    let plan = KubernetesRoutingPlan::build(&spec, ctx.namespace_resolver(slot))?;
    let layout = crate::commands::compile::artifact_layout(ctx);
    let fingerprint =
        crate::commands::compile::fingerprint(ctx, &loaded, "k8s", &layout, Some(&plan))?;
    let fresh = lease.is_fresh("k8s", &fingerprint);
    if !fresh {
        output.println("hint: Kubernetes assets missing, outdated, or routed for another slot; compiling implicitly");
        crate::commands::compile::run_compile_k8s_with_lease_and_output(
            ctx, &loaded, &plan, &lease, output,
        )?;
    }
    Ok((spec, plan))
}

pub(crate) fn ensure_assets_in(
    root: &Path,
    file: Option<PathBuf>,
    target: DeployTarget,
) -> Result<AppSpec> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    ensure_assets_with_ctx(&ctx, file, target)
}

#[allow(dead_code)]
pub(crate) fn ensure_assets(file: Option<PathBuf>, target: DeployTarget) -> Result<AppSpec> {
    ensure_assets_in(Path::new("."), file, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CompileTarget;
    use crate::context::WorkspaceContext;
    use crate::paths::{CliDirs, WorkspacePaths};

    #[test]
    fn test_assets_up_to_date_freshness() {
        let tmp = std::env::temp_dir().join(format!("ved-test-freshness-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join(ved_core::r::dirs::CONFIG_DIR)).expect("create config dir");

        let spec_file = tmp.join(ved_core::r::files::VED_YAML);
        fs::write(&spec_file, "app: test\nversion: '1'\nservices: {}\n").expect("write spec");
        let paths = WorkspacePaths::resolve_with_env(
            &CliDirs {
                dir: Some(tmp.clone()),
                ..Default::default()
            },
            Path::new("/"),
            |_| None,
        );
        let ctx = WorkspaceContext::with_paths(paths).unwrap();
        assert!(!assets_up_to_date_in(&tmp, &spec_file, "compose"));
        let compile_output = crate::output::BufferOutputter::default();
        crate::commands::compile::run_compile_with_output(
            &ctx,
            None,
            CompileTarget::Compose,
            "local",
            &compile_output,
        )
        .expect("compile compose");
        assert_eq!(
            compile_output.stdout_lines(),
            [
                format!(
                    "compile: wrote {}",
                    tmp.join(".ved/docker-compose.yml").display()
                ),
                "compile: done".to_string(),
            ]
        );
        assert!(assets_up_to_date_in(&tmp, &spec_file, "compose"));
        assert!(
            !tmp.join("config/.env").exists(),
            "compile must not create declared input env files"
        );
        fs::write(tmp.join(".ved/docker-compose.yml"), "tampered").expect("tamper output");
        assert!(
            !assets_up_to_date_in(&tmp, &spec_file, "compose"),
            "changed output must make cache stale"
        );
        let implicit_output = crate::output::BufferOutputter::default();
        ensure_assets_with_output(&ctx, None, DeployTarget::Compose, &implicit_output)
            .expect("implicit compile should recover tampered output");
        assert_eq!(implicit_output.stdout_lines()[0], "hint: deployment assets missing or outdated for target 'compose'; compiling implicitly");
        assert_eq!(
            implicit_output.stdout_lines().last().unwrap(),
            "compile: done"
        );
        crate::commands::compile::run_compile_with_ctx(&ctx, None, CompileTarget::Compose, "local")
            .expect("regenerate compose");

        fs::write(env_file_in(&tmp), "NEW_KEY=val").expect("write .env");
        let refreshed = WorkspaceContext::with_paths(WorkspacePaths::resolve_with_env(
            &CliDirs {
                dir: Some(tmp.clone()),
                ..Default::default()
            },
            Path::new("/"),
            |_| None,
        ))
        .unwrap();
        assert!(
            !assets_up_to_date_with_ctx(&refreshed, &spec_file, "compose"),
            "input change must invalidate cache"
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
