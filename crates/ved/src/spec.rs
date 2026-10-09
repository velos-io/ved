use std::path::{Path, PathBuf};

use crate::cli::{CompileTarget, DeployTarget};
use crate::context::WorkspaceContext;
use crate::error::{CliError, Result};

pub(crate) fn resolve_spec_file_in(root: &Path, file: Option<PathBuf>) -> PathBuf {
    if let Some(f) = file {
        if f.is_absolute() {
            f
        } else {
            root.join(f)
        }
    } else if root.join(ved_core::r::files::VEDFILE).exists() {
        root.join(ved_core::r::files::VEDFILE)
    } else if root.join(ved_core::r::files::VED_YAML).exists() {
        root.join(ved_core::r::files::VED_YAML)
    } else if root.join("spec.ved").exists() {
        root.join("spec.ved")
    } else if root.join("app.ved").exists() {
        root.join("app.ved")
    } else {
        root.join(ved_core::r::files::VEDFILE)
    }
}

#[allow(dead_code)]
pub(crate) fn resolve_spec_file(file: Option<PathBuf>) -> PathBuf {
    resolve_spec_file_in(Path::new("."), file)
}

pub(crate) trait ResolvableTarget: Sized {
    fn parse_target(s: &str) -> Option<Self>;
    fn valid_choices() -> &'static str;
    fn default_target() -> Self;
}

impl ResolvableTarget for DeployTarget {
    fn parse_target(s: &str) -> Option<Self> {
        match s {
            "compose" => Some(DeployTarget::Compose),
            "k8s" => Some(DeployTarget::Kubernetes),
            _ => None,
        }
    }

    fn valid_choices() -> &'static str {
        "'compose', 'k8s'"
    }

    fn default_target() -> Self {
        DeployTarget::Compose
    }
}

impl ResolvableTarget for CompileTarget {
    fn parse_target(s: &str) -> Option<Self> {
        match s {
            "compose" => Some(CompileTarget::Compose),
            "k8s" => Some(CompileTarget::Kubernetes),
            "helm" => Some(CompileTarget::Helm),
            "all" => Some(CompileTarget::All),
            _ => None,
        }
    }

    fn valid_choices() -> &'static str {
        "'compose', 'k8s', 'helm', 'all'"
    }

    fn default_target() -> Self {
        CompileTarget::Compose
    }
}

pub(crate) fn resolve_target_with_ctx<T: ResolvableTarget>(
    ctx: &WorkspaceContext,
    target: Option<T>,
) -> Result<T> {
    if let Some(t) = target {
        return Ok(t);
    }
    let dep_target = ctx
        .config()
        .get_prop(ved_core::r::keys::DEPLOYMENT_TARGET, "");
    if !dep_target.is_empty() {
        let val = dep_target.trim();
        return T::parse_target(val).ok_or_else(|| {
            CliError::invalid_target(
                ved_core::r::keys::DEPLOYMENT_TARGET,
                val,
                T::valid_choices(),
            )
        });
    }
    let tgt = ctx.config().get_prop(ved_core::r::keys::TARGET, "");
    if !tgt.is_empty() {
        let val = tgt.trim();
        return T::parse_target(val).ok_or_else(|| {
            CliError::invalid_target(ved_core::r::keys::TARGET, val, T::valid_choices())
        });
    }
    Ok(T::default_target())
}

pub(crate) fn resolve_target_in<T: ResolvableTarget>(root: &Path, target: Option<T>) -> Result<T> {
    resolve_target_with_ctx(&WorkspaceContext::new(root.to_path_buf())?, target)
}

pub(crate) fn resolve_deploy_target_in(
    root: &Path,
    target: Option<DeployTarget>,
) -> Result<DeployTarget> {
    resolve_target_in(root, target)
}

#[allow(dead_code)]
pub(crate) fn resolve_deploy_target(target: Option<DeployTarget>) -> Result<DeployTarget> {
    resolve_deploy_target_in(Path::new("."), target)
}

pub(crate) fn resolve_compile_target_in(
    root: &Path,
    target: Option<CompileTarget>,
) -> Result<CompileTarget> {
    resolve_target_in(root, target)
}

#[allow(dead_code)]
pub(crate) fn resolve_compile_target(target: Option<CompileTarget>) -> Result<CompileTarget> {
    resolve_compile_target_in(Path::new("."), target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolvable_target_deploy() {
        assert_eq!(
            DeployTarget::parse_target("compose"),
            Some(DeployTarget::Compose)
        );
        assert_eq!(
            DeployTarget::parse_target("k8s"),
            Some(DeployTarget::Kubernetes)
        );
        assert_eq!(DeployTarget::parse_target("helm"), None);
        assert_eq!(DeployTarget::valid_choices(), "'compose', 'k8s'");
        assert_eq!(DeployTarget::default_target(), DeployTarget::Compose);
    }

    #[test]
    fn test_resolvable_target_compile() {
        assert_eq!(
            CompileTarget::parse_target("compose"),
            Some(CompileTarget::Compose)
        );
        assert_eq!(
            CompileTarget::parse_target("k8s"),
            Some(CompileTarget::Kubernetes)
        );
        assert_eq!(
            CompileTarget::parse_target("helm"),
            Some(CompileTarget::Helm)
        );
        assert_eq!(CompileTarget::parse_target("all"), Some(CompileTarget::All));
        assert_eq!(CompileTarget::parse_target("invalid"), None);
        assert_eq!(
            CompileTarget::valid_choices(),
            "'compose', 'k8s', 'helm', 'all'"
        );
        assert_eq!(CompileTarget::default_target(), CompileTarget::Compose);
    }

    #[test]
    fn test_resolve_target_explicit_arg() {
        let res = resolve_deploy_target_in(Path::new("."), Some(DeployTarget::Kubernetes)).unwrap();
        assert_eq!(res, DeployTarget::Kubernetes);

        let res_compile =
            resolve_compile_target_in(Path::new("."), Some(CompileTarget::Helm)).unwrap();
        assert_eq!(res_compile, CompileTarget::Helm);
    }
}
