use crate::error::Result;
use std::path::Path;

use ved_core::{extract_health_path, get_service_endpoints, AppSpec, KubernetesRoutingPlan};

use crate::cli::{DeployTarget, PsArgs, Slot};
use crate::context::WorkspaceContext;
use crate::orchestrator::{Deployer, HostDeployer};
use crate::output::{Outputter, TerminalOutputter};
use crate::spec::resolve_target_with_ctx;

pub(crate) fn handle_ps_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    args: PsArgs,
    output: &dyn Outputter,
) -> Result<()> {
    let tgt = resolve_target_with_ctx(ctx, args.target)?;
    let slot = ctx.resolve_slot(args.slot);
    let (spec_opt, plan_opt) = if tgt == DeployTarget::Kubernetes {
        if ctx.spec_path(args.file.as_deref()).exists() {
            let (spec, plan) = crate::assets::ensure_k8s_assets_with_plan_and_output(
                ctx, args.file, &slot, output,
            )?;
            (Some(spec), Some(plan))
        } else {
            (None, None)
        }
    } else {
        let spec = if ctx.spec_path(args.file.as_deref()).exists() {
            Some(crate::assets::ensure_assets_with_output(
                ctx,
                args.file,
                tgt.clone(),
                output,
            )?)
        } else {
            None
        };
        (spec, None)
    };

    match tgt {
        DeployTarget::Kubernetes => run_ps_k8s_with_plan_and_output(
            deployer,
            ctx,
            spec_opt.as_ref(),
            plan_opt.as_ref(),
            &slot,
            output,
        ),
        DeployTarget::Compose => {
            run_ps_compose_with_output(deployer, ctx, spec_opt.as_ref(), output)
        }
    }
}

#[allow(dead_code)]
pub(crate) fn run_ps_k8s(root: &Path, spec_opt: Option<&AppSpec>, slot: &Slot) -> Result<()> {
    run_ps_k8s_with(&HostDeployer, root, spec_opt, slot)
}

pub(crate) fn run_ps_k8s_with(
    deployer: &dyn Deployer,
    root: &Path,
    spec_opt: Option<&AppSpec>,
    slot: &Slot,
) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_ps_k8s_with_ctx(deployer, &ctx, spec_opt, slot)
}

pub(crate) fn run_ps_k8s_with_ctx(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec_opt: Option<&AppSpec>,
    slot: &Slot,
) -> Result<()> {
    run_ps_k8s_with_plan(deployer, ctx, spec_opt, None, slot)
}

pub(crate) fn run_ps_k8s_with_plan(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec_opt: Option<&AppSpec>,
    plan: Option<&KubernetesRoutingPlan>,
    slot: &Slot,
) -> Result<()> {
    run_ps_k8s_with_plan_and_output(deployer, ctx, spec_opt, plan, slot, &TerminalOutputter)
}

fn run_ps_k8s_with_plan_and_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec_opt: Option<&AppSpec>,
    plan: Option<&KubernetesRoutingPlan>,
    slot: &Slot,
    output: &dyn Outputter,
) -> Result<()> {
    let suffix = ctx
        .config()
        .resolve_ingress_suffix(spec_opt.and_then(|s| s.ingress.as_deref()));
    let ns_agent = spec_opt
        .and_then(|spec| {
            let job = spec.jobs.get("smoke-test");
            job.map(|j| {
                plan.map(|p| p.namespace_for_job(j, "smoke-test"))
                    .unwrap_or_else(|| slot.agent_namespace())
            })
        })
        .unwrap_or_else(|| slot.agent_namespace());
    output.println(&crate::ui::section(
        &format!("Kubernetes pods ({ns_agent})"),
        26,
    ));

    deployer.k8s_get_pods_with_output(ctx, &ns_agent, output)?;

    output.println(&format!("\n{}", crate::ui::section("Service URLs", 41)));
    if let Some(spec) = spec_opt {
        for (name, svc) in &spec.services {
            let host = svc.ingress.as_deref().unwrap_or(name.as_str());
            let health_path = extract_health_path(svc);

            let url = if host == "app" {
                format!("http://app{}.{}{}", slot.as_str(), suffix, health_path)
            } else {
                format!("http://{host}.{suffix}{health_path}")
            };
            output.println(&format!("  {:<16} {}", format!("{}:", name), url));
        }
    } else {
        output.println("  (no specification found; URLs unavailable)");
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn run_ps_compose(root: &Path, spec_opt: Option<&AppSpec>) -> Result<()> {
    run_ps_compose_with(&HostDeployer, root, spec_opt)
}

pub(crate) fn run_ps_compose_with(
    deployer: &dyn Deployer,
    root: &Path,
    spec_opt: Option<&AppSpec>,
) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_ps_compose_with_ctx(deployer, &ctx, spec_opt)
}

pub(crate) fn run_ps_compose_with_ctx(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec_opt: Option<&AppSpec>,
) -> Result<()> {
    run_ps_compose_with_output(deployer, ctx, spec_opt, &TerminalOutputter)
}

fn run_ps_compose_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec_opt: Option<&AppSpec>,
    output: &dyn Outputter,
) -> Result<()> {
    output.println("── Docker Compose status ────────────────────────────────");
    deployer.compose_ps(ctx)?;
    output.println(&format!("\n{}", crate::ui::section("Service URLs", 41)));
    if let Some(spec) = spec_opt {
        for (name, svc) in &spec.services {
            let endpoints = get_service_endpoints(name, svc);
            for ep in endpoints {
                output.println(&format!("  {:<16} {}", format!("{}:", name), ep));
            }
        }
    } else {
        output.println("  (no specification found; URLs unavailable)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{DeployAction, MockDeployer};

    #[test]
    fn test_run_ps_k8s_with_mock_deployer() {
        let mock = MockDeployer::new();
        let slot = Slot("1".to_string());
        let root = Path::new(".");

        run_ps_k8s_with(&mock, root, None, &slot).expect("run_ps_k8s_with");

        let actions = mock.recorded_actions();
        assert_eq!(
            actions,
            vec![DeployAction::K8sGetPods {
                namespace: "ns-1".to_string(),
            }]
        );
    }

    #[test]
    fn k8s_status_and_service_headers_use_injected_output() {
        let mock = MockDeployer::new();
        let output = crate::output::BufferOutputter::default();
        let ctx = WorkspaceContext::new(Path::new(".").to_path_buf()).unwrap();
        run_ps_k8s_with_plan_and_output(&mock, &ctx, None, None, &Slot("2".into()), &output)
            .expect("run ps with injected output");
        assert_eq!(
            output.stdout_lines(),
            [
                crate::ui::section("Kubernetes pods (ns-2)", 26),
                format!("\n{}", crate::ui::section("Service URLs", 41)),
                "  (no specification found; URLs unavailable)".to_string(),
            ]
        );
    }

    #[test]
    fn test_run_ps_compose_with_mock_deployer() {
        let mock = MockDeployer::new();
        let root = Path::new(".");

        run_ps_compose_with(&mock, root, None).expect("run_ps_compose_with");

        let actions = mock.recorded_actions();
        assert_eq!(actions, vec![DeployAction::ComposePs]);
    }
}
