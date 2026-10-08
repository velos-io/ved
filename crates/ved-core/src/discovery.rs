use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModuleKind {
    Node,
    Maven,
    Gradle,
    Go,
    Python,
    Rust,
    Docker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedModule {
    pub name: String,
    pub path: String,
    pub kinds: Vec<ModuleKind>,
    pub build_cmd: String,
    pub has_dockerfile: bool,
    pub default_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub updated_at: String,
    pub root: String,
    pub modules: Vec<DetectedModule>,
}

/// Check if a directory name matches common build artifacts, cache, or ignored directories.
pub fn is_ignored_directory(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "target"
                | "node_modules"
                | "dist"
                | "build"
                | crate::r::dirs::INSTALLER_DIR
                | crate::r::dirs::CONFIG_DIR
                | "agent-work"
                | "vendor"
                | "venv"
                | "__pycache__"
                | crate::r::dirs::VED_DIR
        )
}

pub fn scan_workspace(root: &Path) -> WorkspaceState {
    let mut modules = Vec::new();

    if let Some(m) = inspect_directory(root, "app", ".") {
        modules.push(m);
    }

    if let Ok(entries) = fs::read_dir(root) {
        let mut dirs: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
            .collect();
        dirs.sort_by_key(|a| a.file_name());

        for entry in dirs {
            let name_os = entry.file_name();
            let name = name_os.to_string_lossy();

            if is_ignored_directory(&name) {
                continue;
            }

            let path = format!("./{name}");
            if let Some(m) = inspect_directory(&entry.path(), &name, &path) {
                modules.push(m);
            }
        }
    }

    WorkspaceState {
        updated_at: chrono_now(),
        root: root.display().to_string(),
        modules,
    }
}

fn inspect_directory(dir: &Path, name: &str, rel_path: &str) -> Option<DetectedModule> {
    let mut kinds = Vec::new();
    let mut build_cmd = String::new();
    let mut default_port = 8080;

    let has_dockerfile = dir.join("Dockerfile").exists();
    if has_dockerfile {
        kinds.push(ModuleKind::Docker);
        build_cmd = format!("docker build -t {name} {rel_path}");
    }

    if dir.join("package.json").exists() {
        kinds.push(ModuleKind::Node);
        if build_cmd.is_empty() {
            build_cmd = "npm run build".to_string();
        }
        default_port = 3000;
    }

    if dir.join("pom.xml").exists() {
        kinds.push(ModuleKind::Maven);
        if build_cmd.is_empty() {
            build_cmd = "mvn package -DskipTests".to_string();
        }
        default_port = 8080;
    }

    if dir.join("build.gradle").exists() || dir.join("build.gradle.kts").exists() {
        kinds.push(ModuleKind::Gradle);
        if build_cmd.is_empty() {
            build_cmd = if dir.join("gradlew").exists() {
                "./gradlew build -x test".to_string()
            } else {
                "gradle build -x test".to_string()
            };
        }
        default_port = 8080;
    }

    if dir.join("go.mod").exists() {
        kinds.push(ModuleKind::Go);
        if build_cmd.is_empty() {
            build_cmd = "go build ./...".to_string();
        }
        default_port = 8080;
    }

    if dir.join("pyproject.toml").exists() || dir.join("requirements.txt").exists() {
        kinds.push(ModuleKind::Python);
        if build_cmd.is_empty() {
            build_cmd = "python3 -m build".to_string();
        }
        default_port = 8000;
    }

    if dir.join("Cargo.toml").exists() {
        kinds.push(ModuleKind::Rust);
        if build_cmd.is_empty() {
            build_cmd = "cargo build --release".to_string();
        }
        default_port = 8080;
    }

    if kinds.is_empty() {
        None
    } else {
        Some(DetectedModule {
            name: name.to_string(),
            path: rel_path.to_string(),
            kinds,
            build_cmd,
            has_dockerfile,
            default_port,
        })
    }
}

pub fn save_workspace_state(state: &WorkspaceState, ved_dir: &Path) -> Result<()> {
    fs::create_dir_all(ved_dir)?;
    let state_file = ved_dir.join("state.json");
    let json = serde_json::to_string_pretty(state)?;
    fs::write(&state_file, json)?;
    Ok(())
}

pub fn load_workspace_state(ved_dir: &Path) -> Result<WorkspaceState> {
    let state_file = ved_dir.join("state.json");
    let content = fs::read_to_string(&state_file)?;
    let state: WorkspaceState = serde_json::from_str(&content)?;
    Ok(state)
}

pub fn generate_scaffold_vedfile(app_name: &str, state: &WorkspaceState) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\"\"\"Vedfile — application deployment specification.\"\"\"\n\n\
from ved import App, Health, Port, Service\n\n\
app = App(\n    name=\"{app_name}\",\n    version=\"0.1.0\",\n    network=\"{app_name}-net\",\n)\n\n"
    ));

    if state.modules.is_empty() {
        out.push_str(
            "app.services.add(\n\
    Service(\n\
        name=\"web\",\n\
        image=\"nginx:alpine\",\n\
        ports=[Port(80, host=8080)],\n\
        health=Health.http(\"/\"),\n\
    )\n)\n",
        );
        return out;
    }

    for (idx, m) in state.modules.iter().enumerate() {
        let port = m.default_port + (idx as u16 * 10);
        let image = if m.has_dockerfile {
            format!("{}:latest", m.name)
        } else if m.kinds.contains(&ModuleKind::Node) {
            "node:20-alpine".to_string()
        } else if m.kinds.contains(&ModuleKind::Go) {
            "golang:1.22-alpine".to_string()
        } else if m.kinds.contains(&ModuleKind::Python) {
            "python:3.11-alpine".to_string()
        } else {
            "alpine:latest".to_string()
        };

        out.push_str(&format!(
            "# Module: {} ({})\n\
app.services.add(\n\
    Service(\n\
        name=\"{}\",\n\
        image=\"{}\",\n\
        ports=[Port({}, host={})],\n\
        health=Health.http(\"/\"),\n\
    )\n)\n\n",
            m.name, m.path, m.name, image, m.default_port, port
        ));
    }

    out
}

fn chrono_now() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{}", dur.as_secs(), dur.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inspect_directory_node() {
        let test_dir = std::env::temp_dir().join(format!("ved_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&test_dir);
        fs::write(test_dir.join("package.json"), "{}").unwrap();
        let m = inspect_directory(&test_dir, "my-node", "./my-node").unwrap();
        assert_eq!(m.name, "my-node");
        assert!(m.kinds.contains(&ModuleKind::Node));
        assert_eq!(m.default_port, 3000);
        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_scaffold_with_detected_modules() {
        let state = WorkspaceState {
            updated_at: "now".to_string(),
            root: ".".to_string(),
            modules: vec![DetectedModule {
                name: "api".to_string(),
                path: "./api".to_string(),
                kinds: vec![ModuleKind::Go],
                build_cmd: "go build".to_string(),
                has_dockerfile: false,
                default_port: 8080,
            }],
        };
        let code = generate_scaffold_vedfile("demo", &state);
        assert!(code.contains("name=\"api\""));
        assert!(code.contains("golang:1.22-alpine"));
    }

    #[test]
    fn test_is_ignored_directory() {
        assert!(is_ignored_directory(".git"));
        assert!(is_ignored_directory(crate::r::dirs::VED_DIR));
        assert!(is_ignored_directory("target"));
        assert!(is_ignored_directory("node_modules"));
        assert!(is_ignored_directory("dist"));
        assert!(is_ignored_directory("build"));
        assert!(is_ignored_directory(crate::r::dirs::INSTALLER_DIR));
        assert!(is_ignored_directory(crate::r::dirs::CONFIG_DIR));
        assert!(is_ignored_directory("agent-work"));
        assert!(is_ignored_directory("vendor"));
        assert!(is_ignored_directory("venv"));
        assert!(is_ignored_directory("__pycache__"));

        assert!(!is_ignored_directory("services"));
        assert!(!is_ignored_directory("api-gateway"));
        assert!(!is_ignored_directory("frontend"));
    }
}
