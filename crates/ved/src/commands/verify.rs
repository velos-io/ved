use crate::error::Result;
#[cfg(test)]
use std::path::Path;
use ved_core::{AppSpec, KubernetesRoutingPlan};

use crate::cli::{DeployTarget, Slot, VerifyArgs};
use crate::context::WorkspaceContext;
use crate::orchestrator::Deployer;
use crate::output::Outputter;
use crate::spec::resolve_target_with_ctx;

pub(crate) fn handle_verify_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    args: VerifyArgs,
    output: &dyn Outputter,
) -> Result<()> {
    super::up::run_auto_build_if_enabled(ctx, args.build, output)?;
    let tgt = resolve_target_with_ctx(ctx, args.target)?;
    let slot = ctx.resolve_slot(args.slot);

    match tgt {
        DeployTarget::Kubernetes => {
            let (spec, plan) = crate::assets::ensure_k8s_assets_with_plan_and_output(
                ctx, args.file, &slot, output,
            )?;
            run_verify_k8s_with_plan_and_output(
                deployer,
                ctx,
                &spec,
                &plan,
                &slot,
                args.workload.as_deref(),
                output,
            )
        }
        DeployTarget::Compose => {
            let spec = crate::assets::ensure_assets_with_output(
                ctx,
                args.file,
                DeployTarget::Compose,
                output,
            )?;
            run_verify_compose_with_output(deployer, ctx, &spec, args.workload.as_deref(), output)
        }
    }
}

fn select_verify_workload<'a>(
    spec: &'a AppSpec,
    explicit_workload: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(workload) = explicit_workload {
        return Some(workload);
    }
    if spec
        .jobs
        .get("smoke-test")
        .is_some_and(|job| job.is_enabled("smoke-test") && job.schedule.is_none())
    {
        return Some("smoke-test");
    }
    spec.jobs
        .iter()
        .find(|(name, job)| job.is_enabled(name) && job.schedule.is_none())
        .map(|(name, _)| name.as_str())
}

pub(crate) fn run_verify_k8s_with_plan_and_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    plan: &KubernetesRoutingPlan,
    slot: &Slot,
    explicit_workload: Option<&str>,
    output: &dyn Outputter,
) -> Result<()> {
    super::reset::run_reset_k8s_with_plan_and_output(ctx, slot, Some(plan), deployer, output)?;
    super::up::run_up_k8s_with_plan_and_output(deployer, ctx, spec, plan, output)?;
    super::ps::run_ps_k8s_with_plan_and_output(
        deployer,
        ctx,
        Some(spec),
        Some(plan),
        slot,
        output,
    )?;

    if let Some(workload) = select_verify_workload(spec, explicit_workload) {
        super::test::run_test_k8s_with_plan_and_output(
            deployer,
            ctx,
            workload,
            Some(plan),
            slot,
            spec,
            output,
        )?;
    } else {
        output.println("verify: no runnable test jobs defined; skipping test step");
    }

    output.println("ok: verification complete");
    Ok(())
}

pub(crate) fn run_verify_compose_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    explicit_workload: Option<&str>,
    output: &dyn Outputter,
) -> Result<()> {
    super::reset::run_reset_compose_with_output(ctx, deployer, output)?;
    super::up::run_up_compose_with_output(deployer, ctx, spec, output)?;
    super::ps::run_ps_compose_with_output(deployer, ctx, Some(spec), output)?;

    if let Some(workload) = select_verify_workload(spec, explicit_workload) {
        super::test::run_test_compose_with_output(deployer, ctx, workload, output)?;
    } else {
        output.println("verify: no runnable test jobs defined; skipping test step");
    }

    output.println("ok: verification complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{DeployAction, MockDeployer};
    use crate::output::BufferOutputter;
    use indexmap::IndexMap;
    use ved_core::{JobSpec, ServiceSpec};

    #[test]
    fn test_run_verify_k8s_executes_reset_up_ps_and_test_in_order() {
        let mock = MockDeployer::new();
        let output = BufferOutputter::default();
        let ctx = WorkspaceContext::new(Path::new(".").to_path_buf()).expect("workspace context");
        let slot = Slot("1".to_string());

        let mut services = IndexMap::new();
        services.insert(
            "api".to_string(),
            ServiceSpec {
                image: Some("nginx:alpine".to_string()),
                group: "agent".to_string(),
                ..Default::default()
            },
        );
        let mut jobs = IndexMap::new();
        jobs.insert(
            "smoke-test".to_string(),
            JobSpec {
                image: "curlimages/curl:8.6.0".to_string(),
                group: "agent".to_string(),
                command: Some(serde_json::json!(["curl", "-f", "http://api:80"])),
                ..Default::default()
            },
        );
        let spec = AppSpec {
            app: "verify-k8s".to_string(),
            version: "0.1.0".to_string(),
            services,
            jobs,
            ..Default::default()
        };
        let plan = KubernetesRoutingPlan::build(&spec, ctx.namespace_resolver(&slot)).unwrap();

        run_verify_k8s_with_plan_and_output(&mock, &ctx, &spec, &plan, &slot, None, &output)
            .expect("run_verify_k8s");

        let actions = mock.recorded_actions();
        assert!(matches!(&actions[0], DeployAction::ResetK8s { .. }));
        assert!(actions
            .iter()
            .any(|a| matches!(a, DeployAction::ApplyManifestStream(_))));
        assert!(actions.iter().any(
            |a| matches!(a, DeployAction::WaitRollout { deployment, .. } if deployment == "api")
        ));
        assert!(actions.contains(&DeployAction::K8sGetPods {
            namespace: "ns-1".to_string(),
        }));
        assert_eq!(
            actions.last(),
            Some(&DeployAction::TestK8s {
                job_name: "smoke-test".to_string(),
                namespace: "ns-1".to_string(),
                timeout_secs: 60,
            })
        );
        assert!(output
            .stdout_lines()
            .iter()
            .any(|line| line == "ok: verification complete"));
    }

    #[test]
    fn test_run_verify_k8s_skips_test_when_no_runnable_jobs_exist() {
        let mock = MockDeployer::new();
        let output = BufferOutputter::default();
        let ctx = WorkspaceContext::new(Path::new(".").to_path_buf()).expect("workspace context");
        let slot = Slot("1".to_string());

        let mut services = IndexMap::new();
        services.insert(
            "cache".to_string(),
            ServiceSpec {
                image: Some("redis:7-alpine".to_string()),
                group: "agent".to_string(),
                ..Default::default()
            },
        );
        let mut jobs = IndexMap::new();
        jobs.insert(
            "nightly-cron".to_string(),
            JobSpec {
                image: "busybox:1.36".to_string(),
                schedule: Some("0 2 * * *".to_string()),
                ..Default::default()
            },
        );
        let spec = AppSpec {
            app: "verify-no-job".to_string(),
            version: "0.1.0".to_string(),
            services,
            jobs,
            ..Default::default()
        };
        let plan = KubernetesRoutingPlan::build(&spec, ctx.namespace_resolver(&slot)).unwrap();

        run_verify_k8s_with_plan_and_output(&mock, &ctx, &spec, &plan, &slot, None, &output)
            .expect("run_verify_k8s without runnable job");

        let actions = mock.recorded_actions();
        assert!(!actions
            .iter()
            .any(|a| matches!(a, DeployAction::TestK8s { .. })));
        assert!(output
            .stdout_lines()
            .iter()
            .any(|line| line.contains("no runnable test jobs defined")));
    }

    #[test]
    fn test_run_verify_compose_executes_reset_up_ps_and_test_in_order() {
        let mock = MockDeployer::new();
        let output = BufferOutputter::default();
        let ctx = WorkspaceContext::new(Path::new(".").to_path_buf()).expect("workspace context");

        let mut jobs = IndexMap::new();
        jobs.insert(
            "migration".to_string(),
            JobSpec {
                image: "flyway/flyway:10-alpine".to_string(),
                command: Some(serde_json::json!(["migrate"])),
                ..Default::default()
            },
        );
        let spec = AppSpec {
            app: "verify-compose".to_string(),
            version: "0.1.0".to_string(),
            jobs,
            ..Default::default()
        };

        run_verify_compose_with_output(&mock, &ctx, &spec, None, &output)
            .expect("run_verify_compose");

        assert_eq!(
            mock.recorded_actions(),
            vec![
                DeployAction::ResetCompose,
                DeployAction::ComposeUp {
                    profiles: vec!["core".to_string()],
                },
                DeployAction::ComposePs,
                DeployAction::TestCompose {
                    workload: "migration".to_string(),
                },
            ]
        );
    }
}
