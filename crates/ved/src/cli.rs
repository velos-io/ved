use clap::{Args, Parser, Subcommand, ValueEnum};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeployTarget {
    Compose,
    #[value(name = "k8s")]
    Kubernetes,
}

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CompileTarget {
    Compose,
    #[value(name = "k8s")]
    Kubernetes,
    Helm,
    All,
}

impl DeployTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compose => "compose",
            Self::Kubernetes => "k8s",
        }
    }
}

impl CompileTarget {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compose => "compose",
            Self::Kubernetes => "k8s",
            Self::Helm => "helm",
            Self::All => "all",
        }
    }
}

#[derive(Debug)]
pub(crate) struct SlotError(String);
impl std::fmt::Display for SlotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for SlotError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Slot(pub String);

impl FromStr for Slot {
    type Err = SlotError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(SlotError("Slot cannot be empty".to_string()));
        }
        for c in s.chars() {
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return Err(SlotError(format!(
                    "Invalid character '{c}' in slot. Must be lowercase alphanumeric or dash"
                )));
            }
        }
        if s.starts_with('-') || s.ends_with('-') {
            return Err(SlotError(
                "Slot cannot start or end with a dash".to_string(),
            ));
        }
        Ok(Slot(s.to_string()))
    }
}

impl Slot {
    #[cfg(test)]
    pub(crate) fn resolve_in(root: &Path, slot: Option<Slot>) -> Self {
        slot.unwrap_or_else(|| {
            let locator = ved_core::ConfigLocator::legacy(root);
            let snapshot = ved_core::ConfigSnapshot::with_environment(
                &locator,
                std::collections::BTreeMap::new(),
            );
            let val = snapshot.get_prop("K8S_APP_SLOT", "1");
            Slot(val)
        })
    }

    pub(crate) fn agent_namespace(&self) -> String {
        format!("ns-{}", self.0)
    }

    #[cfg(test)]
    pub(crate) fn resolver_in(&self, root: &Path) -> ved_core::NamespaceResolver {
        ved_core::NamespaceResolver::from_root(root, &self.0)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Parser)]
#[command(name = "ved")]
#[command(
    about = "High-performance deployment compiler and orchestrator for Ved specifications",
    after_help = "User Guide: https://velos-io.github.io/veddocs\nRepository: https://github.com/velos-io/veddocs",
    version = concat!(env!("CARGO_PKG_VERSION"), " (built ", env!("BUILD_TIMESTAMP"), ")")
)]
pub(crate) struct Cli {
    /// Working directory for command execution
    #[arg(short = 'd', long = "dir", global = true, value_name = "DIR")]
    pub(crate) dir: Option<PathBuf>,

    /// Configuration directory [default: {root}/config]
    #[arg(
        long = "config-dir",
        global = true,
        env = "VED_CONFIG_DIR",
        value_name = "DIR"
    )]
    pub(crate) config_dir: Option<PathBuf>,

    /// Mutable runtime state directory [default: {root}/.ved]
    #[arg(
        long = "work-dir",
        global = true,
        env = "VED_WORK_DIR",
        value_name = "DIR"
    )]
    pub(crate) work_dir: Option<PathBuf>,

    /// Unified artifact directory [default: legacy layout]
    #[arg(
        short = 'o',
        long = "output-dir",
        global = true,
        env = "VED_OUTPUT_DIR",
        value_name = "DIR"
    )]
    pub(crate) output_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompileArgs {
    /// Path to the specification file (e.g. Vedfile or ved.yaml)
    #[arg(short = 'f', long)]
    pub(crate) file: Option<PathBuf>,

    /// Deployment target: compose, k8s, helm, or all
    #[arg(short = 't', long)]
    pub(crate) target: Option<CompileTarget>,

    /// Target organization profile
    #[arg(long, default_value = "local")]
    pub(crate) org: String,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct UpArgs {
    /// Path to the specification file (e.g. Vedfile or ved.yaml)
    #[arg(short = 'f', long)]
    pub(crate) file: Option<PathBuf>,

    /// Deployment target: compose or k8s
    #[arg(short = 't', long)]
    pub(crate) target: Option<DeployTarget>,

    /// Kubernetes slot index (default: K8S_APP_SLOT or 1)
    #[arg(short = 's', long)]
    pub(crate) slot: Option<Slot>,

    /// Run toolchain builds for detected modules before starting stack
    #[arg(long)]
    pub(crate) build: bool,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownArgs {
    /// Deployment target: compose or k8s
    #[arg(short = 't', long)]
    pub(crate) target: Option<DeployTarget>,

    /// Kubernetes slot index (default: K8S_APP_SLOT or 1)
    #[arg(short = 's', long)]
    pub(crate) slot: Option<Slot>,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PsArgs {
    /// Path to the specification file (e.g. Vedfile or ved.yaml)
    #[arg(short = 'f', long)]
    pub(crate) file: Option<PathBuf>,

    /// Deployment target: compose or k8s
    #[arg(short = 't', long)]
    pub(crate) target: Option<DeployTarget>,

    /// Kubernetes slot index (default: K8S_APP_SLOT or 1)
    #[arg(short = 's', long)]
    pub(crate) slot: Option<Slot>,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResetArgs {
    /// Deployment target: compose or k8s
    #[arg(short = 't', long)]
    pub(crate) target: Option<DeployTarget>,

    /// Kubernetes slot index (default: K8S_APP_SLOT or 1)
    #[arg(short = 's', long)]
    pub(crate) slot: Option<Slot>,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestArgs {
    /// Target test workload (default: smoke-test or first job)
    #[arg(value_name = "WORKLOAD")]
    pub(crate) workload: Option<String>,

    /// Deployment target: compose or k8s
    #[arg(short = 't', long)]
    pub(crate) target: Option<DeployTarget>,

    /// Kubernetes slot index (default: K8S_APP_SLOT or 1)
    #[arg(short = 's', long)]
    pub(crate) slot: Option<Slot>,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidateArgs {
    /// Path to the specification file
    #[arg(short = 'f', long)]
    pub(crate) file: Option<PathBuf>,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct InitArgs {
    /// Convert existing docker-compose into Vedfile
    #[arg(long)]
    pub(crate) compose: bool,

    /// Specific docker-compose file to convert
    #[arg(long = "from-compose")]
    pub(crate) from_compose: Option<PathBuf>,

    /// Generate project structure based on Dockerfile occurrence
    #[arg(long)]
    pub(crate) project: bool,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeinitArgs {
    /// Target project directory
    #[arg(default_value = ".")]
    pub(crate) dir: PathBuf,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub(crate) force: bool,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigArgs {
    #[command(subcommand)]
    pub(crate) action: ConfigAction,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyncArgs {
    /// Target project directory
    #[arg(default_value = ".")]
    pub(crate) dir: PathBuf,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuildArgs {
    /// Specific module to build (builds all detected modules if omitted)
    #[arg(short = 'm', long)]
    pub(crate) module: Option<String>,

    /// Maximum concurrent module builds (defaults to CPU core count)
    #[arg(short = 'j', long)]
    pub(crate) concurrency: Option<usize>,
}

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Commands {
    /// Compile a Ved specification into target deployment manifests
    Compile(CompileArgs),
    /// Start stack services
    Up(UpArgs),
    /// Stop running stack services
    Down(DownArgs),
    /// Display status and endpoint URLs of running services
    Ps(PsArgs),
    /// Reset environment and remove containers, volumes, or namespaces
    Reset(ResetArgs),
    /// Run test workload against active stack
    #[command(alias = "smoke-test")]
    Test(TestArgs),
    /// Validate a Ved specification against schema
    Validate(ValidateArgs),
    /// Initialize a project with the ved wrapper (vew) and configuration
    Init(InitArgs),
    /// Remove Ved files and configuration from project
    Deinit(DeinitArgs),
    /// Manage configuration files
    Config(ConfigArgs),
    /// Rescan workspace and refresh cached module state (.ved/state.json)
    Sync(SyncArgs),
    /// Build project modules using detected toolchains
    Build(BuildArgs),
    /// Display compiler version
    Version,
}

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfigAction {
    /// Initialize configuration environment files (.env and .local.env)
    Init {
        /// Force overwrite of existing files
        #[arg(short, long)]
        force: bool,
    },
}

pub(crate) fn parse() -> Cli {
    Cli::parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_parsing_and_validation() {
        assert!(Slot::from_str("1").is_ok());
        assert!(Slot::from_str("blue-2").is_ok());
        assert!(Slot::from_str("dev-prod-9").is_ok());

        assert!(Slot::from_str("").is_err());
        assert!(Slot::from_str("-blue").is_err());
        assert!(Slot::from_str("blue-").is_err());
        assert!(Slot::from_str("Blue").is_err());
        assert!(Slot::from_str("slot_1").is_err());
        assert!(Slot::from_str("slot@1").is_err());
    }

    #[test]
    fn test_slot_namespace_generation() {
        let slot = Slot("prod-1".to_string());
        assert_eq!(slot.agent_namespace(), "ns-prod-1");
        assert_eq!(slot.as_str(), "prod-1");
        assert_eq!(format!("{slot}"), "prod-1");

        let tmp = std::env::temp_dir().join(format!("slot_test_ns_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let resolver = slot.resolver_in(&tmp);
        assert_eq!(resolver.resolve("infra"), "infra");
        assert_eq!(resolver.resolve("tools"), "tools");
        assert_eq!(resolver.resolve("agent"), "ns-prod-1");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_slot_resolve_in() {
        let tmp = std::env::temp_dir().join(format!("slot_test_res_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);

        let explicit = Slot::resolve_in(&tmp, Some(Slot("5".to_string())));
        assert_eq!(explicit.as_str(), "5");

        let fallback = Slot::resolve_in(&tmp, None);
        assert_eq!(fallback.as_str(), "1");

        let config_dir = tmp.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join(".env"), "K8S_APP_SLOT=staging\n").unwrap();

        let from_env = Slot::resolve_in(&tmp, None);
        assert_eq!(from_env.as_str(), "staging");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cli_dispatch_parsing_and_subcommands() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["ved", "test"]).expect("parse test default");
        match cli.command {
            Commands::Test(args) => {
                assert_eq!(args.workload, None);
                assert_eq!(args.target, None);
                assert_eq!(args.slot, None);
            }
            _ => panic!("expected Commands::Test"),
        }

        let cli =
            Cli::try_parse_from(["ved", "test", "integration-job", "-t", "compose", "-s", "3"])
                .expect("parse custom test");
        match cli.command {
            Commands::Test(args) => {
                assert_eq!(args.workload.as_deref(), Some("integration-job"));
                assert_eq!(args.target, Some(DeployTarget::Compose));
                assert_eq!(args.slot, Some(Slot("3".to_string())));
            }
            _ => panic!("expected Commands::Test"),
        }

        let cli = Cli::try_parse_from(["ved", "smoke-test"]).expect("parse alias");
        match cli.command {
            Commands::Test(args) => {
                assert_eq!(args.workload, None);
            }
            _ => panic!("expected Commands::Test"),
        }

        let cli = Cli::try_parse_from(["ved", "up", "-t", "k8s", "-s", "2", "--build"])
            .expect("parse up");
        match cli.command {
            Commands::Up(args) => {
                assert_eq!(args.target, Some(DeployTarget::Kubernetes));
                assert_eq!(args.slot, Some(Slot("2".to_string())));
                assert!(args.build);
            }
            _ => panic!("expected Commands::Up"),
        }

        let cli =
            Cli::try_parse_from(["ved", "build", "-m", "web", "-j", "8"]).expect("parse build");
        match cli.command {
            Commands::Build(args) => {
                assert_eq!(args.module.as_deref(), Some("web"));
                assert_eq!(args.concurrency, Some(8));
            }
            _ => panic!("expected Commands::Build"),
        }

        let cli = Cli::try_parse_from([
            "ved",
            "compile",
            "-f",
            "my-spec.yaml",
            "-t",
            "all",
            "-o",
            "dist",
            "--org",
            "velos",
        ])
        .expect("parse compile");
        match cli.command {
            Commands::Compile(args) => {
                assert_eq!(args.file, Some(PathBuf::from("my-spec.yaml")));
                assert_eq!(args.target, Some(CompileTarget::All));
                assert_eq!(cli.output_dir, Some(PathBuf::from("dist")));
                assert_eq!(args.org, "velos");
            }
            _ => panic!("expected Commands::Compile"),
        }
    }

    #[test]
    fn test_cli_global_directory_flags_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from([
            "ved",
            "--config-dir",
            "custom-cfg",
            "--work-dir",
            "custom-work",
            "-o",
            "dist",
            "version",
        ])
        .expect("parse global flags");

        assert_eq!(cli.config_dir, Some(PathBuf::from("custom-cfg")));
        assert_eq!(cli.work_dir, Some(PathBuf::from("custom-work")));
        assert_eq!(cli.output_dir, Some(PathBuf::from("dist")));
        assert_eq!(cli.command, Commands::Version);

        let cli_after = Cli::try_parse_from([
            "ved",
            "compile",
            "--output-dir",
            "out-dir",
            "--config-dir",
            "conf",
            "--work-dir",
            "work",
        ])
        .expect("parse global flags after subcommand");

        assert_eq!(cli_after.output_dir, Some(PathBuf::from("out-dir")));
        assert_eq!(cli_after.config_dir, Some(PathBuf::from("conf")));
        assert_eq!(cli_after.work_dir, Some(PathBuf::from("work")));
    }
}
