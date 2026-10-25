use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::VedError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSpec {
    pub app: String,
    pub version: String,
    #[serde(default = "default_network")]
    pub network: String,
    #[serde(default)]
    pub ingress: Option<String>,
    #[serde(
        default = "default_config_dir",
        rename = "config_dir",
        alias = "configDir"
    )]
    pub config_dir: String,
    #[serde(
        default = "default_env_files",
        rename = "env_files",
        alias = "envFiles"
    )]
    pub env_files: Vec<String>,
    #[serde(default)]
    pub services: IndexMap<String, ServiceSpec>,
    #[serde(default)]
    pub jobs: IndexMap<String, JobSpec>,
    #[serde(default)]
    pub profiles: IndexMap<String, ProfileSpec>,
    #[serde(default)]
    pub tasks: IndexMap<String, String>,
    #[serde(default)]
    pub stacks: Vec<String>,
}

fn default_config_dir() -> String {
    ".".to_string()
}

fn default_env_files() -> Vec<String> {
    vec![
        crate::config::ENV_FILE.to_string(),
        crate::config::LOCAL_ENV_FILE.to_string(),
    ]
}

fn default_network() -> String {
    "default".to_string()
}

impl AppSpec {
    /// Checks specification-derived path components before any host filesystem effects.
    pub(crate) fn validate_identifiers(&self) -> Result<(), VedError> {
        use crate::identifier::validate_identifier;

        let check = |profile: Option<&str>,
                     kind: &str,
                     name: &str,
                     field: &str,
                     value: &str|
         -> Result<(), VedError> {
            validate_identifier(field, value).map_err(|source| {
                crate::error::ValidationError::new(
                    crate::error::ValidationReason::InvalidIdentifier,
                    profile.map(str::to_owned),
                    Some(kind.into()),
                    if field == "name" {
                        None
                    } else {
                        Some(name.into())
                    },
                    field,
                )
                .with_detail("expected a single filesystem component")
                .with_source(source)
                .into()
            })
        };
        for (name, svc) in &self.services {
            check(None, "Service", name, "name", name)?;
            check(
                None,
                "Service",
                name,
                "group",
                &crate::k8s::resolve_group(&svc.group, name),
            )?;
        }
        for (name, job) in &self.jobs {
            check(None, "Job", name, "name", name)?;
            check(
                None,
                "Job",
                name,
                "group",
                &crate::k8s::resolve_group(&job.group, name),
            )?;
        }
        for (profile_name, profile) in &self.profiles {
            for (name, svc) in &profile.services {
                check(Some(profile_name), "Service", name, "name", name)?;
                check(
                    Some(profile_name),
                    "Service",
                    name,
                    "group",
                    &crate::k8s::resolve_group(&svc.group, name),
                )?;
            }
            for (name, job) in &profile.jobs {
                check(Some(profile_name), "Job", name, "name", name)?;
                check(
                    Some(profile_name),
                    "Job",
                    name,
                    "group",
                    &crate::k8s::resolve_group(&job.group, name),
                )?;
            }
        }
        for name in &self.stacks {
            validate_identifier("stack", name)?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), VedError> {
        crate::validated::ValidatedSpec::try_from(self).map(|_| ())
    }
}

impl Default for AppSpec {
    fn default() -> Self {
        Self {
            app: String::new(),
            version: "0.1.0".to_string(),
            network: default_network(),
            ingress: None,
            config_dir: default_config_dir(),
            env_files: default_env_files(),
            services: IndexMap::new(),
            jobs: IndexMap::new(),
            profiles: IndexMap::new(),
            tasks: IndexMap::new(),
            stacks: Vec::new(),
        }
    }
}

impl Default for ServiceSpec {
    fn default() -> Self {
        Self {
            image: None,
            build: None,
            group: default_group(),
            port: None,
            ports: Vec::new(),
            entrypoint: None,
            command: None,
            memory: None,
            ingress: None,
            env: IndexMap::new(),
            env_file: None,
            mounts: Vec::new(),
            security: None,
            health: None,
            healthcheck: None,
            wait_for: Vec::new(),
            restart: None,
            enabled: None,
            container_name: None,
            profiles: Vec::new(),
            deploy: None,
            shm_size: None,
            ulimits: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySpec {
    #[serde(default)]
    pub caps: Vec<String>,
    #[serde(default)]
    pub escalate: Option<bool>,
    #[serde(default)]
    pub user: Option<serde_json::Value>,
    #[serde(rename = "fsGroup", default)]
    pub fs_group: Option<i64>,
    #[serde(rename = "dropAll", default)]
    pub drop_all: Option<bool>,
}

/// Failure to parse a port mapping specification string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortParseError {
    #[error("invalid port '{input}': {source}")]
    InvalidPort {
        input: String,
        source: std::num::ParseIntError,
    },
    #[error("invalid host port '{input}': {source}")]
    InvalidHost {
        input: String,
        source: std::num::ParseIntError,
    },
    #[error("invalid container port '{input}': {source}")]
    InvalidContainer {
        input: String,
        source: std::num::ParseIntError,
    },
    #[error("invalid port specification '{input}'")]
    InvalidSpec { input: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortSpec {
    pub host: u16,
    pub container: u16,
}

impl PortSpec {
    pub fn new(host: u16, container: u16) -> Self {
        Self { host, container }
    }

    pub fn single(port: u16) -> Self {
        Self {
            host: port,
            container: port,
        }
    }

    pub fn host(&self) -> u16 {
        self.host
    }

    pub fn container(&self) -> u16 {
        self.container
    }

    pub fn parse(s: &str) -> Result<Self, PortParseError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(PortParseError::InvalidSpec {
                input: s.to_string(),
            });
        }
        if let Some((host_part, container_part)) = trimmed.split_once(':') {
            if container_part.contains(':') {
                return Err(PortParseError::InvalidSpec {
                    input: s.to_string(),
                });
            }
            let host = host_part
                .parse::<u16>()
                .map_err(|source| PortParseError::InvalidHost {
                    input: host_part.to_string(),
                    source,
                })?;
            let container = container_part.parse::<u16>().map_err(|source| {
                PortParseError::InvalidContainer {
                    input: container_part.to_string(),
                    source,
                }
            })?;
            Ok(Self { host, container })
        } else {
            let port = trimmed
                .parse::<u16>()
                .map_err(|source| PortParseError::InvalidPort {
                    input: trimmed.to_string(),
                    source,
                })?;
            Ok(Self::single(port))
        }
    }
}

impl serde::Serialize for PortSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&format_args!("{}:{}", self.host, self.container))
    }
}

struct PortSpecVisitor;

impl<'de> serde::de::Visitor<'de> for PortSpecVisitor {
    type Value = PortSpec;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "a port number (0..65535) or a port mapping string (e.g. \"8080:80\" or \"8080\")",
        )
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if v > u16::MAX as u64 {
            return Err(E::custom(format!("port {v} exceeds maximum u16")));
        }
        Ok(PortSpec::single(v as u16))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if !(0..=u16::MAX as i64).contains(&v) {
            return Err(E::custom(format!("port {v} out of bounds")));
        }
        Ok(PortSpec::single(v as u16))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        PortSpec::parse(v).map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for PortSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(PortSpecVisitor)
    }
}

impl TryFrom<serde_json::Value> for PortSpec {
    type Error = String;

    fn try_from(val: serde_json::Value) -> Result<Self, Self::Error> {
        match val {
            serde_json::Value::Number(n) => {
                let p = n
                    .as_u64()
                    .ok_or_else(|| "port number out of bounds".to_string())?;
                if p > u16::MAX as u64 {
                    return Err(format!("port {p} exceeds maximum u16"));
                }
                Ok(Self::single(p as u16))
            }
            serde_json::Value::String(s) => Self::parse(&s).map_err(|e| e.to_string()),
            _ => Err("port must be an integer or string".to_string()),
        }
    }
}

impl From<PortSpec> for serde_json::Value {
    fn from(spec: PortSpec) -> Self {
        serde_json::Value::String(format!("{}:{}", spec.host, spec.container))
    }
}

/// A port mapping whose host and container ports are nonzero.
///
/// ```
/// use ved_core::CheckedPortSpec;
/// let port = CheckedPortSpec::new(9000, 8080)?;
/// assert_eq!((port.host(), port.container()), (9000, 8080));
/// assert!(CheckedPortSpec::single(0).is_err());
/// # Ok::<(), ved_core::PortValidationError>(())
/// ```
/// Fields cannot be constructed or mutated directly:
/// ```compile_fail
/// use ved_core::CheckedPortSpec;
/// let port = CheckedPortSpec { host: 0, container: 80 };
/// ```
/// ```compile_fail
/// use ved_core::CheckedPortSpec;
/// let mut port = CheckedPortSpec::single(80).unwrap();
/// port.host = std::num::NonZeroU16::new(81).unwrap();
/// ```
/// A plain zero cannot stand in for a nonzero value:
/// ```compile_fail
/// let port: std::num::NonZeroU16 = 0;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CheckedPortSpec {
    host: std::num::NonZeroU16,
    container: std::num::NonZeroU16,
}

/// Failure to construct a checked port mapping.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortValidationError {
    #[error("{field} port must be between 1 and 65535 (got zero)")]
    Zero { field: &'static str },
    #[error("invalid port input {input}: {message}")]
    InvalidInput { input: String, message: String },
}

impl CheckedPortSpec {
    pub fn new(host: u16, container: u16) -> Result<Self, PortValidationError> {
        Ok(Self {
            host: std::num::NonZeroU16::new(host)
                .ok_or(PortValidationError::Zero { field: "host" })?,
            container: std::num::NonZeroU16::new(container)
                .ok_or(PortValidationError::Zero { field: "container" })?,
        })
    }

    pub fn single(port: u16) -> Result<Self, PortValidationError> {
        Self::new(port, port)
    }

    pub fn host(&self) -> u16 {
        self.host.get()
    }

    pub fn container(&self) -> u16 {
        self.container.get()
    }

    pub fn parse(input: &str) -> Result<Self, PortValidationError> {
        PortSpec::parse(input)
            .map_err(|err| PortValidationError::InvalidInput {
                input: format!("{input:?}"),
                message: err.to_string(),
            })?
            .try_into()
    }
}

impl serde::Serialize for CheckedPortSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&format_args!("{}:{}", self.host(), self.container()))
    }
}

struct CheckedPortSpecVisitor;

impl<'de> serde::de::Visitor<'de> for CheckedPortSpecVisitor {
    type Value = CheckedPortSpec;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "a nonzero port number (1..65535) or a port mapping string (e.g. \"8080:80\" or \"8080\")",
        )
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if v == 0 {
            return Err(E::custom("port must be between 1 and 65535 (got zero)"));
        }
        if v > u16::MAX as u64 {
            return Err(E::custom(format!("port {v} exceeds maximum u16")));
        }
        CheckedPortSpec::new(v as u16, v as u16).map_err(E::custom)
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if v <= 0 || v > u16::MAX as i64 {
            return Err(E::custom(format!("port {v} out of bounds")));
        }
        CheckedPortSpec::new(v as u16, v as u16).map_err(E::custom)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        CheckedPortSpec::parse(v).map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for CheckedPortSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(CheckedPortSpecVisitor)
    }
}

impl std::str::FromStr for CheckedPortSpec {
    type Err = PortValidationError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl TryFrom<PortSpec> for CheckedPortSpec {
    type Error = PortValidationError;

    fn try_from(port: PortSpec) -> Result<Self, Self::Error> {
        Self::new(port.host, port.container)
    }
}

impl From<CheckedPortSpec> for PortSpec {
    fn from(port: CheckedPortSpec) -> Self {
        Self::new(port.host(), port.container())
    }
}

impl TryFrom<serde_json::Value> for CheckedPortSpec {
    type Error = PortValidationError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        PortSpec::try_from(value.clone())
            .map_err(|message| PortValidationError::InvalidInput {
                input: value.to_string(),
                message,
            })?
            .try_into()
    }
}

impl From<CheckedPortSpec> for serde_json::Value {
    fn from(port: CheckedPortSpec) -> Self {
        PortSpec::from(port).into()
    }
}

impl PartialEq<serde_json::Value> for PortSpec {
    fn eq(&self, other: &serde_json::Value) -> bool {
        match other {
            serde_json::Value::Number(n) => {
                n.as_u64() == Some(self.container as u64) && self.host == self.container
            }
            serde_json::Value::String(s) => {
                if let Ok(parsed) = PortSpec::parse(s) {
                    *self == parsed
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

impl PartialEq<PortSpec> for serde_json::Value {
    fn eq(&self, other: &PortSpec) -> bool {
        other.eq(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandSpec {
    String(String),
    List(Vec<String>),
}

impl CommandSpec {
    pub fn to_args(&self) -> Vec<String> {
        match self {
            Self::String(s) => s.split_whitespace().map(|x| x.to_string()).collect(),
            Self::List(l) => l.clone(),
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::List(l) => l.join(" "),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HealthTest {
    Command(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthProbe {
    Http {
        path: String,
        port: u16,
        scheme: String,
    },
    Tcp {
        port: u16,
    },
    Exec {
        command: Vec<String>,
    },
}

impl HealthProbe {
    pub fn parse_cmd(cmd: &str, default_port: u16) -> Self {
        let trimmed = cmd.trim();
        if trimmed.starts_with("tcp:") {
            let p = trimmed
                .trim_start_matches("tcp:")
                .parse::<u16>()
                .unwrap_or(default_port);
            Self::Tcp { port: p }
        } else if trimmed.starts_with('/') {
            Self::Http {
                path: trimmed.to_string(),
                port: default_port,
                scheme: "HTTP".to_string(),
            }
        } else if let Some(rest) = trimmed.strip_prefix("http://") {
            let (port, path) = parse_host_port_path(rest, default_port);
            Self::Http {
                path,
                port,
                scheme: "HTTP".to_string(),
            }
        } else if let Some(rest) = trimmed.strip_prefix("https://") {
            let (port, path) = parse_host_port_path(rest, default_port);
            Self::Http {
                path,
                port,
                scheme: "HTTPS".to_string(),
            }
        } else {
            Self::Exec {
                command: vec!["sh".to_string(), "-c".to_string(), trimmed.to_string()],
            }
        }
    }
}

fn parse_host_port_path(rest: &str, default_port: u16) -> (u16, String) {
    let (host_port, path) = if let Some(idx) = rest.find('/') {
        (&rest[..idx], rest[idx..].to_string())
    } else {
        (rest, "/".to_string())
    };
    let port = if let Some(idx) = host_port.find(':') {
        host_port[idx + 1..].parse::<u16>().unwrap_or(default_port)
    } else {
        default_port
    };
    (port, path)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountKind {
    PersistentVolumeClaim {
        claim_name: String,
        mount_path: String,
        read_only: bool,
    },
    HostPath {
        volume_name: String,
        host_path: String,
        mount_path: String,
        read_only: bool,
    },
    ConfigMap {
        config_map_name: String,
        mount_path: String,
        read_only: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountSpec {
    pub raw: String,
    pub host_or_volume: String,
    pub container_path: String,
    pub read_only: bool,
    pub kind: MountKind,
}

pub fn sanitize_volume_name(path: &str) -> String {
    let clean = path
        .trim_start_matches("./")
        .trim_start_matches('/')
        .replace(['/', '.', '_'], "-");
    let trimmed = clean.trim_matches('-');
    if trimmed.is_empty() {
        "vol-data".to_string()
    } else {
        format!("vol-{trimmed}")
    }
}

pub fn is_valid_dns1123_label(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    let bytes = name.as_bytes();
    if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl MountSpec {
    pub fn parse(s: &str) -> Result<Self, VedError> {
        let trimmed = s.trim();
        let (is_cm, rest) = if let Some(r) = trimmed.strip_prefix("cm:") {
            (true, r)
        } else if let Some(r) = trimmed.strip_prefix("configmap:") {
            (true, r)
        } else {
            (false, trimmed)
        };

        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(VedError::spec_parse(
                "",
                "Mount string must contain 2 or 3 colon-separated segments",
            ));
        }

        let src = parts[0];
        let dst = parts[1];
        let mode_opt = parts.get(2).copied();

        if src.is_empty() {
            return Err(VedError::spec_parse(
                "",
                "Mount source path cannot be empty",
            ));
        }

        if dst.is_empty() {
            return Err(VedError::spec_parse(
                "",
                "Mount destination path cannot be empty",
            ));
        }

        if !dst.starts_with('/') {
            return Err(VedError::spec_parse(
                "",
                "Mount destination must be an absolute path starting with '/'",
            ));
        }

        let read_only = match mode_opt {
            Some("ro") => true,
            Some("rw") | None => false,
            Some(invalid) => {
                return Err(VedError::spec_parse(
                    "",
                    format!("Mount mode must be 'ro' or 'rw', found '{invalid}'"),
                ));
            }
        };

        let kind = if is_cm {
            if !is_valid_dns1123_label(src) {
                return Err(VedError::spec_parse(
                    "",
                    format!("ConfigMap name '{src}' must be a valid DNS-1123 label"),
                ));
            }
            MountKind::ConfigMap {
                config_map_name: src.to_string(),
                mount_path: dst.to_string(),
                read_only,
            }
        } else if src.starts_with("./")
            || src.starts_with("../")
            || src.starts_with('/')
            || src.contains('/')
        {
            let volume_name = sanitize_volume_name(src);
            MountKind::HostPath {
                volume_name,
                host_path: src.to_string(),
                mount_path: dst.to_string(),
                read_only,
            }
        } else {
            if !is_valid_dns1123_label(src) {
                return Err(VedError::spec_parse(
                    "",
                    format!("PVC claim name '{src}' must be a valid DNS-1123 label"),
                ));
            }
            MountKind::PersistentVolumeClaim {
                claim_name: src.to_string(),
                mount_path: dst.to_string(),
                read_only,
            }
        };

        Ok(Self {
            raw: s.to_string(),
            host_or_volume: src.to_string(),
            container_path: dst.to_string(),
            read_only,
            kind,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    No,
    Always,
    OnFailure,
    UnlessStopped,
}

impl RestartPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::OnFailure => "on-failure",
            Self::UnlessStopped => "unless-stopped",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthCheckSpec {
    pub test: Option<serde_json::Value>,
    #[serde(default = "default_health_interval")]
    pub interval: String,
    #[serde(default = "default_health_timeout")]
    pub timeout: String,
    #[serde(default = "default_health_retries")]
    pub retries: u32,
    #[serde(
        rename = "start_period",
        alias = "startPeriod",
        default = "default_health_start_period"
    )]
    pub start_period: String,
}

pub fn default_health_interval() -> String {
    "15s".to_string()
}

pub fn default_health_timeout() -> String {
    "10s".to_string()
}

pub fn default_health_retries() -> u32 {
    5
}

pub fn default_health_start_period() -> String {
    "30s".to_string()
}

impl HealthCheckSpec {
    #[allow(dead_code)]
    pub fn typed_test(&self) -> Option<HealthTest> {
        self.test
            .as_ref()
            .and_then(|t| serde_json::from_value(t.clone()).ok())
    }

    pub fn from_value(val: &serde_json::Value) -> Self {
        if let serde_json::Value::Object(map) = val {
            let interval = map
                .get("interval")
                .and_then(|v| v.as_str())
                .unwrap_or("15s")
                .to_string();
            let timeout = map
                .get("timeout")
                .and_then(|v| v.as_str())
                .unwrap_or("10s")
                .to_string();
            let retries = map
                .get("retries")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or(5);
            let start_period = map
                .get("start_period")
                .or_else(|| map.get("startPeriod"))
                .and_then(|v| v.as_str())
                .unwrap_or("30s")
                .to_string();
            let test = map.get("test").cloned();
            Self {
                test,
                interval,
                timeout,
                retries,
                start_period,
            }
        } else {
            Self {
                test: Some(val.clone()),
                interval: default_health_interval(),
                timeout: default_health_timeout(),
                retries: default_health_retries(),
                start_period: default_health_start_period(),
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default = "default_group")]
    pub group: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub ports: Vec<serde_json::Value>,
    #[serde(default)]
    pub entrypoint: Option<serde_json::Value>,
    #[serde(default)]
    pub command: Option<serde_json::Value>,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub ingress: Option<String>,
    #[serde(default)]
    pub env: IndexMap<String, serde_json::Value>,
    #[serde(default)]
    pub env_file: Option<String>,
    #[serde(default)]
    pub mounts: Vec<String>,
    #[serde(default)]
    pub security: Option<SecuritySpec>,
    #[serde(default)]
    pub health: Option<serde_json::Value>,
    #[serde(default)]
    pub healthcheck: Option<serde_json::Value>,
    #[serde(rename = "waitFor", default)]
    pub wait_for: Vec<String>,
    #[serde(default)]
    pub restart: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, rename = "container_name", alias = "containerName")]
    pub container_name: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub deploy: Option<serde_json::Value>,
    #[serde(rename = "shm_size", alias = "shmSize", default)]
    pub shm_size: Option<serde_json::Value>,
    #[serde(default)]
    pub ulimits: Option<serde_json::Value>,
}

impl ServiceSpec {
    pub fn health_value(&self) -> Option<&serde_json::Value> {
        self.healthcheck.as_ref().or(self.health.as_ref())
    }

    pub fn has_health(&self) -> bool {
        self.health_value().is_some()
    }

    pub fn health_probe(&self, default_port: u16) -> Option<HealthProbe> {
        let health_val = self.health_value()?;
        if let Some(cmd) = health_val.as_str() {
            Some(HealthProbe::parse_cmd(cmd, default_port))
        } else if let Some(arr) = health_val.as_array() {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(HealthProbe::Exec { command: parts })
            }
        } else if let Some(obj) = health_val.as_object() {
            if let Some(cmd) = obj.get("test").and_then(|t| t.as_str()) {
                Some(HealthProbe::parse_cmd(cmd, default_port))
            } else if let Some(arr) = obj.get("test").and_then(|t| t.as_array()) {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if parts.is_empty() {
                    None
                } else {
                    Some(HealthProbe::Exec { command: parts })
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Use explicit activation, then ambient host configuration when `host` is enabled.
    /// Without `host`, unspecified activation defaults to true. Contextual rendering
    /// resolves activation from its supplied snapshot before calling this method.
    pub fn is_enabled(&self, name: &str) -> bool {
        if let Some(explicit) = self.enabled {
            return explicit;
        }
        #[cfg(feature = "host")]
        {
            check_implicit_env_flag(name)
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = name;
            true
        }
    }

    pub fn typed_ports(&self) -> Vec<PortSpec> {
        let mut result = Vec::new();
        for p in &self.ports {
            if let Ok(spec) = PortSpec::try_from(p.clone()) {
                result.push(spec);
            }
        }
        if result.is_empty() {
            if let Some(p) = self.port {
                result.push(PortSpec::single(p));
            }
        }
        result
    }

    pub fn typed_mounts(&self) -> Result<Vec<MountSpec>, VedError> {
        self.mounts.iter().map(|m| MountSpec::parse(m)).collect()
    }

    pub fn typed_entrypoint(&self) -> Option<CommandSpec> {
        self.entrypoint
            .as_ref()
            .and_then(|c| serde_json::from_value(c.clone()).ok())
    }

    pub fn typed_command(&self) -> Option<CommandSpec> {
        self.command
            .as_ref()
            .and_then(|c| serde_json::from_value(c.clone()).ok())
    }

    pub fn restart_policy(&self) -> Option<RestartPolicy> {
        self.restart.as_deref().and_then(|r| match r {
            "no" => Some(RestartPolicy::No),
            "always" => Some(RestartPolicy::Always),
            "on-failure" => Some(RestartPolicy::OnFailure),
            "unless-stopped" => Some(RestartPolicy::UnlessStopped),
            _ => None,
        })
    }

    pub fn extract_ports(&self) -> Vec<(u16, u16)> {
        self.typed_ports()
            .into_iter()
            .map(|p| (p.host, p.container))
            .collect()
    }

    pub fn container_port(&self) -> u16 {
        self.extract_ports()
            .first()
            .map(|(_, cp)| *cp)
            .unwrap_or(80)
    }
}

fn default_group() -> String {
    "default".to_string()
}

#[cfg(feature = "host")]
pub fn check_implicit_env_flag(name: &str) -> bool {
    let flag_name = format!("{}_ENABLED", name.replace('-', "_").to_uppercase());
    let val = crate::config::get_prop(&flag_name, "");
    if !val.is_empty() {
        let v = val.trim().to_lowercase();
        return !matches!(v.as_str(), "0" | "false" | "no" | "off");
    }
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub image: String,
    #[serde(default = "default_group")]
    pub group: String,
    #[serde(default)]
    pub entrypoint: Option<serde_json::Value>,
    #[serde(default)]
    pub command: Option<serde_json::Value>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub hook: Option<String>,
    #[serde(default)]
    pub mounts: Vec<String>,
    #[serde(default)]
    pub env: IndexMap<String, serde_json::Value>,
    #[serde(rename = "waitFor", default)]
    pub wait_for: Vec<String>,
    #[serde(default)]
    pub restart: Option<String>,
    #[serde(default)]
    pub when: Option<serde_json::Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, rename = "container_name", alias = "containerName")]
    pub container_name: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub deploy: Option<serde_json::Value>,
    #[serde(rename = "shm_size", alias = "shmSize", default)]
    pub shm_size: Option<serde_json::Value>,
    #[serde(default)]
    pub ulimits: Option<serde_json::Value>,
}

impl Default for JobSpec {
    fn default() -> Self {
        Self {
            image: String::new(),
            group: default_group(),
            entrypoint: None,
            command: None,
            schedule: None,
            hook: None,
            mounts: Vec::new(),
            env: IndexMap::new(),
            wait_for: Vec::new(),
            restart: None,
            when: None,
            enabled: None,
            container_name: None,
            profiles: Vec::new(),
            deploy: None,
            shm_size: None,
            ulimits: None,
        }
    }
}

impl JobSpec {
    /// Use explicit activation, then ambient host configuration when `host` is enabled.
    /// Without `host`, unspecified activation defaults to true. Contextual rendering
    /// resolves activation from its supplied snapshot before calling this method.
    pub fn is_enabled(&self, name: &str) -> bool {
        if let Some(explicit) = self.enabled {
            return explicit;
        }
        #[cfg(feature = "host")]
        {
            check_implicit_env_flag(name)
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = name;
            true
        }
    }

    pub fn typed_entrypoint(&self) -> Option<CommandSpec> {
        self.entrypoint
            .as_ref()
            .and_then(|c| serde_json::from_value(c.clone()).ok())
    }

    pub fn typed_command(&self) -> Option<CommandSpec> {
        self.command
            .as_ref()
            .and_then(|c| serde_json::from_value(c.clone()).ok())
    }

    pub fn typed_mounts(&self) -> Result<Vec<MountSpec>, VedError> {
        self.mounts.iter().map(|m| MountSpec::parse(m)).collect()
    }

    pub fn restart_policy(&self) -> Option<RestartPolicy> {
        self.restart.as_deref().and_then(|r| match r {
            "no" => Some(RestartPolicy::No),
            "always" => Some(RestartPolicy::Always),
            "on-failure" => Some(RestartPolicy::OnFailure),
            "unless-stopped" => Some(RestartPolicy::UnlessStopped),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSpec {
    pub when: serde_json::Value,
    #[serde(default)]
    pub services: IndexMap<String, ServiceSpec>,
    #[serde(default)]
    pub jobs: IndexMap<String, JobSpec>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_port_spec_parsing_and_equality() {
        let p1 = PortSpec::parse("8080:80").unwrap();
        assert_eq!(p1.host(), 8080);
        assert_eq!(p1.container(), 80);
        assert_eq!(p1, json!("8080:80"));

        let p2 = PortSpec::parse("3000").unwrap();
        assert_eq!(p2.host(), 3000);
        assert_eq!(p2.container(), 3000);
        assert_eq!(p2, json!(3000));
        assert_eq!(p2, json!("3000:3000"));

        let p3 = PortSpec::try_from(json!(9090)).unwrap();
        assert_eq!(p3.host(), 9090);
        assert_eq!(p3.container(), 9090);

        assert!(PortSpec::parse("invalid").is_err());
        assert!(PortSpec::parse("80:abc").is_err());
    }

    #[test]
    fn test_command_spec_parsing() {
        let cmd_str = CommandSpec::String("bundle exec puma -C config/puma.rb".to_string());
        assert_eq!(
            cmd_str.to_args(),
            vec!["bundle", "exec", "puma", "-C", "config/puma.rb"]
        );

        let cmd_list = CommandSpec::List(vec!["python".to_string(), "app.py".to_string()]);
        assert_eq!(cmd_list.to_args(), vec!["python", "app.py"]);
        assert_eq!(cmd_list.to_display_string(), "python app.py");
    }

    #[test]
    fn test_health_probe_resolution() {
        let probe_http = HealthProbe::parse_cmd("/healthz", 8080);
        assert_eq!(
            probe_http,
            HealthProbe::Http {
                path: "/healthz".to_string(),
                port: 8080,
                scheme: "HTTP".to_string(),
            }
        );

        let probe_url = HealthProbe::parse_cmd("http://localhost:3000/api/health", 8080);
        assert_eq!(
            probe_url,
            HealthProbe::Http {
                path: "/api/health".to_string(),
                port: 3000,
                scheme: "HTTP".to_string(),
            }
        );

        let probe_tcp = HealthProbe::parse_cmd("tcp:6379", 8080);
        assert_eq!(probe_tcp, HealthProbe::Tcp { port: 6379 });

        let probe_exec = HealthProbe::parse_cmd("redis-cli ping", 6379);
        assert_eq!(
            probe_exec,
            HealthProbe::Exec {
                command: vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "redis-cli ping".to_string()
                ]
            }
        );
    }

    #[test]
    fn test_mount_spec_parsing() {
        let m1 = MountSpec::parse("./data:/var/lib/data:ro").unwrap();
        assert_eq!(m1.host_or_volume, "./data");
        assert_eq!(m1.container_path, "/var/lib/data");
        assert!(m1.read_only);
        assert_eq!(
            m1.kind,
            MountKind::HostPath {
                volume_name: "vol-data".to_string(),
                host_path: "./data".to_string(),
                mount_path: "/var/lib/data".to_string(),
                read_only: true,
            }
        );

        let m2 = MountSpec::parse("pgdata:/var/lib/postgresql/data").unwrap();
        assert_eq!(m2.host_or_volume, "pgdata");
        assert_eq!(m2.container_path, "/var/lib/postgresql/data");
        assert!(!m2.read_only);
        assert_eq!(
            m2.kind,
            MountKind::PersistentVolumeClaim {
                claim_name: "pgdata".to_string(),
                mount_path: "/var/lib/postgresql/data".to_string(),
                read_only: false,
            }
        );

        let m3 = MountSpec::parse("cm:app-config:/etc/app:ro").unwrap();
        assert_eq!(m3.host_or_volume, "app-config");
        assert_eq!(m3.container_path, "/etc/app");
        assert!(m3.read_only);
        assert_eq!(
            m3.kind,
            MountKind::ConfigMap {
                config_map_name: "app-config".to_string(),
                mount_path: "/etc/app".to_string(),
                read_only: true,
            }
        );

        let m4 = MountSpec::parse("configmap:app-secrets:/etc/secrets:rw").unwrap();
        assert_eq!(
            m4.kind,
            MountKind::ConfigMap {
                config_map_name: "app-secrets".to_string(),
                mount_path: "/etc/secrets".to_string(),
                read_only: false,
            }
        );

        let m5 = MountSpec::parse("/var/log:/var/log").unwrap();
        assert_eq!(
            m5.kind,
            MountKind::HostPath {
                volume_name: "vol-var-log".to_string(),
                host_path: "/var/log".to_string(),
                mount_path: "/var/log".to_string(),
                read_only: false,
            }
        );

        assert!(MountSpec::parse("invalid").is_err());
    }

    #[test]
    fn test_mount_spec_parsing_diagnostics() {
        let err1 = MountSpec::parse("single_token").unwrap_err().to_string();
        assert!(
            err1.contains("Mount string must contain 2 or 3 colon-separated segments"),
            "got: {err1}"
        );

        let err2 = MountSpec::parse("a:b:c:d").unwrap_err().to_string();
        assert!(
            err2.contains("Mount string must contain 2 or 3 colon-separated segments"),
            "got: {err2}"
        );

        let err3 = MountSpec::parse(":/target").unwrap_err().to_string();
        assert!(
            err3.contains("Mount source path cannot be empty"),
            "got: {err3}"
        );

        let err4 = MountSpec::parse("source:").unwrap_err().to_string();
        assert!(
            err4.contains("Mount destination path cannot be empty"),
            "got: {err4}"
        );

        let err5 = MountSpec::parse("source:relative/path")
            .unwrap_err()
            .to_string();
        assert!(
            err5.contains("Mount destination must be an absolute path starting with '/'"),
            "got: {err5}"
        );

        let err6 = MountSpec::parse("source:/target:invalid")
            .unwrap_err()
            .to_string();
        assert!(
            err6.contains("Mount mode must be 'ro' or 'rw', found 'invalid'"),
            "got: {err6}"
        );

        let err7 = MountSpec::parse("Invalid_PVC!:/data")
            .unwrap_err()
            .to_string();
        assert!(
            err7.contains("PVC claim name 'Invalid_PVC!' must be a valid DNS-1123 label"),
            "got: {err7}"
        );

        let err8 = MountSpec::parse("cm:-bad-name-:/cfg")
            .unwrap_err()
            .to_string();
        assert!(
            err8.contains("ConfigMap name '-bad-name-' must be a valid DNS-1123 label"),
            "got: {err8}"
        );
    }

    #[test]
    fn test_dns1123_and_volume_name_sanitization() {
        assert!(is_valid_dns1123_label("my-pvc-1"));
        assert!(!is_valid_dns1123_label("-bad-start"));
        assert!(!is_valid_dns1123_label("bad-end-"));
        assert!(!is_valid_dns1123_label("UPPERCASE"));
        assert!(!is_valid_dns1123_label("with_underscore"));
        assert!(!is_valid_dns1123_label(""));
        assert!(!is_valid_dns1123_label(&"a".repeat(64)));
        assert!(is_valid_dns1123_label(&"a".repeat(63)));

        assert_eq!(
            sanitize_volume_name("./config/app.json"),
            "vol-config-app-json"
        );
        assert_eq!(sanitize_volume_name("/var/log/app"), "vol-var-log-app");
        assert_eq!(sanitize_volume_name("./"), "vol-data");
        assert_eq!(sanitize_volume_name(""), "vol-data");
    }

    #[test]
    fn test_app_spec_validate_mounts() {
        let mut spec = AppSpec::default();
        let svc = ServiceSpec {
            mounts: vec![
                "./data:/app/data:ro".to_string(),
                "cm:my-cfg:/etc/cfg".to_string(),
            ],
            ..Default::default()
        };
        spec.services.insert("web".to_string(), svc);

        assert!(spec.validate().is_ok());

        spec.services
            .get_mut("web")
            .unwrap()
            .mounts
            .push("bad-mount".to_string());
        let err = spec.validate().unwrap_err().to_string();
        assert!(err.contains("Service 'web': invalid mounts[2]"));

        spec.services.get_mut("web").unwrap().mounts.pop();
        let job = JobSpec {
            image: "busybox".to_string(),
            group: "default".to_string(),
            entrypoint: None,
            command: None,
            schedule: None,
            hook: None,
            mounts: vec!["Invalid_Name!:/data".to_string()],
            env: IndexMap::new(),
            wait_for: Vec::new(),
            restart: None,
            when: None,
            enabled: None,
            ..Default::default()
        };
        spec.jobs.insert("migrate".to_string(), job);
        let err = spec.validate().unwrap_err().to_string();
        assert!(err.contains("Job 'migrate': invalid mounts[0]"));
    }

    #[test]
    fn test_restart_policy_resolution() {
        let mut svc = ServiceSpec {
            restart: Some("unless-stopped".to_string()),
            ..Default::default()
        };
        assert_eq!(svc.restart_policy(), Some(RestartPolicy::UnlessStopped));

        svc.restart = Some("always".to_string());
        assert_eq!(svc.restart_policy(), Some(RestartPolicy::Always));

        svc.restart = Some("invalid".to_string());
        assert_eq!(svc.restart_policy(), None);
    }
}

#[cfg(test)]
mod checked_port_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checked_constructors_enforce_both_boundaries_and_preserve_legacy_dto() {
        for host in [1, u16::MAX] {
            for container in [1, u16::MAX] {
                let checked = CheckedPortSpec::new(host, container).unwrap();
                assert_eq!((checked.host(), checked.container()), (host, container));
                let legacy: PortSpec = checked.into();
                assert_eq!(legacy, PortSpec { host, container });
                assert_eq!(CheckedPortSpec::try_from(legacy).unwrap(), checked);
            }
        }
        assert_eq!(
            CheckedPortSpec::new(0, 1),
            Err(PortValidationError::Zero { field: "host" })
        );
        assert_eq!(
            CheckedPortSpec::new(1, 0),
            Err(PortValidationError::Zero { field: "container" })
        );
        assert!(CheckedPortSpec::single(0).is_err());
        let mut legacy = PortSpec::single(0);
        legacy.container = 80;
        assert!(CheckedPortSpec::try_from(legacy).is_err());
        assert_eq!(PortSpec::parse("0").unwrap().host, 0);
    }

    #[test]
    fn checked_serde_rejects_invalid_wire_values() {
        for value in [
            json!(0),
            json!(-1),
            json!(65536),
            json!(2.5),
            json!(true),
            json!(null),
            json!({}),
            json!([]),
            json!("0"),
            json!("0:80"),
            json!("80:0"),
            json!("65536:80"),
            json!("80:65536"),
            json!("-1"),
            json!("2.5"),
            json!(""),
            json!("80:"),
            json!(":80"),
            json!("1:2:3"),
        ] {
            assert!(
                serde_json::from_value::<CheckedPortSpec>(value.clone()).is_err(),
                "accepted {value}"
            );
        }
        let error = CheckedPortSpec::parse("80:invalid").unwrap_err();
        assert!(error.to_string().contains("container"));
        assert!(error.to_string().contains("80:invalid"));
        assert!(std::error::Error::source(&error).is_none());
    }

    #[test]
    fn checked_serde_matches_legacy_canonical_wire_format() {
        for (value, host, container) in [
            (json!(1), 1, 1),
            (json!(65535), 65535, 65535),
            (json!("8080"), 8080, 8080),
            (json!(" 9000:8080 "), 9000, 8080),
        ] {
            let checked: CheckedPortSpec = serde_json::from_value(value).unwrap();
            assert_eq!(checked, CheckedPortSpec::new(host, container).unwrap());
            let canonical = serde_json::to_value(checked).unwrap();
            assert_eq!(
                canonical,
                serde_json::to_value(PortSpec::from(checked)).unwrap()
            );
            assert_eq!(canonical, json!(format!("{host}:{container}")));
            assert_eq!(
                serde_json::from_value::<CheckedPortSpec>(canonical).unwrap(),
                checked
            );
            assert_eq!(
                format!("{host}:{container}")
                    .parse::<CheckedPortSpec>()
                    .unwrap(),
                checked
            );
        }
    }
}
