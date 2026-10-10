use crate::error::Result;
use std::path::Path;

use ved_core::{AppSpec, KubernetesRoutingPlan};

use crate::cli::{DeployTarget, Slot, UpArgs};
use crate::context::WorkspaceContext;
use crate::orchestrator::{Deployer, HostDeployer};
use crate::output::{CoreOutputAdapter, Outputter, TerminalOutputter};
use crate::spec::resolve_target_with_ctx;

pub(crate) fn handle_up_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    args: UpArgs,
    output: &dyn Outputter,
) -> Result<()> {
    if args.build
        || ctx
            .config()
            .get_prop(ved_core::r::keys::VED_AUTO_BUILD, "false")
            == "true"
    {
        let core_output = CoreOutputAdapter(output);
        let build_options = ved_core::build::BuildOptions::new()
            .concurrency(ved_core::build::default_concurrency().max(1))?
            .cancellation_token(ctx.cancellation_token());
        ved_core::build::build_workspace(ctx.core(), ctx.root(), &build_options, &core_output)?;
    }
    let tgt = resolve_target_with_ctx(ctx, args.target)?;
    let slot = ctx.resolve_slot(args.slot);
    let (spec, plan) = if tgt == DeployTarget::Kubernetes {
        let (spec, plan) =
            crate::assets::ensure_k8s_assets_with_plan_and_output(ctx, args.file, &slot, output)?;
        (spec, Some(plan))
    } else {
        (
            crate::assets::ensure_assets_with_output(ctx, args.file, tgt.clone(), output)?,
            None,
        )
    };

    match tgt {
        DeployTarget::Kubernetes => run_up_k8s_with_plan_and_output(
            deployer,
            ctx,
            &spec,
            plan.as_ref().expect("k8s plan"),
            output,
        ),
        DeployTarget::Compose => run_up_compose_with_output(deployer, ctx, &spec, output),
    }
}

pub(crate) fn build_namespace_manifests(namespaces: &[&str]) -> String {
    let mut docs = Vec::with_capacity(namespaces.len());
    for ns in namespaces {
        docs.push(format!(
            "apiVersion: v1
kind: Namespace
metadata:
  name: {ns}"
        ));
    }
    docs.join("\n---\n")
}

#[allow(dead_code)]
pub(crate) fn run_up_k8s(root: &Path, spec: &AppSpec, slot: &Slot) -> Result<()> {
    run_up_k8s_with(&HostDeployer, root, spec, slot)
}

pub(crate) fn run_up_k8s_with_ctx(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    slot: &Slot,
) -> Result<()> {
    let plan = KubernetesRoutingPlan::build(spec, ctx.namespace_resolver(slot))?;
    run_up_k8s_with_plan(deployer, ctx, spec, &plan)
}

pub(crate) fn run_up_k8s_with_plan(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    plan: &KubernetesRoutingPlan,
) -> Result<()> {
    run_up_k8s_with_plan_and_output(deployer, ctx, spec, plan, &TerminalOutputter)
}

fn run_up_k8s_with_plan_and_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    plan: &KubernetesRoutingPlan,
    output: &dyn Outputter,
) -> Result<()> {
    let namespaces = plan.namespaces();

    output.println(&format!(
        "up: applying kustomize overlays ({})",
        namespaces.join(" -> ")
    ));

    let ns_refs: Vec<&str> = namespaces.iter().map(|s| s.as_str()).collect();
    let manifest_stream = build_namespace_manifests(&ns_refs);
    deployer.apply_manifest_stream(ctx, &manifest_stream)?;

    for (job_name, job) in &spec.jobs {
        if job.schedule.is_some() {
            continue;
        }
        let target_ns = plan.namespace_for_job(job, job_name);
        deployer.delete_job(ctx, job_name, &target_ns)?;
    }

    for ns in &namespaces {
        let layout = ctx.artifact_layout(&ctx.config().get_prop(
            ved_core::r::keys::COMPOSE_PATH,
            crate::process::default_compose_path(),
        ));
        let overlay_path = layout
            .k8s_dir
            .join("overlays/local")
            .join(ns)
            .display()
            .to_string();
        deployer.apply_kustomize(ctx, &overlay_path)?;
    }

    for (name, svc) in &spec.services {
        if !svc.is_enabled(name) {
            continue;
        }
        let target_ns = plan.namespace_for_service(svc, name);
        output.println(&format!("up: waiting for {name} rollout in {target_ns}"));
        deployer.wait_rollout(ctx, name, &target_ns, 120)?;
    }
    Ok(())
}

pub(crate) fn run_up_k8s_with(
    deployer: &dyn Deployer,
    root: &Path,
    spec: &AppSpec,
    slot: &Slot,
) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_up_k8s_with_ctx(deployer, &ctx, spec, slot)
}

#[allow(dead_code)]
pub(crate) fn run_up_compose(root: &Path, spec: &AppSpec) -> Result<()> {
    run_up_compose_with(&HostDeployer, root, spec)
}

pub(crate) fn run_up_compose_with(
    deployer: &dyn Deployer,
    root: &Path,
    spec: &AppSpec,
) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_up_compose_with_ctx(deployer, &ctx, spec)
}

pub(crate) fn run_up_compose_with_ctx(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec: &AppSpec,
) -> Result<()> {
    run_up_compose_with_output(deployer, ctx, spec, &TerminalOutputter)
}

fn run_up_compose_with_output(
    deployer: &dyn Deployer,
    ctx: &WorkspaceContext,
    spec: &AppSpec,
    output: &dyn Outputter,
) -> Result<()> {
    let mut profile_set = std::collections::HashSet::new();
    profile_set.insert("core".to_string());
    if ctx
        .config()
        .get_prop(ved_core::r::keys::OTEL_ENABLED, "false")
        == "true"
    {
        profile_set.insert("otel".to_string());
    }
    for stack in &spec.stacks {
        profile_set.insert(stack.clone());
    }
    let mut profiles: Vec<String> = profile_set.into_iter().collect();
    profiles.sort();
    let profiles_str = profiles.join(",");
    output.println(&format!(
        "up: starting compose with profiles={profiles_str}"
    ));
    deployer.compose_up(ctx, &profiles)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{DeployAction, MockDeployer};
    use indexmap::IndexMap;
    use ved_core::ServiceSpec;

    #[test]
    fn test_build_namespace_manifests() {
        let namespaces = vec!["infra", "tools", "agent"];
        let manifests = build_namespace_manifests(&namespaces);

        assert!(manifests.contains("apiVersion: v1\nkind: Namespace\nmetadata:\n  name: infra"));
        assert!(manifests.contains("apiVersion: v1\nkind: Namespace\nmetadata:\n  name: tools"));
        assert!(manifests.contains("apiVersion: v1\nkind: Namespace\nmetadata:\n  name: agent"));
        assert_eq!(manifests.matches("kind: Namespace").count(), 3);
        assert_eq!(manifests.matches("---").count(), 2);
    }

    #[test]
    fn test_build_namespace_manifests_deduplication() {
        let ns1 = "shared-ns".to_string();
        let ns2 = "shared-ns".to_string();
        let ns3 = "agent-ns".to_string();

        let mut namespaces: Vec<&str> = vec![&ns1, &ns2, &ns3];
        let mut seen = std::collections::HashSet::new();
        namespaces.retain(|ns| seen.insert(*ns));

        assert_eq!(namespaces, vec!["shared-ns", "agent-ns"]);
        let manifests = build_namespace_manifests(&namespaces);
        assert_eq!(manifests.matches("kind: Namespace").count(), 2);
    }

    #[test]
    fn test_run_up_k8s_with_mock_deployer() {
        let mock = MockDeployer::new();
        let mut services = IndexMap::new();
        services.insert(
            "web".to_string(),
            ServiceSpec {
                image: Some("nginx:alpine".to_string()),
                group: "agent".to_string(),
                ..Default::default()
            },
        );
        let spec = AppSpec {
            services,
            ..Default::default()
        };
        let slot = Slot("1".to_string());
        let root = Path::new(".");

        run_up_k8s_with(&mock, root, &spec, &slot).expect("run_up_k8s_with");

        let actions = mock.recorded_actions();
        assert!(actions
            .iter()
            .any(|a| matches!(a, DeployAction::ApplyManifestStream(_))));
        assert!(actions
            .iter()
            .any(|a| matches!(a, DeployAction::ApplyKustomize(p) if p.contains("infra"))));
        assert!(actions
            .iter()
            .any(|a| matches!(a, DeployAction::ApplyKustomize(p) if p.contains("tools"))));
        assert!(actions
            .iter()
            .any(|a| matches!(a, DeployAction::ApplyKustomize(p) if p.contains("ns-1"))));
        assert!(actions.iter().any(
            |a| matches!(a, DeployAction::WaitRollout { deployment, .. } if deployment == "web")
        ));
    }

    #[test]
    fn test_run_up_compose_with_mock_deployer() {
        let mock = MockDeployer::new();
        let spec = AppSpec {
            stacks: vec!["otel".to_string()],
            ..Default::default()
        };
        let root = Path::new(".");

        run_up_compose_with(&mock, root, &spec).expect("run_up_compose_with");

        let actions = mock.recorded_actions();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            DeployAction::ComposeUp { profiles } => {
                assert!(profiles.contains(&"core".to_string()));
                assert!(profiles.contains(&"otel".to_string()));
            }
            _ => panic!("Expected ComposeUp action"),
        }
    }

    #[test]
    fn test_run_up_k8s_with_custom_group_routes_to_dedicated_namespace() {
        let mock = MockDeployer::new();
        let mut services = IndexMap::new();
        services.insert(
            "custom-db".to_string(),
            ServiceSpec {
                image: Some("postgres:16".to_string()),
                group: "database".to_string(),
                ..Default::default()
            },
        );
        let spec = AppSpec {
            services,
            ..Default::default()
        };
        let slot = Slot("1".to_string());
        let root = Path::new(".");

        run_up_k8s_with(&mock, root, &spec, &slot).expect("run_up_k8s_with");

        let actions = mock.recorded_actions();
        assert!(actions
            .iter()
            .any(|a| matches!(a, DeployAction::ApplyKustomize(p) if p.contains("database"))));
        assert!(actions.iter().any(|a| matches!(a, DeployAction::WaitRollout { deployment, namespace, .. } if deployment == "custom-db" && namespace == "database")));
    }
}
