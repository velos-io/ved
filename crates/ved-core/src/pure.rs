//! Explicit-input compilation with no host effects.
use crate::{Result, ValidatedSpec, WorkspaceContext};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Deployment artifacts to render. Helm produces values for an external chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationTarget {
    Compose,
    Kubernetes,
    Helm,
    All,
}

/// Deterministically ordered relative paths and their rendered bytes.
/// Debug output deliberately excludes artifact contents, which can contain secrets.
#[derive(Clone, PartialEq, Eq)]
pub struct ArtifactBundle {
    files: BTreeMap<PathBuf, Vec<u8>>,
}
impl ArtifactBundle {
    pub fn files(&self) -> &BTreeMap<PathBuf, Vec<u8>> {
        &self.files
    }
    pub fn into_files(self) -> BTreeMap<PathBuf, Vec<u8>> {
        self.files
    }
}
impl std::fmt::Debug for ArtifactBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactBundle")
            .field("paths", &self.files.keys().collect::<Vec<_>>())
            .field("file_count", &self.files.len())
            .finish()
    }
}

/// Compiles validated inputs without filesystem access, environment capture or processes.
/// Use `WorkspaceContext::from_explicit_paths` and `ConfigSnapshot::from_values`
/// to construct the input context without host effects too.
#[derive(Debug, Clone)]
pub struct PureCompiler {
    context: WorkspaceContext,
}
impl PureCompiler {
    pub fn new(context: WorkspaceContext) -> Result<Self> {
        context.validate_absolute_paths()?;
        Ok(Self { context })
    }
    /// Return `docker-compose.yml`, `helm/values.yaml`, and/or `k8s/base/...`
    /// and `k8s/overlays/...`, depending on the requested target.
    pub fn compile(
        &self,
        spec: &ValidatedSpec,
        target: CompilationTarget,
    ) -> Result<ArtifactBundle> {
        let spec = spec.with_context(&self.context);
        let mut files = BTreeMap::new();
        if matches!(target, CompilationTarget::Compose | CompilationTarget::All) {
            files.insert(
                PathBuf::from("docker-compose.yml"),
                crate::compose::generate_compose_validated_with_context(&spec, &self.context)?
                    .into_bytes(),
            );
        }
        if matches!(target, CompilationTarget::Helm | CompilationTarget::All) {
            files.insert(
                PathBuf::from("helm/values.yaml"),
                crate::helm::generate_helm_values_validated_with_context(&spec, &self.context)?
                    .into_bytes(),
            );
        }
        if matches!(
            target,
            CompilationTarget::Kubernetes | CompilationTarget::All
        ) {
            let slot = self
                .context
                .config()
                .get_prop(crate::r::keys::K8S_APP_SLOT, "1");
            let plan = crate::k8s::KubernetesRoutingPlan::from_context(
                spec.as_spec(),
                &self.context,
                &slot,
            )?;
            for (path, bytes) in
                crate::k8s::render_k8s_validated_with_plan(&spec, &self.context, &plan)?
            {
                files.insert(PathBuf::from("k8s").join(path), bytes);
            }
        }
        Ok(ArtifactBundle { files })
    }
}
