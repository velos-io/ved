//! Deployment specification validation and explicit-input compilation.
//!
//! The pure facade returns artifact bytes without accessing the host. YAML, Starlark
//! and shorthand file loading, configuration capture and persistence are provided by
//! the legacy `VedCompiler` adapter. Validation enforces existing specification rules;
//! Kubernetes rendering is not cluster admission validation, and Helm emits values
//! for an external chart rather than a chart.
//!
//! ```
//! use std::{collections::BTreeMap, path::PathBuf};
//! use ved_core::{AppSpec, ConfigSnapshot, WorkspaceContext, ValidatedSpec,
//!                PureCompiler, CompilationTarget};
//! let root = PathBuf::from(if cfg!(windows) { r"C:\explicit\nonexistent-workspace" }
//!                          else { "/explicit/nonexistent-workspace" });
//! let context = WorkspaceContext::from_explicit_paths(
//!     &root, root.join("config"), ConfigSnapshot::from_values(BTreeMap::new()))?;
//! let spec = ValidatedSpec::try_from(AppSpec {
//!     app: "example".into(), version: "1".into(), ..Default::default()
//! })?;
//! let artifacts = PureCompiler::new(context)?.compile(&spec, CompilationTarget::All)?;
//! assert!(artifacts.files().contains_key(std::path::Path::new("docker-compose.yml")));
//! # Ok::<(), ved_core::VedError>(())
//! ```

#[cfg(feature = "host")]
pub mod build;
mod command;
#[cfg(feature = "host")]
pub(crate) mod compiler;
pub(crate) mod compose;
pub(crate) mod config;
#[cfg(feature = "host")]
pub(crate) mod convert;
#[cfg(feature = "host")]
pub(crate) mod discovery;
pub(crate) mod endpoints;
pub(crate) mod error;
#[cfg(feature = "host")]
pub(crate) mod filesystem;
pub(crate) mod helm;
mod identifier;
pub(crate) mod k8s;
pub(crate) mod model;
pub(crate) mod output;
#[cfg(feature = "host")]
pub(crate) mod parser;
pub mod r;
#[cfg(feature = "host")]
pub(crate) mod secrets;
#[cfg(feature = "host")]
pub(crate) mod shorthand;
#[cfg(feature = "host")]
pub(crate) mod stacks;
#[cfg(feature = "host")]
pub(crate) mod starlark;
pub(crate) mod validated;
#[cfg(feature = "host")]
pub(crate) mod vcs;
pub(crate) mod workspace;

#[cfg(feature = "host")]
pub mod resources;

mod pure;

pub use validated::{DependencyCondition, DependencySpec, ValidatedSpec, ValidatedWorkload};

#[cfg(feature = "host")]
pub use compiler::VedCompiler;
pub use error::{
    Result, SpecLoadError, SpecLoadPhase, StarlarkError, ValidationError, ValidationReason,
    VedError,
};
#[cfg(feature = "host")]
pub use k8s::{generate_k8s, generate_secrets_yaml};
pub use k8s::{
    render_k8s_validated_with_plan, resolve_group, KubernetesRoutingPlan, ManifestDocBuilder,
    NamespaceResolver, YamlNode,
};
pub use model::{
    AppSpec, CheckedPortSpec, CommandSpec, HealthProbe, HealthTest, JobSpec, MountKind, MountSpec,
    PortParseError, PortSpec, PortValidationError, ServiceSpec,
};
pub use output::{BufferSink, OutputSink, SilentSink, StdoutSink};
pub use workspace::WorkspaceContext;
#[cfg(feature = "host")]
pub use workspace::WorkspaceResolutionError;

#[cfg(feature = "host")]
pub use resources::{LoadOptions, ResourceBudget, ResourceLimitError, ResourceLimitKind};
#[cfg(feature = "host")]
pub use vcs::{HostGitRunner, VcsDriver, VcsError, VcsOutput};

#[cfg(feature = "host")]
pub use compose::generate_compose;
pub use compose::{
    build_compose_healthcheck, build_compose_job, build_compose_service, build_compose_volumes,
    extract_volume,
};

pub use config::{
    config_dir_in, default_config_env, default_local_env, env_file_in, generated_env_file_in,
    local_env_file_in, ConfigLocator, ConfigSnapshot,
};
#[cfg(feature = "host")]
pub use config::{
    get_prop_in, get_prop_opt_in, require_prop, require_prop_in, resolve_ingress_suffix,
    resolve_ingress_suffix_in, stacks_repo_url,
};

pub use endpoints::{extract_health_path, get_service_endpoints};
#[cfg(feature = "host")]
pub use helm::generate_helm_values;
pub use pure::{ArtifactBundle, CompilationTarget, PureCompiler};

#[cfg(feature = "host")]
pub use convert::{convert_compose_to_ved, scaffold_from_dockerfiles};
#[cfg(feature = "host")]
pub use discovery::{
    generate_scaffold_vedfile, is_ignored_directory, load_workspace_state, save_workspace_state,
    scan_workspace, DetectedModule, ModuleKind, WorkspaceState,
};
#[cfg(feature = "host")]
pub use parser::{
    expand_env_vars, expand_env_vars_with, load_any_spec, load_any_spec_with_context,
    load_any_spec_with_context_and_dependencies,
    load_any_spec_with_context_and_dependencies_and_options, load_spec, load_validated_spec,
    LoadedSpec,
};
#[cfg(feature = "host")]
pub use secrets::looks_like_secret;
#[cfg(feature = "host")]
pub use shorthand::{
    is_only_comments, is_shorthand, load_shorthand_spec, parse_shorthand, shorthand_to_starlark,
};
#[cfg(feature = "host")]
pub use stacks::{
    apply_stacks, apply_stacks_with_context, apply_stacks_with_context_and_options,
    load_stacks_with_context, load_stacks_with_context_and_options, resolve_stacks_dir,
    resolve_stacks_dir_with_context, sync_stacks_catalog, sync_stacks_catalog_with_cancellation,
    sync_stacks_catalog_with_context, sync_stacks_catalog_with_context_and_driver,
    sync_stacks_catalog_with_driver, PassiveVcsDriver,
};
#[cfg(feature = "host")]
pub use starlark::{evaluate_stack_file, load_starlark_spec};
#[cfg(feature = "host")]
pub use vcs::{GitCommandRunner, GitOutput, GitRunnerError};
