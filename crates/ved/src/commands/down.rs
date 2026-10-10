use crate::error::Result;
use std::path::Path;

use crate::cli::{DeployTarget, DownArgs, Slot};
use crate::context::WorkspaceContext;
use crate::orchestrator::{Deployer, HostDeployer};
use crate::output::Outputter;
use crate::spec::resolve_target_with_ctx;

#[allow(dead_code)]
pub(crate) fn handle_down(ctx: &WorkspaceContext, args: DownArgs) -> Result<()> {
    handle_down_with_output(ctx, args, &HostDeployer, &crate::output::TerminalOutputter)
}

#[allow(dead_code)]
pub(crate) fn handle_down_with(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    args: DownArgs,
) -> Result<()> {
    handle_down_with_output(ctx, args, deployer, &crate::output::TerminalOutputter)
}

pub(crate) fn handle_down_with_output(
    ctx: &WorkspaceContext,
    args: DownArgs,
    deployer: &dyn Deployer,
    output: &dyn Outputter,
) -> Result<()> {
    let tgt = resolve_target_with_ctx(ctx, args.target)?;
    let slot = ctx.resolve_slot(args.slot);

    match tgt {
        DeployTarget::Kubernetes => run_down_k8s_with_output(deployer, ctx, &slot, output),
        DeployTarget::Compose => run_down_compose_with_output(deployer, ctx, output),
    }
}

#[allow(dead_code)]
pub(crate) fn run_down_k8s(root: &Path, slot: &Slot) -> Result<()> {
    run_down_k8s_with(&HostDeployer, root, slot)
}

pub(crate) fn run_down_k8s_with(deployer: &dyn Deployer, root: &Path, slot: &Slot) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_down_k8s_with_ctx(deployer, &ctx, slot)
}

pub(crate) fn run_down_k8s_with_ctx(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    slot: &Slot,
) -> Result<()> {
    run_down_k8s_with_output(deployer, ctx, slot, &crate::output::TerminalOutputter)
}

pub(crate) fn run_down_k8s_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    slot: &Slot,
    output: &dyn Outputter,
) -> Result<()> {
    let resolver = ctx.namespace_resolver(slot);
    let namespaces = resolver.known_namespaces();

    output.println(&format!(
        "down: scaling all deployments to zero in {}",
        namespaces.join(", ")
    ));
    let mut completed = Vec::new();
    let mut failures = Vec::new();
    for ns in &namespaces {
        match deployer.scale_deployments(ctx, ns, 0) {
            Ok(()) => completed.push(format!("scale deployments in namespace {ns}")),
            Err(err) => {
                failures.push(err.with_context("scale deployments", format!("namespace {ns}")))
            }
        }
    }

    for ns in &namespaces {
        if ns != &resolver.resolve("tools") {
            match deployer.delete_jobs_all(ctx, ns) {
                Ok(()) => completed.push(format!("delete jobs in namespace {ns}")),
                Err(err) => {
                    failures.push(err.with_context("delete jobs", format!("namespace {ns}")))
                }
            }
        }
    }

    if !failures.is_empty() {
        return Err(crate::error::CliError::aggregate(completed, failures));
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn run_down_compose(root: &Path) -> Result<()> {
    run_down_compose_with(&HostDeployer, root)
}

pub(crate) fn run_down_compose_with(deployer: &dyn Deployer, root: &Path) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_down_compose_with_ctx(deployer, &ctx)
}

pub(crate) fn run_down_compose_with_ctx(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
) -> Result<()> {
    run_down_compose_with_output(deployer, ctx, &crate::output::TerminalOutputter)
}

pub(crate) fn run_down_compose_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    output: &dyn Outputter,
) -> Result<()> {
    output.println("down: stopping all compose services");
    deployer.compose_down(ctx)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{DeployAction, MockDeployer};

    #[test]
    fn test_run_down_k8s_with_mock_deployer() {
        let mock = MockDeployer::new();
        let slot = Slot("1".to_string());
        let root = Path::new(".");

        run_down_k8s_with(&mock, root, &slot).expect("run_down_k8s_with");

        let actions = mock.recorded_actions();
        assert_eq!(actions.len(), 5);
        assert!(actions.contains(&DeployAction::ScaleDeployments {
            namespace: "ns-1".to_string(),
            replicas: 0,
        }));
        assert!(actions.contains(&DeployAction::ScaleDeployments {
            namespace: "infra".to_string(),
            replicas: 0,
        }));
        assert!(actions.contains(&DeployAction::ScaleDeployments {
            namespace: "tools".to_string(),
            replicas: 0,
        }));
        assert!(actions.contains(&DeployAction::DeleteJobsAll {
            namespace: "ns-1".to_string(),
        }));
        assert!(actions.contains(&DeployAction::DeleteJobsAll {
            namespace: "infra".to_string(),
        }));
    }

    #[test]
    fn test_run_down_compose_with_mock_deployer() {
        let mock = MockDeployer::new();
        let root = Path::new(".");

        run_down_compose_with(&mock, root).expect("run_down_compose_with");

        let actions = mock.recorded_actions();
        assert_eq!(actions, vec![DeployAction::ComposeDown]);
    }
}
