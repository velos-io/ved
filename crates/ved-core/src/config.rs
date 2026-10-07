#[cfg(feature = "host")]
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_DIR: &str = crate::r::dirs::CONFIG_DIR;
pub const ENV_FILE: &str = "config/.env";
pub const LOCAL_ENV_FILE: &str = "config/.local.env";
pub const GENERATED_ENV_FILE: &str = "config/.generated.env";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocator {
    pub config_dir: PathBuf,
    pub generated_env: PathBuf,
}

impl ConfigLocator {
    pub fn new(config_dir: PathBuf, generated_env: PathBuf) -> Self {
        Self {
            config_dir,
            generated_env,
        }
    }

    pub fn legacy(root: &Path) -> Self {
        Self {
            config_dir: root.join(CONFIG_DIR),
            generated_env: root.join(GENERATED_ENV_FILE),
        }
    }

    pub fn env_file(&self) -> PathBuf {
        self.config_dir.join(crate::r::files::ENV_FILE)
    }

    pub fn local_env_file(&self) -> PathBuf {
        self.config_dir.join(crate::r::files::LOCAL_ENV_FILE)
    }

    pub fn generated_env_file(&self) -> PathBuf {
        self.generated_env.clone()
    }

    #[cfg(feature = "host")]
    pub fn get_prop(&self, key: &str, default: &str) -> String {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        for file in &[
            self.local_env_file(),
            self.env_file(),
            self.generated_env_file(),
        ] {
            if let Ok(content) = fs::read_to_string(file) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('#') || !trimmed.contains('=') {
                        continue;
                    }
                    let mut parts = trimmed.splitn(2, '=');
                    if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                        if k.trim() == key {
                            let clean_v = v.trim().trim_matches('"').trim_matches('\'');
                            if !clean_v.is_empty() {
                                return clean_v.to_string();
                            }
                        }
                    }
                }
            }
        }
        default.to_string()
    }

    #[cfg(feature = "host")]
    pub fn get_prop_opt(&self, key: &str) -> Option<String> {
        let val = self.get_prop(key, "");
        if val.is_empty() {
            None
        } else {
            Some(val)
        }
    }

    #[cfg(feature = "host")]
    pub fn require_prop(&self, key: &str) -> crate::error::Result<String> {
        self.get_prop_opt(key).ok_or_else(|| {
            crate::error::VedError::config(format!(
                "Required configuration property '{key}' is not set"
            ))
        })
    }

    #[cfg(feature = "host")]
    pub fn resolve_ingress_suffix(&self, spec_ingress: Option<&str>) -> String {
        if let Some(ing) = spec_ingress {
            let trimmed = ing.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        self.get_prop(crate::r::keys::INGRESS_SUFFIX, "app.localapp")
    }
}

// ProvidesStaticType emits an unsafe marker impl for this entirely owned type.
// Restrict the macro exception to this module; handwritten code remains denied.
#[allow(unsafe_code)]
mod snapshot_type {
    /// Configuration resolved once for an execution, without later global reads.
    #[derive(Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "host", derive(starlark::values::ProvidesStaticType))]
    pub struct ConfigSnapshot {
        pub(super) values: std::collections::BTreeMap<String, String>,
        pub(super) environment: std::collections::BTreeMap<String, String>,
    }
}
pub use snapshot_type::ConfigSnapshot;
impl std::fmt::Debug for ConfigSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigSnapshot")
            .field("property_count", &self.values.len())
            .field("environment_count", &self.environment.len())
            .finish()
    }
}
impl ConfigSnapshot {
    /// Build configuration exclusively from supplied values, without reading the host.
    /// Property lookup trims nonempty values; interpolation retains their original bytes.
    pub fn from_values(environment: std::collections::BTreeMap<String, String>) -> Self {
        let values = environment
            .iter()
            .filter(|(_, value)| !value.trim().is_empty())
            .map(|(key, value)| (key.clone(), value.trim().to_owned()))
            .collect();
        Self {
            values,
            environment,
        }
    }

    #[cfg(feature = "host")]
    pub fn capture(locator: &ConfigLocator) -> Self {
        Self::with_environment(
            locator,
            std::env::vars_os()
                .filter_map(|(key, value)| {
                    Some((key.into_string().ok()?, value.into_string().ok()?))
                })
                .collect(),
        )
    }
    #[cfg(feature = "host")]
    pub fn with_environment(
        locator: &ConfigLocator,
        environment: std::collections::BTreeMap<String, String>,
    ) -> Self {
        let mut values = std::collections::BTreeMap::new();
        for path in [
            locator.local_env_file(),
            locator.env_file(),
            locator.generated_env_file(),
        ] {
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with('#') {
                        continue;
                    }
                    if let Some((key, value)) = line.split_once('=') {
                        let value = value.trim().trim_matches('"').trim_matches('\'');
                        if !value.is_empty() {
                            values
                                .entry(key.trim().to_owned())
                                .or_insert_with(|| value.to_owned());
                        }
                    }
                }
            }
        }
        for (key, value) in &environment {
            if !value.trim().is_empty() {
                values.insert(key.clone(), value.trim().to_owned());
            }
        }
        Self {
            values,
            environment,
        }
    }
    pub fn get_prop(&self, key: &str, default: &str) -> String {
        self.values
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_owned())
    }
    pub fn get_prop_opt(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }
    pub fn require_prop(&self, key: &str) -> crate::error::Result<String> {
        self.get_prop_opt(key).ok_or_else(|| {
            crate::error::VedError::config(format!(
                "Required configuration property '{key}' is not set"
            ))
        })
    }
    pub fn resolve_ingress_suffix(&self, ingress: Option<&str>) -> String {
        ingress
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.get_prop(crate::r::keys::INGRESS_SUFFIX, "app.localapp"))
    }
    /// Raw captured environment wins for interpolation; configuration supplies a fallback.
    pub fn interpolation_value(&self, key: &str) -> Option<String> {
        self.environment
            .get(key)
            .cloned()
            .or_else(|| self.get_prop_opt(key))
    }
    pub fn environment(&self) -> &std::collections::BTreeMap<String, String> {
        &self.environment
    }
    pub fn effective_environment(&self) -> std::collections::BTreeMap<String, String> {
        let mut environment = self.environment.clone();
        for (key, value) in &self.values {
            if environment.get(key).is_none_or(|raw| raw.trim().is_empty()) {
                environment.insert(key.clone(), value.clone());
            }
        }
        environment
    }
}

pub fn config_dir_in(root: &Path) -> PathBuf {
    ConfigLocator::legacy(root).config_dir
}

pub fn env_file_in(root: &Path) -> PathBuf {
    ConfigLocator::legacy(root).env_file()
}

pub fn local_env_file_in(root: &Path) -> PathBuf {
    ConfigLocator::legacy(root).local_env_file()
}

pub fn generated_env_file_in(root: &Path) -> PathBuf {
    ConfigLocator::legacy(root).generated_env_file()
}

/// Retrieve a configuration property from environment variables or config/.env files relative to root.
#[cfg(feature = "host")]
pub fn get_prop_in(root: &Path, key: &str, default: &str) -> String {
    ConfigLocator::legacy(root).get_prop(key, default)
}

/// Retrieve a configuration property from environment variables or config/.env files.
#[cfg(feature = "host")]
pub fn get_prop(key: &str, default: &str) -> String {
    ConfigLocator::legacy(Path::new(".")).get_prop(key, default)
}

/// Retrieve an optional configuration property from environment variables or config/.env files relative to root.
#[cfg(feature = "host")]
pub fn get_prop_opt_in(root: &Path, key: &str) -> Option<String> {
    ConfigLocator::legacy(root).get_prop_opt(key)
}

/// Retrieve an optional configuration property from environment variables or config/.env files.
#[cfg(feature = "host")]
pub fn get_prop_opt(key: &str) -> Option<String> {
    ConfigLocator::legacy(Path::new(".")).get_prop_opt(key)
}

/// Retrieve a required configuration property or return a typed `VedError::Config` relative to root.
#[cfg(feature = "host")]
pub fn require_prop_in(root: &Path, key: &str) -> crate::error::Result<String> {
    ConfigLocator::legacy(root).require_prop(key)
}

/// Retrieve a required configuration property or return a typed `VedError::Config`.
#[cfg(feature = "host")]
pub fn require_prop(key: &str) -> crate::error::Result<String> {
    ConfigLocator::legacy(Path::new(".")).require_prop(key)
}

/// Resolve the ingress domain suffix relative to root, preferring spec setting then config/env then fallback.
#[cfg(feature = "host")]
pub fn resolve_ingress_suffix_in(root: &Path, spec_ingress: Option<&str>) -> String {
    ConfigLocator::legacy(root).resolve_ingress_suffix(spec_ingress)
}

/// Resolve the ingress domain suffix, preferring spec setting then config/env then fallback.
#[cfg(feature = "host")]
pub fn resolve_ingress_suffix(spec_ingress: Option<&str>) -> String {
    ConfigLocator::legacy(Path::new(".")).resolve_ingress_suffix(spec_ingress)
}

/// Remote git repository URL for the stacks catalog.
#[cfg(feature = "host")]
pub fn stacks_repo_url() -> String {
    get_prop(
        crate::r::keys::VED_STACKS_REPO,
        "https://github.com/velos-io/stacks.git",
    )
}

/// Default template for committed base configuration (config/.env).
pub fn default_config_env() -> &'static str {
    r#"# ─────────────────────────────────────────────────────────────────────────────
# config/.env — committed base configuration
# Developers override values locally in config/.local.env
# ─────────────────────────────────────────────────────────────────────────────

# ── Deployment & Orchestration ───────────────────────────────────────────────
# Target runtime: compose or k8s
DEPLOYMENT_TARGET=compose

# Docker Compose specification path (default: .ved/docker-compose.yml)
# COMPOSE_PATH=.ved/docker-compose.yml

# Automatically trigger local builds before deployment up (true / false)
VED_AUTO_BUILD=false

# ── Ingress & Routing ───────────────────────────────────────────────────────
# Ingress domain suffix for routing (e.g. app1.app.localapp)
INGRESS_SUFFIX=app.localapp

# ── Kubernetes Settings ─────────────────────────────────────────────────────
# Deployment slot index (drives ns-<slot> namespace and app<slot> host prefix)
K8S_APP_SLOT=1

# Infrastructure and observability namespaces
K8S_INFRA_NAMESPACE=infra
K8S_TOOLS_NAMESPACE=tools

# ── Stacks Catalog ──────────────────────────────────────────────────────────
# Remote repository for shared technology stacks
VED_STACKS_REPO=https://github.com/velos-io/stacks.git

# Local stacks directory override (optional, defaults to ~/.ved/stacks)
# VED_STACKS_PATH=

# ── Container Registry ──────────────────────────────────────────────────────
# Internal image registry prefix (e.g. registry.local:5000)
# INTERNAL_REGISTRY=

# Upstream registry mirror
# EXTERNAL_REGISTRY_MIRROR=

# ── Feature Flags ───────────────────────────────────────────────────────────
# OpenTelemetry collector and tracing instrumentation
OTEL_ENABLED=false

# In-memory Redis cache workload
REDIS_ENABLED=true

# ── Toolchain Executables (optional PATH overrides) ─────────────────────────
# NODE_BIN=
# NPM_BIN=
# GRADLE_BIN=
# MAVEN_BIN=
# GO_BIN=
# RUST_BIN=
# PYTHON_BIN=
"#
}

/// Default template for local developer overrides (config/.local.env).
pub fn default_local_env() -> &'static str {
    r#"# ─────────────────────────────────────────────────────────────────────────────
# config/.local.env — local overrides (do not commit to version control)
# Values defined here take precedence over config/.env
# ─────────────────────────────────────────────────────────────────────────────
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "host")]
    fn test_resolve_ingress_suffix_with_spec_ingress() {
        assert_eq!(
            resolve_ingress_suffix(Some("custom.domain")),
            "custom.domain"
        );
        assert_eq!(
            resolve_ingress_suffix(Some("  trimmed.domain  ")),
            "trimmed.domain"
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn test_resolve_ingress_suffix_default() {
        assert_eq!(resolve_ingress_suffix(None), "app.localapp");
        assert_eq!(resolve_ingress_suffix(Some("")), "app.localapp");
    }

    #[test]
    #[cfg(feature = "host")]
    fn test_stacks_repo_url_default() {
        if std::env::var("VED_STACKS_REPO").is_err() {
            assert_eq!(stacks_repo_url(), "https://github.com/velos-io/stacks.git");
        }
    }

    #[test]
    fn test_default_templates_contain_expected_keys() {
        let env = default_config_env();
        assert!(env.contains("DEPLOYMENT_TARGET=compose"));
        assert!(env.contains("INGRESS_SUFFIX=app.localapp"));
        assert!(env.contains("K8S_APP_SLOT=1"));
        assert!(env.contains("K8S_INFRA_NAMESPACE=infra"));
        assert!(env.contains("K8S_TOOLS_NAMESPACE=tools"));
        assert!(env.contains("VED_STACKS_REPO=https://github.com/velos-io/stacks.git"));
        assert!(env.contains("VED_AUTO_BUILD=false"));
        assert!(env.contains("OTEL_ENABLED=false"));
        assert!(env.contains("REDIS_ENABLED=true"));

        let local = default_local_env();
        assert!(local.contains("config/.local.env"));
    }

    #[test]
    #[cfg(feature = "host")]
    fn test_require_prop() {
        let err = require_prop("NON_EXISTENT_KEY_12345").unwrap_err();
        match err {
            crate::error::VedError::Config(msg) => {
                assert!(msg.contains("NON_EXISTENT_KEY_12345"));
            }
            other => panic!("expected VedError::Config, got: {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "host")]
    fn test_config_locator_custom_dirs() {
        let tmp = std::env::temp_dir().join(format!("test_config_locator_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let custom_cfg = tmp.join("my-config");
        let custom_work = tmp.join("my-work");
        fs::create_dir_all(&custom_cfg).unwrap();
        fs::create_dir_all(&custom_work).unwrap();

        let gen_file = custom_work.join(".generated.env");
        fs::write(
            custom_cfg.join(".env"),
            "MY_KEY=from_env\nOVERRIDE_KEY=base\n",
        )
        .unwrap();
        fs::write(custom_cfg.join(".local.env"), "OVERRIDE_KEY=local\n").unwrap();
        fs::write(&gen_file, "GEN_KEY=generated\n").unwrap();

        let locator = ConfigLocator::new(custom_cfg.clone(), gen_file.clone());
        assert_eq!(locator.env_file(), custom_cfg.join(".env"));
        assert_eq!(locator.local_env_file(), custom_cfg.join(".local.env"));
        assert_eq!(locator.generated_env_file(), gen_file);

        assert_eq!(locator.get_prop("MY_KEY", "default"), "from_env");
        assert_eq!(locator.get_prop("OVERRIDE_KEY", "default"), "local");
        assert_eq!(locator.get_prop("GEN_KEY", "default"), "generated");
        assert_eq!(locator.get_prop("MISSING_KEY", "default"), "default");

        let _ = fs::remove_dir_all(&tmp);
    }
}
