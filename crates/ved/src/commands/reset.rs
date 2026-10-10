use crate::error::Result;
use std::path::Path;

use crate::cli::{DeployTarget, ResetArgs, Slot};
use crate::context::WorkspaceContext;
use crate::orchestrator::{Deployer, HostDeployer};
use crate::output::Outputter;
use crate::spec::resolve_target_with_ctx;

#[allow(dead_code)]
pub(crate) fn handle_reset(ctx: &WorkspaceContext, args: ResetArgs) -> Result<()> {
    handle_reset_with_output(ctx, args, &HostDeployer, &crate::output::TerminalOutputter)
}

#[allow(dead_code)]
pub(crate) fn handle_reset_with(
    ctx: &WorkspaceContext,
    args: ResetArgs,
    deployer: &impl Deployer,
) -> Result<()> {
    handle_reset_with_output(ctx, args, deployer, &crate::output::TerminalOutputter)
}

pub(crate) fn handle_reset_with_output(
    ctx: &WorkspaceContext,
    args: ResetArgs,
    deployer: &impl Deployer,
    output: &dyn Outputter,
) -> Result<()> {
    let tgt = resolve_target_with_ctx(ctx, args.target)?;
    let slot = ctx.resolve_slot(args.slot);

    match tgt {
        DeployTarget::Kubernetes => run_reset_k8s_with_output(ctx, &slot, deployer, output),
        DeployTarget::Compose => run_reset_compose_with_output(ctx, deployer, output),
    }
}

#[allow(dead_code)]
pub(crate) fn run_reset_k8s(root: &Path, slot: &Slot) -> Result<()> {
    run_reset_k8s_with(root, slot, &HostDeployer)
}

pub(crate) fn run_reset_k8s_with(root: &Path, slot: &Slot, deployer: &impl Deployer) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_reset_k8s_with_ctx(&ctx, slot, deployer)
}

pub(crate) fn run_reset_k8s_with_ctx(
    ctx: &WorkspaceContext,
    slot: &Slot,
    deployer: &impl Deployer,
) -> Result<()> {
    run_reset_k8s_with_output(ctx, slot, deployer, &crate::output::TerminalOutputter)
}

pub(crate) fn run_reset_k8s_with_output(
    ctx: &WorkspaceContext,
    slot: &Slot,
    deployer: &impl Deployer,
    output: &dyn Outputter,
) -> Result<()> {
    let resolver = ctx.namespace_resolver(slot);
    let namespaces = resolver.known_namespaces();

    output.println(&format!("reset: deleting k8s namespaces for slot {slot}"));
    deployer.reset_k8s(ctx, &namespaces)?;
    output.println("ok: namespaces deleted");
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn run_reset_compose(root: &Path) -> Result<()> {
    run_reset_compose_with(root, &HostDeployer)
}

pub(crate) fn run_reset_compose_with(root: &Path, deployer: &impl Deployer) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_reset_compose_with_ctx(&ctx, deployer)
}

pub(crate) fn run_reset_compose_with_ctx(
    ctx: &WorkspaceContext,
    deployer: &impl Deployer,
) -> Result<()> {
    run_reset_compose_with_output(ctx, deployer, &crate::output::TerminalOutputter)
}

pub(crate) fn run_reset_compose_with_output(
    ctx: &WorkspaceContext,
    deployer: &impl Deployer,
    output: &dyn Outputter,
) -> Result<()> {
    output.println("reset: removing all compose resources (volumes + images)");
    deployer.reset_compose(ctx)?;
    output.println("ok: compose resources removed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{DeployAction, MockDeployer};
    use std::path::PathBuf;

    #[test]
    fn test_reset_k8s_mock() {
        let ctx = WorkspaceContext::new(PathBuf::from(".")).unwrap();
        let mock = MockDeployer::new();
        let args = ResetArgs {
            target: Some(DeployTarget::Kubernetes),
            slot: Some(Slot("99".to_string())),
        };

        handle_reset_with(&ctx, args, &mock).expect("reset k8s");
        let actions = mock.recorded_actions();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            DeployAction::ResetK8s { namespaces } => {
                assert_eq!(namespaces.len(), 3);
                assert_eq!(namespaces[0], "ns-99");
                assert_eq!(namespaces[1], "infra");
                assert_eq!(namespaces[2], "tools");
            }
            other => panic!("Unexpected action: {other:?}"),
        }
    }

    #[test]
    fn test_reset_compose_mock() {
        let ctx = WorkspaceContext::new(PathBuf::from(".")).unwrap();
        let mock = MockDeployer::new();
        let args = ResetArgs {
            target: Some(DeployTarget::Compose),
            slot: None,
        };

        handle_reset_with(&ctx, args, &mock).expect("reset compose");
        let actions = mock.recorded_actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], DeployAction::ResetCompose);
    }
}
