use crate::context::WorkspaceContext;
use crate::error::{CliError, Result};
use crate::output::Outputter;
use crate::process::{command_with_ctx, compose_cmd_with_ctx};
pub trait Deployer {
    fn apply_manifest_stream(&self, ctx: &WorkspaceContext, stream: &str) -> Result<()>;
    fn apply_kustomize(&self, ctx: &WorkspaceContext, overlay_rel_path: &str) -> Result<()>;
    fn delete_job(&self, ctx: &WorkspaceContext, job_name: &str, namespace: &str) -> Result<()>;
    fn wait_rollout(
        &self,
        ctx: &WorkspaceContext,
        deployment: &str,
        namespace: &str,
        timeout_secs: u32,
    ) -> Result<()>;
    fn scale_deployments(
        &self,
        ctx: &WorkspaceContext,
        namespace: &str,
        replicas: u32,
    ) -> Result<()>;
    fn delete_jobs_all(&self, ctx: &WorkspaceContext, namespace: &str) -> Result<()>;
    fn compose_up(&self, ctx: &WorkspaceContext, profiles: &[String]) -> Result<()>;
    fn compose_down(&self, ctx: &WorkspaceContext) -> Result<()>;
    fn compose_ps(&self, ctx: &WorkspaceContext) -> Result<()>;
    /// Legacy terminal-backed pod listing. New call paths use the output-aware method.
    /// Lists pods and routes application-managed output through the supplied sink.
    fn k8s_get_pods_with_output(
        &self,
        ctx: &WorkspaceContext,
        namespace: &str,
        output: &dyn Outputter,
    ) -> Result<()>;
    fn reset_k8s(&self, ctx: &WorkspaceContext, namespaces: &[String]) -> Result<()>;
    fn reset_compose(&self, ctx: &WorkspaceContext) -> Result<()>;
    fn test_k8s(
        &self,
        ctx: &WorkspaceContext,
        job_name: &str,
        namespace: &str,
        timeout_secs: u32,
    ) -> Result<()>;
    fn test_compose(&self, ctx: &WorkspaceContext, workload: &str) -> Result<()>;
}

#[derive(Default, Clone, Copy)]
pub struct HostDeployer;

impl Deployer for HostDeployer {
    fn apply_manifest_stream(&self, ctx: &WorkspaceContext, stream: &str) -> Result<()> {
        command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args(["apply", "-f", "-"])
            .stdin(stream)
            .exec_visible()
            .map_err(|e| CliError::from(e.with_context("apply manifest", "stdin")))?;
        Ok(())
    }

    fn apply_kustomize(&self, ctx: &WorkspaceContext, overlay_rel_path: &str) -> Result<()> {
        command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args(["apply", "-k", overlay_rel_path])
            .exec_visible()
            .map_err(|e| CliError::from(e.with_context("apply kustomize", overlay_rel_path)))?;
        Ok(())
    }

    fn delete_job(&self, ctx: &WorkspaceContext, job_name: &str, namespace: &str) -> Result<()> {
        command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args([
                "delete",
                "job",
                job_name,
                "-n",
                namespace,
                "--ignore-not-found=true",
            ])
            .exec_visible()
            .map_err(|e| {
                CliError::from(
                    e.with_context("delete job", format!("{job_name} in namespace {namespace}")),
                )
            })?;
        Ok(())
    }

    fn wait_rollout(
        &self,
        ctx: &WorkspaceContext,
        deployment: &str,
        namespace: &str,
        timeout_secs: u32,
    ) -> Result<()> {
        let timeout_arg = format!("--timeout={timeout_secs}s");
        command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args([
                "rollout",
                "status",
                &format!("deployment/{deployment}"),
                "-n",
                namespace,
                &timeout_arg,
            ])
            .exec_visible()
            .map_err(|e| {
                CliError::from(e.with_context(
                    "wait for rollout",
                    format!("deployment {deployment} in namespace {namespace}"),
                ))
            })?;
        Ok(())
    }

    fn scale_deployments(
        &self,
        ctx: &WorkspaceContext,
        namespace: &str,
        replicas: u32,
    ) -> Result<()> {
        let replicas_arg = format!("--replicas={replicas}");
        let res = command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args([
                "scale",
                "deployment",
                "--all",
                "-n",
                namespace,
                &replicas_arg,
            ])
            .exec_visible();
        res.map_err(|e| {
            CliError::from(e.with_context(
                "scale deployments",
                format!("namespace {namespace} to {replicas}"),
            ))
        })?;
        Ok(())
    }

    fn delete_jobs_all(&self, ctx: &WorkspaceContext, namespace: &str) -> Result<()> {
        let res = command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args(["delete", "job", "--all", "-n", namespace])
            .exec_visible();
        res.map_err(|e| {
            CliError::from(e.with_context("delete jobs", format!("all in namespace {namespace}")))
        })?;
        Ok(())
    }

    fn compose_up(&self, ctx: &WorkspaceContext, profiles: &[String]) -> Result<()> {
        let profiles_str = profiles.join(",");
        compose_cmd_with_ctx(ctx)
            .env("COMPOSE_PROFILES", profiles_str)
            .args(["up", "-d", "--build", "--no-recreate"])
            .exec_visible()
            .map_err(|e| CliError::from(e.with_context("compose up", "services")))?;
        Ok(())
    }

    fn compose_down(&self, ctx: &WorkspaceContext) -> Result<()> {
        compose_cmd_with_ctx(ctx)
            .env("COMPOSE_PROFILES", "*")
            .args(["down", "--remove-orphans"])
            .exec_visible()
            .map_err(|e| CliError::from(e.with_context("compose down", "services")))?;
        Ok(())
    }

    fn compose_ps(&self, ctx: &WorkspaceContext) -> Result<()> {
        let res = compose_cmd_with_ctx(ctx)
            .env("COMPOSE_PROFILES", "*")
            .arg("ps")
            .exec_visible();
        res.map_err(|e| CliError::from(e.with_context("list compose services", "project")))?;
        Ok(())
    }

    fn k8s_get_pods_with_output(
        &self,
        ctx: &WorkspaceContext,
        namespace: &str,
        output: &dyn Outputter,
    ) -> Result<()> {
        run_k8s_get_pods_with_output(
            command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
                .args(["get", "pods", "-n", namespace]),
            namespace,
            output,
        )
    }

    fn reset_k8s(&self, ctx: &WorkspaceContext, namespaces: &[String]) -> Result<()> {
        run_reset_k8s(namespaces, |ns| {
            command_with_ctx(ctx, ved_core::r::bins::KUBECTL).args([
                "delete",
                "namespace",
                ns,
                "--ignore-not-found",
            ])
        })
    }

    fn reset_compose(&self, ctx: &WorkspaceContext) -> Result<()> {
        let res = compose_cmd_with_ctx(ctx)
            .env("COMPOSE_PROFILES", "*")
            .args(["down", "--remove-orphans", "--volumes", "--rmi", "local"])
            .exec_visible();
        res.map_err(|e| CliError::from(e.with_context("reset compose", "project")))?;
        Ok(())
    }

    fn test_k8s(
        &self,
        ctx: &WorkspaceContext,
        job_name: &str,
        namespace: &str,
        timeout_secs: u32,
    ) -> Result<()> {
        let timeout_arg = format!("--timeout={timeout_secs}s");
        command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args([
                "wait",
                "--for=condition=complete",
                &format!("job/{job_name}"),
                "-n",
                namespace,
                &timeout_arg,
            ])
            .exec_visible()
            .map_err(|e| {
                CliError::from(e.with_context(
                    "wait for job",
                    format!("{job_name} in namespace {namespace}"),
                ))
            })?;
        command_with_ctx(ctx, ved_core::r::bins::KUBECTL)
            .args([
                "logs",
                "-n",
                namespace,
                "-l",
                &format!("job-name={job_name}"),
                "--tail=20",
            ])
            .exec_visible()
            .map_err(|e| {
                CliError::from(e.with_context(
                    "read job logs",
                    format!("{job_name} in namespace {namespace}"),
                ))
            })?;
        Ok(())
    }

    fn test_compose(&self, ctx: &WorkspaceContext, workload: &str) -> Result<()> {
        compose_cmd_with_ctx(ctx)
            .env("COMPOSE_PROFILES", "core")
            .args(["run", "--rm", workload])
            .exec_visible()
            .map_err(|e| CliError::from(e.with_context("run compose workload", workload)))?;
        Ok(())
    }
}

fn run_k8s_get_pods_with_output(
    command: crate::process::CommandBuilder,
    namespace: &str,
    outputter: &dyn Outputter,
) -> Result<()> {
    match command.capture().exec() {
        Ok(output) => {
            if let Some(stdout) = output.stdout {
                outputter.write_stdout(&stdout);
            }
        }
        Err(e) => {
            if e.is_missing_resource(&format!("namespace {namespace}")) {
                outputter.println("  (namespace not found)");
                return Ok(());
            }
            return Err(CliError::from(
                e.with_context("get pods", format!("namespace {namespace}")),
            ));
        }
    }
    Ok(())
}

fn run_reset_k8s(
    namespaces: &[String],
    mut command_for_namespace: impl FnMut(&str) -> crate::process::CommandBuilder,
) -> Result<()> {
    let mut completed = Vec::new();
    let mut failures = Vec::new();
    for ns in namespaces {
        match command_for_namespace(ns).exec_visible() {
            Ok(_) => completed.push(ns.clone()),
            Err(e) => failures.push(CliError::from(e).with_context("delete namespace", ns.clone())),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(CliError::aggregate(completed, failures))
    }
}

#[cfg(test)]
pub use mock::{DeployAction, MockDeployer};

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum DeployAction {
        ApplyManifestStream(String),
        ApplyKustomize(String),
        DeleteJob {
            name: String,
            namespace: String,
        },
        WaitRollout {
            deployment: String,
            namespace: String,
            timeout_secs: u32,
        },
        ScaleDeployments {
            namespace: String,
            replicas: u32,
        },
        DeleteJobsAll {
            namespace: String,
        },
        ComposeUp {
            profiles: Vec<String>,
        },
        ComposeDown,
        ComposePs,
        K8sGetPods {
            namespace: String,
        },
        ResetK8s {
            namespaces: Vec<String>,
        },
        ResetCompose,
        TestK8s {
            job_name: String,
            namespace: String,
            timeout_secs: u32,
        },
        TestCompose {
            workload: String,
        },
    }

    #[derive(Default)]
    pub struct MockDeployer {
        pub actions: RefCell<Vec<DeployAction>>,
    }

    impl MockDeployer {
        pub fn new() -> Self {
            Self {
                actions: RefCell::new(Vec::new()),
            }
        }

        pub fn recorded_actions(&self) -> Vec<DeployAction> {
            self.actions.borrow().clone()
        }
    }

    impl Deployer for MockDeployer {
        fn apply_manifest_stream(&self, _ctx: &WorkspaceContext, stream: &str) -> Result<()> {
            self.actions
                .borrow_mut()
                .push(DeployAction::ApplyManifestStream(stream.to_string()));
            Ok(())
        }

        fn apply_kustomize(&self, _ctx: &WorkspaceContext, overlay_rel_path: &str) -> Result<()> {
            self.actions
                .borrow_mut()
                .push(DeployAction::ApplyKustomize(overlay_rel_path.to_string()));
            Ok(())
        }

        fn delete_job(
            &self,
            _ctx: &WorkspaceContext,
            job_name: &str,
            namespace: &str,
        ) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::DeleteJob {
                name: job_name.to_string(),
                namespace: namespace.to_string(),
            });
            Ok(())
        }

        fn wait_rollout(
            &self,
            _ctx: &WorkspaceContext,
            deployment: &str,
            namespace: &str,
            timeout_secs: u32,
        ) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::WaitRollout {
                deployment: deployment.to_string(),
                namespace: namespace.to_string(),
                timeout_secs,
            });
            Ok(())
        }

        fn scale_deployments(
            &self,
            _ctx: &WorkspaceContext,
            namespace: &str,
            replicas: u32,
        ) -> Result<()> {
            self.actions
                .borrow_mut()
                .push(DeployAction::ScaleDeployments {
                    namespace: namespace.to_string(),
                    replicas,
                });
            Ok(())
        }

        fn delete_jobs_all(&self, _ctx: &WorkspaceContext, namespace: &str) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::DeleteJobsAll {
                namespace: namespace.to_string(),
            });
            Ok(())
        }

        fn compose_up(&self, _ctx: &WorkspaceContext, profiles: &[String]) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::ComposeUp {
                profiles: profiles.to_vec(),
            });
            Ok(())
        }

        fn compose_down(&self, _ctx: &WorkspaceContext) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::ComposeDown);
            Ok(())
        }

        fn compose_ps(&self, _ctx: &WorkspaceContext) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::ComposePs);
            Ok(())
        }

        fn k8s_get_pods_with_output(
            &self,
            _ctx: &WorkspaceContext,
            namespace: &str,
            _output: &dyn Outputter,
        ) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::K8sGetPods {
                namespace: namespace.to_string(),
            });
            Ok(())
        }

        fn reset_k8s(&self, _ctx: &WorkspaceContext, namespaces: &[String]) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::ResetK8s {
                namespaces: namespaces.to_vec(),
            });
            Ok(())
        }

        fn reset_compose(&self, _ctx: &WorkspaceContext) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::ResetCompose);
            Ok(())
        }

        fn test_k8s(
            &self,
            _ctx: &WorkspaceContext,
            job_name: &str,
            namespace: &str,
            timeout_secs: u32,
        ) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::TestK8s {
                job_name: job_name.to_string(),
                namespace: namespace.to_string(),
                timeout_secs,
            });
            Ok(())
        }

        fn test_compose(&self, _ctx: &WorkspaceContext, workload: &str) -> Result<()> {
            self.actions.borrow_mut().push(DeployAction::TestCompose {
                workload: workload.to_string(),
            });
            Ok(())
        }
    }
}

#[cfg(all(test, unix))]
mod host_tests {
    use super::*;
    use crate::error::CliError;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture {
        root: PathBuf,
        executable: PathBuf,
    }

    impl Fixture {
        fn new(script: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "ved-host-deployer-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let bin = root.join("bin");
            fs::create_dir_all(&bin).unwrap();
            let executable = bin.join("kubectl");
            fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            Self { root, executable }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn host_pod_listing_only_accepts_requested_namespace_not_found() {
        let script = r#"case "$KUBECTL_RESPONSE" in
not_found) echo 'Error from server (NotFound): namespaces "gone" not found' >&2; exit 1 ;;
forbidden) echo 'Error from server (Forbidden): forbidden' >&2; exit 1 ;;
esac
echo 'NAME READY STATUS'"#;
        let missing = Fixture::new(script);
        let missing_output = crate::output::BufferOutputter::default();
        run_k8s_get_pods_with_output(
            crate::process::CommandBuilder::new(&missing.executable)
                .env("KUBECTL_RESPONSE", "not_found")
                .args(["get", "pods", "-n", "gone"]),
            "gone",
            &missing_output,
        )
        .unwrap();
        assert_eq!(missing_output.stdout_lines(), ["  (namespace not found)"]);

        let denied = Fixture::new(script);
        let denied_output = crate::output::BufferOutputter::default();
        let error = run_k8s_get_pods_with_output(
            crate::process::CommandBuilder::new(&denied.executable)
                .env("KUBECTL_RESPONSE", "forbidden")
                .args(["get", "pods", "-n", "gone"]),
            "gone",
            &denied_output,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CliError::Process(crate::process::ProcessError::Context { ref operation, ref resource, .. })
                if operation == "get pods" && resource == "namespace gone"
        ));

        let success = Fixture::new(script);
        let success_output = crate::output::BufferOutputter::default();
        run_k8s_get_pods_with_output(
            crate::process::CommandBuilder::new(&success.executable)
                .args(["get", "pods", "-n", "ok"]),
            "ok",
            &success_output,
        )
        .unwrap();
        assert_eq!(success_output.stdout_lines(), ["NAME READY STATUS\n"]);
    }

    #[test]
    fn host_reset_attempts_every_namespace_and_retains_failures() {
        let trace = std::env::temp_dir().join(format!(
            "ved-host-deployer-trace-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("worker")
        ));
        let script = format!(
            "echo \"$*\" >> '{}'\ncase \"$*\" in *ns-b*) echo forbidden >&2; exit 1 ;; esac",
            trace.display()
        );
        let fixture = Fixture::new(&script);
        let error = run_reset_k8s(&["ns-a".into(), "ns-b".into(), "ns-c".into()], |ns| {
            crate::process::CommandBuilder::new(&fixture.executable).args([
                "delete",
                "namespace",
                ns,
                "--ignore-not-found",
            ])
        })
        .unwrap_err();
        let calls = fs::read_to_string(&trace).unwrap();
        let _ = fs::remove_file(&trace);
        assert_eq!(calls.lines().count(), 3);
        assert!(calls.lines().any(|line| line.contains("ns-c")));
        assert!(error.to_string().contains("ns-a"));
        assert!(error.to_string().contains("ns-c"));
        assert!(error.to_string().contains("ns-b"));
        assert!(!error.to_string().contains("forbidden"));
        assert!(matches!(error, CliError::Aggregate { ref failures, .. } if failures.len() == 1));
    }
}
