use crate::error::{CliError, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CliDirs {
    pub dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub work_dir: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspacePaths {
    pub root: PathBuf,
    pub config_dir: PathBuf,
    pub config_dir_explicit: bool,
    pub work_dir: PathBuf,
    pub work_dir_explicit: bool,
    pub output_dir: Option<PathBuf>,
}

impl WorkspacePaths {
    pub fn resolve(cli: &CliDirs, cwd: &Path) -> Self {
        Self::resolve_with_env(cli, cwd, |k| std::env::var(k).ok())
    }

    pub fn resolve_with_env<F>(cli: &CliDirs, cwd: &Path, env_lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let root = match &cli.dir {
            Some(d) if !d.as_os_str().is_empty() => {
                if d.is_absolute() {
                    d.clone()
                } else {
                    cwd.join(d)
                }
            }
            _ => cwd.to_path_buf(),
        };

        let env_config = env_lookup("VED_CONFIG_DIR").filter(|s| !s.trim().is_empty());
        let (config_dir, config_dir_explicit) = match (&cli.config_dir, env_config) {
            (Some(c), _) if !c.as_os_str().is_empty() => {
                let p = if c.is_absolute() {
                    c.clone()
                } else {
                    cwd.join(c)
                };
                (p, true)
            }
            (_, Some(e)) => {
                let p = PathBuf::from(e.trim());
                let p = if p.is_absolute() { p } else { cwd.join(p) };
                (p, true)
            }
            _ => {
                let local = root.join(ved_core::r::dirs::CONFIG_DIR);
                let fallback = root.parent().map(|p| p.join(ved_core::r::dirs::CONFIG_DIR));
                if !local.is_dir() {
                    if let Some(parent) = fallback {
                        if parent.is_dir() {
                            (parent, false)
                        } else {
                            (local, false)
                        }
                    } else {
                        (local, false)
                    }
                } else {
                    (local, false)
                }
            }
        };

        let env_work = env_lookup("VED_WORK_DIR").filter(|s| !s.trim().is_empty());
        let (mut work_dir, work_dir_explicit) = match (&cli.work_dir, env_work) {
            (Some(w), _) if !w.as_os_str().is_empty() => {
                let p = if w.is_absolute() {
                    w.clone()
                } else {
                    cwd.join(w)
                };
                (p, true)
            }
            (_, Some(e)) => {
                let p = PathBuf::from(e.trim());
                let p = if p.is_absolute() { p } else { cwd.join(p) };
                (p, true)
            }
            _ => (root.join(ved_core::r::dirs::VED_DIR), false),
        };

        let env_out = env_lookup("VED_OUTPUT_DIR").filter(|s| !s.trim().is_empty());
        let output_dir = match (&cli.output_dir, env_out) {
            (Some(o), _) if !o.as_os_str().is_empty() => {
                let p = if o.is_absolute() {
                    o.clone()
                } else {
                    cwd.join(o)
                };
                Some(p)
            }
            (_, Some(e)) => {
                let p = PathBuf::from(e.trim());
                let p = if p.is_absolute() { p } else { cwd.join(p) };
                Some(p)
            }
            _ => None,
        };

        // When artifacts are redirected and no work directory is configured,
        // keep their ownership metadata and publisher lock beside the artifacts.
        if !work_dir_explicit {
            if let Some(output) = &output_dir {
                work_dir = output.join(".ved");
            }
        }

        Self {
            root,
            config_dir,
            config_dir_explicit,
            work_dir,
            work_dir_explicit,
            output_dir,
        }
    }

    pub fn generated_env_file(&self) -> PathBuf {
        if self.work_dir_explicit {
            self.work_dir.join(".generated.env")
        } else if let Some(output_dir) = &self.output_dir {
            output_dir.join(".generated.env")
        } else {
            self.config_dir.join(".generated.env")
        }
    }

    pub fn state_file(&self) -> PathBuf {
        self.work_dir.join("state.json")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirNeeds {
    None,
    Read,
    ReadWrite,
}

pub(crate) fn preflight(paths: &WorkspacePaths, needs: DirNeeds) -> Result<()> {
    if needs == DirNeeds::None {
        return Ok(());
    }

    if paths.config_dir_explicit {
        if !paths.config_dir.exists() || !paths.config_dir.is_dir() {
            return Err(CliError::ConfigDirNotFound(paths.config_dir.clone()));
        }
    } else if paths.config_dir.exists() && !paths.config_dir.is_dir() {
        return Err(CliError::ConfigDirNotFound(paths.config_dir.clone()));
    }

    if needs == DirNeeds::ReadWrite {
        if fs::create_dir_all(&paths.work_dir).is_err() {
            return Err(CliError::dir_not_writable("work", &paths.work_dir));
        }
        let work_probe = paths
            .work_dir
            .join(format!(".ved-write-probe-{}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&work_probe)
        {
            Ok(_) => {
                let _ = fs::remove_file(&work_probe);
            }
            Err(_) => {
                return Err(CliError::dir_not_writable("work", &paths.work_dir));
            }
        }

        if let Some(ref out) = paths.output_dir {
            if fs::create_dir_all(out).is_err() {
                return Err(CliError::dir_not_writable("output", out));
            }
            let out_probe = out.join(format!(".ved-write-probe-{}", std::process::id()));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out_probe)
            {
                Ok(_) => {
                    let _ = fs::remove_file(&out_probe);
                }
                Err(_) => {
                    return Err(CliError::dir_not_writable("output", out));
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactLayout {
    pub compose_file: PathBuf,
    pub k8s_dir: PathBuf,
    pub helm_values: PathBuf,
}

impl ArtifactLayout {
    pub fn from_paths(paths: &WorkspacePaths, compose_path_prop: &str) -> Self {
        if let Some(ref out) = paths.output_dir {
            let k8s_dir =
                if out.file_name().and_then(|s| s.to_str()) == Some(ved_core::r::dirs::K8S_DIR) {
                    out.clone()
                } else {
                    out.join(ved_core::r::dirs::K8S_DIR)
                };
            Self {
                compose_file: out.join(ved_core::r::files::COMPOSE_FILE),
                k8s_dir,
                helm_values: out.join(ved_core::r::files::VALUES_FILE),
            }
        } else {
            let comp = if Path::new(compose_path_prop).is_absolute() {
                PathBuf::from(compose_path_prop)
            } else {
                paths.root.join(compose_path_prop)
            };
            Self {
                compose_file: comp,
                k8s_dir: paths
                    .root
                    .join(ved_core::r::dirs::INSTALLER_DIR)
                    .join(ved_core::r::dirs::K8S_DIR),
                helm_values: paths
                    .root
                    .join(ved_core::r::dirs::INSTALLER_DIR)
                    .join(ved_core::r::files::VALUES_FILE),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_precedence_cli_over_env_over_default() {
        let cwd = PathBuf::from("/cwd");

        let empty_cli = CliDirs::default();
        let paths = WorkspacePaths::resolve_with_env(&empty_cli, &cwd, |_| None);
        assert_eq!(paths.root, cwd.clone());
        assert_eq!(paths.config_dir, cwd.join("config"));
        assert!(!paths.config_dir_explicit);
        assert_eq!(paths.work_dir, cwd.join(".ved"));
        assert!(!paths.work_dir_explicit);
        assert_eq!(paths.output_dir, None);

        let paths_env = WorkspacePaths::resolve_with_env(&empty_cli, &cwd, |k| match k {
            "VED_CONFIG_DIR" => Some("custom-env-cfg".into()),
            "VED_WORK_DIR" => Some("custom-env-work".into()),
            "VED_OUTPUT_DIR" => Some("custom-env-out".into()),
            _ => None,
        });
        assert_eq!(paths_env.config_dir, cwd.join("custom-env-cfg"));
        assert!(paths_env.config_dir_explicit);
        assert_eq!(paths_env.work_dir, cwd.join("custom-env-work"));
        assert!(paths_env.work_dir_explicit);
        assert_eq!(paths_env.output_dir, Some(cwd.join("custom-env-out")));

        let cli_explicit = CliDirs {
            dir: Some(cwd.join("my-app")),
            config_dir: Some(PathBuf::from("cli-cfg")),
            work_dir: Some(PathBuf::from("cli-work")),
            output_dir: Some(PathBuf::from("cli-out")),
        };
        let paths_cli = WorkspacePaths::resolve_with_env(&cli_explicit, &cwd, |k| match k {
            "VED_CONFIG_DIR" => Some("env-cfg".into()),
            "VED_WORK_DIR" => Some("env-work".into()),
            "VED_OUTPUT_DIR" => Some("env-out".into()),
            _ => None,
        });
        assert_eq!(paths_cli.root, cwd.join("my-app"));
        assert_eq!(paths_cli.config_dir, cwd.join("cli-cfg"));
        assert!(paths_cli.config_dir_explicit);
        assert_eq!(paths_cli.work_dir, cwd.join("cli-work"));
        assert!(paths_cli.work_dir_explicit);
        assert_eq!(paths_cli.output_dir, Some(cwd.join("cli-out")));

        let paths_empty_env =
            WorkspacePaths::resolve_with_env(&empty_cli, &cwd, |_| Some("   ".into()));
        assert_eq!(paths_empty_env.config_dir, cwd.join("config"));
        assert!(!paths_empty_env.config_dir_explicit);
        assert_eq!(paths_empty_env.work_dir, cwd.join(".ved"));
        assert!(!paths_empty_env.work_dir_explicit);
        assert_eq!(paths_empty_env.output_dir, None);
    }

    #[test]
    fn test_relative_paths_join_onto_cwd_not_root() {
        let cwd = PathBuf::from("/workspace/user");
        let cli = CliDirs {
            dir: Some(PathBuf::from("app")),
            config_dir: Some(PathBuf::from("custom/config")),
            work_dir: Some(PathBuf::from("w")),
            output_dir: Some(PathBuf::from("out")),
        };
        let paths = WorkspacePaths::resolve_with_env(&cli, &cwd, |_| None);
        assert_eq!(paths.root, cwd.join("app"));
        assert_eq!(paths.config_dir, cwd.join("custom/config"));
        assert_eq!(paths.work_dir, cwd.join("w"));
        assert_eq!(paths.output_dir, Some(cwd.join("out")));
    }

    #[test]
    fn test_generated_env_destination() {
        let cwd = PathBuf::from("/cwd");
        let empty_cli = CliDirs::default();
        let default_paths = WorkspacePaths::resolve_with_env(&empty_cli, &cwd, |_| None);
        assert_eq!(
            default_paths.generated_env_file(),
            cwd.join("config/.generated.env")
        );

        let explicit_work = CliDirs {
            work_dir: Some(PathBuf::from("var/ved")),
            ..Default::default()
        };
        let explicit_paths = WorkspacePaths::resolve_with_env(&explicit_work, &cwd, |_| None);
        assert_eq!(
            explicit_paths.generated_env_file(),
            cwd.join("var/ved/.generated.env")
        );
        assert_eq!(explicit_paths.state_file(), cwd.join("var/ved/state.json"));
    }

    #[test]
    fn test_artifact_layout_legacy_vs_unified() {
        let cwd = PathBuf::from("/cwd");
        let empty_cli = CliDirs::default();
        let legacy_paths = WorkspacePaths::resolve_with_env(&empty_cli, &cwd, |_| None);
        let legacy_layout = ArtifactLayout::from_paths(&legacy_paths, ".ved/docker-compose.yml");
        assert_eq!(
            legacy_layout.compose_file,
            cwd.join(".ved/docker-compose.yml")
        );
        assert_eq!(legacy_layout.k8s_dir, cwd.join("installer/k8s"));
        assert_eq!(legacy_layout.helm_values, cwd.join("installer/values.yaml"));

        let out_cli = CliDirs {
            output_dir: Some(PathBuf::from("dist")),
            ..Default::default()
        };
        let unified_paths = WorkspacePaths::resolve_with_env(&out_cli, &cwd, |_| None);
        let unified_layout = ArtifactLayout::from_paths(&unified_paths, ".ved/docker-compose.yml");
        assert_eq!(
            unified_layout.compose_file,
            cwd.join("dist/docker-compose.yml")
        );
        assert_eq!(unified_layout.k8s_dir, cwd.join("dist/k8s"));
        assert_eq!(unified_layout.helm_values, cwd.join("dist/values.yaml"));
    }

    #[test]
    fn test_preflight_checks() {
        let tmp = std::env::temp_dir().join(format!("ved_paths_preflight_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let cwd = tmp.clone();

        let missing_cfg_cli = CliDirs {
            config_dir: Some(PathBuf::from("non-existent-cfg")),
            ..Default::default()
        };
        let paths = WorkspacePaths::resolve_with_env(&missing_cfg_cli, &cwd, |_| None);
        assert!(matches!(
            preflight(&paths, DirNeeds::Read),
            Err(CliError::ConfigDirNotFound(_))
        ));

        let default_cli = CliDirs {
            dir: Some(tmp.join("project")),
            ..Default::default()
        };
        let paths_default = WorkspacePaths::resolve_with_env(&default_cli, &cwd, |_| None);
        assert!(preflight(&paths_default, DirNeeds::Read).is_ok());

        let rw_cli = CliDirs {
            dir: Some(tmp.join("project")),
            work_dir: Some(tmp.join("new_work")),
            output_dir: Some(tmp.join("new_output")),
            ..Default::default()
        };
        let paths_rw = WorkspacePaths::resolve_with_env(&rw_cli, &cwd, |_| None);
        assert!(!paths_rw.work_dir.exists());
        assert!(!paths_rw.output_dir.as_ref().unwrap().exists());

        assert!(preflight(&paths_rw, DirNeeds::ReadWrite).is_ok());
        assert!(paths_rw.work_dir.exists());
        assert!(paths_rw.output_dir.as_ref().unwrap().exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parent_config_fallback() {
        let tmp = std::env::temp_dir().join(format!("ved_paths_parent_cfg_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let parent_cfg = tmp.join("config");
        let subproject = tmp.join("subproject");
        fs::create_dir_all(&parent_cfg).unwrap();
        fs::create_dir_all(&subproject).unwrap();

        let cli = CliDirs {
            dir: Some(subproject.clone()),
            ..Default::default()
        };
        let paths = WorkspacePaths::resolve_with_env(&cli, &tmp, |_| None);
        assert_eq!(paths.config_dir, parent_cfg);

        // When subproject has its own config, it takes precedence over parent config
        let local_cfg = subproject.join("config");
        fs::create_dir_all(&local_cfg).unwrap();
        let paths_local = WorkspacePaths::resolve_with_env(&cli, &tmp, |_| None);
        assert_eq!(paths_local.config_dir, local_cfg);

        let _ = fs::remove_dir_all(&tmp);
    }
}
