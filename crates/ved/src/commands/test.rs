use crate::error::Result;
#[cfg(test)]
use std::path::Path;
use ved_core::{AppSpec, KubernetesRoutingPlan};

use crate::cli::{DeployTarget, Slot, TestArgs};
use crate::context::WorkspaceContext;
use crate::orchestrator::Deployer;
use crate::output::Outputter;
#[cfg(test)]
use crate::output::TerminalOutputter;
use crate::spec::resolve_target_with_ctx;

pub(crate) fn handle_test_with_output(
    ctx: &WorkspaceContext,
    args: TestArgs,
    deployer: &dyn Deployer,
    output: &dyn Outputter,
) -> Result<()> {
    let tgt = resolve_target_with_ctx(ctx, args.target)?;
    let slot = ctx.resolve_slot(args.slot);
    let (spec, plan) = if tgt == DeployTarget::Kubernetes {
        let (spec, plan) =
            crate::assets::ensure_k8s_assets_with_plan_and_output(ctx, None, &slot, output)?;
        (spec, Some(plan))
    } else {
        (
            crate::assets::ensure_assets_with_output(ctx, None, tgt.clone(), output)?,
            None,
        )
    };

    let target_workload = args.workload.unwrap_or_else(|| {
        if spec.jobs.contains_key("smoke-test") {
            "smoke-test".to_string()
        } else if let Some(first_job) = spec.jobs.keys().next() {
            first_job.clone()
        } else {
            "smoke-test".to_string()
        }
    });

    match tgt {
        DeployTarget::Kubernetes => run_test_k8s_with_plan_and_output(
            deployer,
            ctx,
            &target_workload,
            plan.as_ref(),
            &slot,
            &spec,
            output,
        ),
        DeployTarget::Compose => {
            run_test_compose_with_output(deployer, ctx, &target_workload, output)
        }
    }
}

#[cfg(test)]
pub(crate) fn run_test_k8s_with(
    deployer: &impl Deployer,
    root: &Path,
    target_workload: &str,
    slot: &Slot,
) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_test_k8s_with_ctx(deployer, &ctx, target_workload, slot)
}

fn run_test_k8s_with_plan_and_output(
    deployer: &(impl Deployer + ?Sized),
    ctx: &WorkspaceContext,
    target_workload: &str,
    plan: Option<&KubernetesRoutingPlan>,
    slot: &Slot,
    spec: &AppSpec,
    output: &dyn Outputter,
) -> Result<()> {
    let ns_agent = spec
        .jobs
        .get(target_workload)
        .map(|job| {
            plan.map(|p| p.namespace_for_job(job, target_workload))
                .unwrap_or_else(|| slot.agent_namespace())
        })
        .unwrap_or_else(|| slot.agent_namespace());
    output.println(&format!(
        "test: checking {target_workload} Job in {ns_agent}"
    ));
    deployer.test_k8s(ctx, target_workload, &ns_agent, 60)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn run_test_k8s_with_ctx(
    deployer: &(impl Deployer + ?Sized),
    ctx: &WorkspaceContext,
    target_workload: &str,
    slot: &Slot,
) -> Result<()> {
    run_test_k8s_with_plan(
        deployer,
        ctx,
        target_workload,
        None,
        slot,
        &AppSpec::default(),
    )
}

#[cfg(test)]
pub(crate) fn run_test_k8s_with_plan(
    deployer: &(impl Deployer + ?Sized),
    ctx: &WorkspaceContext,
    target_workload: &str,
    plan: Option<&KubernetesRoutingPlan>,
    slot: &Slot,
    spec: &AppSpec,
) -> Result<()> {
    run_test_k8s_with_plan_and_output(
        deployer,
        ctx,
        target_workload,
        plan,
        slot,
        spec,
        &TerminalOutputter,
    )
}

#[cfg(test)]
pub(crate) fn run_test_compose_with(
    deployer: &impl Deployer,
    root: &Path,
    target_workload: &str,
) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_test_compose_with_ctx(deployer, &ctx, target_workload)
}

#[cfg(test)]
pub(crate) fn run_test_compose_with_ctx(
    deployer: &(impl Deployer + ?Sized),
    ctx: &WorkspaceContext,
    target_workload: &str,
) -> Result<()> {
    run_test_compose_with_output(deployer, ctx, target_workload, &TerminalOutputter)
}

fn run_test_compose_with_output(
    deployer: &(impl Deployer + ?Sized),
    ctx: &WorkspaceContext,
    target_workload: &str,
    output: &dyn Outputter,
) -> Result<()> {
    output.println(&format!("test: running {target_workload} container"));
    deployer.test_compose(ctx, target_workload)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{DeployAction, MockDeployer};

    #[test]
    fn test_test_k8s_mock() {
        let mock = MockDeployer::new();
        let slot = Slot("5".to_string());
        run_test_k8s_with(&mock, Path::new("."), "smoke-test", &slot).expect("run_test_k8s_with");

        let actions = mock.recorded_actions();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            DeployAction::TestK8s {
                job_name,
                namespace,
                timeout_secs,
            } => {
                assert_eq!(job_name, "smoke-test");
                assert_eq!(namespace, "ns-5");
                assert_eq!(*timeout_secs, 60);
            }
            other => panic!("Unexpected action: {other:?}"),
        }
    }

    #[test]
    fn test_test_compose_mock() {
        let mock = MockDeployer::new();
        run_test_compose_with(&mock, Path::new("."), "custom-job").expect("run_test_compose_with");

        let actions = mock.recorded_actions();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            DeployAction::TestCompose { workload } => {
                assert_eq!(workload, "custom-job");
            }
            other => panic!("Unexpected action: {other:?}"),
        }
    }
}
