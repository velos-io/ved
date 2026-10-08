//! High-level facade for loading, validating, and compiling Ved deployment specifications.

use crate::error::Result;
use crate::model::AppSpec;
use crate::validated::ValidatedSpec;
use crate::WorkspaceContext;
use std::path::{Path, PathBuf};

/// Legacy host adapter for file loading, configuration capture and artifact persistence.
/// Use [`crate::PureCompiler`] for explicit-input, in-memory compilation.
///
/// # Examples
///
/// ```rust
/// use ved_core::VedCompiler;
/// use std::path::Path;
///
/// let compiler = VedCompiler::new(".");
/// assert_eq!(compiler.root(), std::env::current_dir().unwrap().join("."));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VedCompiler {
    context: WorkspaceContext,
}

impl VedCompiler {
    /// Creates a new compiler instance configured with the specified workspace root directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            context: WorkspaceContext::new(root),
        }
    }

    pub fn with_context(context: WorkspaceContext) -> Self {
        Self { context }
    }
    pub fn context(&self) -> &WorkspaceContext {
        &self.context
    }

    /// Returns a reference to the configured workspace root directory.
    pub fn root(&self) -> &Path {
        self.context.root()
    }

    /// Loads and validates an application specification from the given path (YAML or Starlark).
    ///
    /// # Errors
    /// Returns `VedError` if the file does not exist, has syntax errors, or fails semantic validation.
    pub fn load_spec(&self, path: impl AsRef<Path>) -> Result<AppSpec> {
        let p = path.as_ref();
        let target_path = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root().join(p)
        };
        crate::parser::load_any_spec_with_context(&target_path, &self.context)
    }

    /// Load an immutable, validated specification from YAML, Starlark or shorthand.
    pub fn load_validated_spec(&self, path: impl AsRef<Path>) -> Result<ValidatedSpec> {
        let path = path.as_ref();
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root().join(path)
        };
        crate::parser::load_validated_spec_with_context(&target, &self.context)
    }

    /// Compile a previously validated snapshot without reparsing workload fields.
    pub fn compile_compose_validated(&self, spec: &ValidatedSpec) -> Result<String> {
        crate::compose::generate_compose_validated_with_context(spec, &self.context)
    }

    /// Emit Kubernetes from a previously validated snapshot.
    pub fn compile_k8s_validated(
        &self,
        spec: &ValidatedSpec,
        out_dir: impl AsRef<Path>,
    ) -> Result<()> {
        let plan = crate::k8s::KubernetesRoutingPlan::from_context(
            spec.as_spec(),
            &self.context,
            &self
                .context
                .config()
                .get_prop(crate::r::keys::K8S_APP_SLOT, "1"),
        )?;
        self.compile_k8s_validated_with_plan(spec, out_dir, &plan)
    }

    pub fn compile_k8s_validated_with_plan(
        &self,
        spec: &ValidatedSpec,
        out_dir: impl AsRef<Path>,
        plan: &crate::k8s::KubernetesRoutingPlan,
    ) -> Result<()> {
        let out = out_dir.as_ref();
        let target = if out.is_absolute() {
            out.to_path_buf()
        } else {
            self.root().join(out)
        };
        crate::k8s::generate_k8s_validated_with_plan(spec, &self.context, &target, plan)
    }

    /// Emit Helm values from a previously validated snapshot.
    pub fn compile_helm_validated(&self, spec: &ValidatedSpec) -> Result<String> {
        crate::helm::generate_helm_values_validated_with_context(spec, &self.context)
    }

    /// Generates Docker Compose YAML manifests from the given specification.
    ///
    /// # Errors
    /// Returns `VedError` if manifest generation fails.
    pub fn compile_compose(&self, spec: &AppSpec) -> Result<String> {
        self.compile_compose_validated(&ValidatedSpec::try_from(spec)?)
    }

    /// Synthesizes complete Kubernetes base and overlay manifests into the target directory.
    ///
    /// # Errors
    /// Returns `VedError` if manifest synthesis or I/O fails.
    pub fn compile_k8s(&self, spec: &AppSpec, out_dir: impl AsRef<Path>) -> Result<()> {
        let slot = self
            .context
            .config()
            .get_prop(crate::r::keys::K8S_APP_SLOT, "1");
        let plan = crate::k8s::KubernetesRoutingPlan::from_context(spec, &self.context, &slot)?;
        self.compile_k8s_with_plan(spec, out_dir, &plan)
    }

    pub fn compile_k8s_with_plan(
        &self,
        spec: &AppSpec,
        out_dir: impl AsRef<Path>,
        plan: &crate::k8s::KubernetesRoutingPlan,
    ) -> Result<()> {
        let out = out_dir.as_ref();
        let target_dir = if out.is_absolute() {
            out.to_path_buf()
        } else {
            self.root().join(out)
        };
        self.compile_k8s_validated_with_plan(&ValidatedSpec::try_from(spec)?, &target_dir, plan)
    }

    /// Synthesizes Helm `values.yaml` content from the given specification.
    ///
    /// # Errors
    /// Returns `VedError` if value serialization fails.
    pub fn compile_helm(&self, spec: &AppSpec) -> Result<String> {
        self.compile_helm_validated(&ValidatedSpec::try_from(spec)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compiler_initialization() {
        let compiler = VedCompiler::new("/tmp/test-project");
        assert_eq!(compiler.root(), Path::new("/tmp/test-project"));
    }

    #[test]
    fn test_compiler_compose_roundtrip() {
        let compiler = VedCompiler::new(".");
        let spec = AppSpec {
            app: "test-app".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };
        let compose = compiler.compile_compose(&spec).expect("compile compose");
        assert!(compose.contains("name: test-app"));
    }

    #[test]
    fn test_compiler_helm_roundtrip() {
        let compiler = VedCompiler::new(".");
        let spec = AppSpec {
            app: "helm-app".to_string(),
            version: "2.0.0".to_string(),
            ..Default::default()
        };
        let helm = compiler.compile_helm(&spec).expect("compile helm");
        assert!(helm.contains("global:"));
        assert!(helm.contains("appVersion: 2.0.0") || helm.contains("appVersion: '2.0.0'"));
    }
}
