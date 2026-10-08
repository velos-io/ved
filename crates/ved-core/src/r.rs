//! Namespaced resource constants module (`r`).
//!
//! Centralizes canonical filenames, CLI binaries, directory names,
//! and configuration keys across the workspace.

pub mod files {
    pub const VEDFILE: &str = "Vedfile";
    pub const VED_YAML: &str = "ved.yaml";
    pub const VED_VERSION: &str = ".ved-version";
    pub const COMPOSE_FILE: &str = "docker-compose.yml";
    pub const VALUES_FILE: &str = "values.yaml";
    pub const KUSTOMIZATION_FILE: &str = "kustomization.yaml";
    pub const ENV_FILE: &str = ".env";
    pub const LOCAL_ENV_FILE: &str = ".local.env";
    pub const GENERATED_ENV_FILE: &str = ".generated.env";
}

pub mod bins {
    pub const KUBECTL: &str = "kubectl";
    pub const DOCKER: &str = "docker";
    pub const GIT: &str = "git";
    pub const VEW: &str = "vew";
}

pub mod dirs {
    pub const VED_DIR: &str = ".ved";
    pub const CONFIG_DIR: &str = "config";
    pub const INSTALLER_DIR: &str = "installer";
    pub const K8S_DIR: &str = "k8s";
    pub const STACKS_DIR: &str = "stacks";
}

pub mod keys {
    pub const INGRESS_SUFFIX: &str = "INGRESS_SUFFIX";
    pub const DEPLOYMENT_TARGET: &str = "DEPLOYMENT_TARGET";
    pub const VED_AUTO_BUILD: &str = "VED_AUTO_BUILD";
    pub const K8S_APP_SLOT: &str = "K8S_APP_SLOT";
    pub const K8S_INFRA_NAMESPACE: &str = "K8S_INFRA_NAMESPACE";
    pub const K8S_TOOLS_NAMESPACE: &str = "K8S_TOOLS_NAMESPACE";
    pub const COMPOSE_PATH: &str = "COMPOSE_PATH";
    pub const VED_STACKS_REPO: &str = "VED_STACKS_REPO";
    pub const VED_STACKS_PATH: &str = "VED_STACKS_PATH";
    pub const VED_K8S_NAMESPACE: &str = "VED_K8S_NAMESPACE";
    pub const TARGET: &str = "TARGET";
    pub const OTEL_ENABLED: &str = "OTEL_ENABLED";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files_constants_stability() {
        assert_eq!(files::VEDFILE, "Vedfile");
        assert_eq!(files::VED_YAML, "ved.yaml");
        assert_eq!(files::VED_VERSION, ".ved-version");
        assert_eq!(files::COMPOSE_FILE, "docker-compose.yml");
        assert_eq!(files::VALUES_FILE, "values.yaml");
        assert_eq!(files::KUSTOMIZATION_FILE, "kustomization.yaml");
        assert_eq!(files::ENV_FILE, ".env");
        assert_eq!(files::LOCAL_ENV_FILE, ".local.env");
        assert_eq!(files::GENERATED_ENV_FILE, ".generated.env");
    }

    #[test]
    fn test_bins_constants_stability() {
        assert_eq!(bins::KUBECTL, "kubectl");
        assert_eq!(bins::DOCKER, "docker");
        assert_eq!(bins::GIT, "git");
        assert_eq!(bins::VEW, "vew");
    }

    #[test]
    fn test_dirs_constants_stability() {
        assert_eq!(dirs::VED_DIR, ".ved");
        assert_eq!(dirs::CONFIG_DIR, "config");
        assert_eq!(dirs::INSTALLER_DIR, "installer");
        assert_eq!(dirs::K8S_DIR, "k8s");
        assert_eq!(dirs::STACKS_DIR, "stacks");
    }

    #[test]
    fn test_keys_constants_stability() {
        assert_eq!(keys::INGRESS_SUFFIX, "INGRESS_SUFFIX");
        assert_eq!(keys::DEPLOYMENT_TARGET, "DEPLOYMENT_TARGET");
        assert_eq!(keys::VED_AUTO_BUILD, "VED_AUTO_BUILD");
        assert_eq!(keys::K8S_APP_SLOT, "K8S_APP_SLOT");
        assert_eq!(keys::K8S_INFRA_NAMESPACE, "K8S_INFRA_NAMESPACE");
        assert_eq!(keys::K8S_TOOLS_NAMESPACE, "K8S_TOOLS_NAMESPACE");
        assert_eq!(keys::COMPOSE_PATH, "COMPOSE_PATH");
        assert_eq!(keys::VED_STACKS_REPO, "VED_STACKS_REPO");
        assert_eq!(keys::VED_STACKS_PATH, "VED_STACKS_PATH");
        assert_eq!(keys::VED_K8S_NAMESPACE, "VED_K8S_NAMESPACE");
        assert_eq!(keys::TARGET, "TARGET");
        assert_eq!(keys::OTEL_ENABLED, "OTEL_ENABLED");
    }
}
