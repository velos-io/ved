use serde_yaml::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, VedError};

/// Starlark expression AST node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// Single-line escaped string literal, e.g., "nginx:alpine"
    Literal(String),
    /// Triple-quoted multiline string, e.g., """sh -c exec ..."""
    Multiline(String),
    /// Numeric integer literal, e.g., 8080
    Int(i64),
    /// Boolean literal, rendered as Python True / False
    #[allow(dead_code)]
    Bool(bool),
    /// Array/List literal, e.g., [Port(80, host=8080)]
    List(Vec<Expr>),
    /// Key-value dictionary, e.g., {"KEY": "VAL"} with sorted keys
    Dict(Vec<(String, Expr)>),
    /// Function or constructor invocation, e.g., Service(name="web")
    Call {
        target: String,
        args: Vec<Expr>,
        kwargs: Vec<(String, Expr)>,
    },
    /// Raw unescaped identifier or preformatted token, e.g., Health.http("/")
    Raw(String),
}

/// Starlark top-level statement AST node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    /// Module-level file docstring
    Docstring(String),
    /// Module import, e.g., from ved import App, Health, Mount, Port, Service
    Import {
        module: String,
        symbols: Vec<String>,
    },
    /// Variable assignment, e.g., app = App(...)
    Assign { target: String, value: Expr },
    /// Standalone expression statement, e.g., app.services.add(Service(...))
    Expr(Expr),
    /// Informational comment line, e.g., # Service: web
    Comment(String),
}

/// Indentation-aware code emitter for Starlark source files.
pub struct StarlarkWriter {
    buffer: String,
    indent_level: usize,
    indent_spaces: usize,
}

impl Default for StarlarkWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl StarlarkWriter {
    pub fn new() -> Self {
        Self {
            buffer: String::with_capacity(2048),
            indent_level: 0,
            indent_spaces: 4,
        }
    }

    pub fn indent(&mut self) {
        self.indent_level += 1;
    }

    pub fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    pub fn emit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Docstring(doc) => {
                self.write_indent();
                self.buffer.push_str("\"\"\"");
                self.buffer.push_str(&Self::escape_multiline(doc));
                self.buffer.push_str("\"\"\"\n\n");
            }
            Stmt::Import { module, symbols } => {
                self.write_indent();
                self.buffer.push_str(&format!(
                    "from {} import {}\n\n",
                    module,
                    symbols.join(", ")
                ));
            }
            Stmt::Assign { target, value } => {
                self.write_indent();
                self.buffer.push_str(target);
                self.buffer.push_str(" = ");
                self.emit_expr(value);
                self.buffer.push_str("\n\n");
            }
            Stmt::Expr(expr) => {
                self.write_indent();
                self.emit_expr(expr);
                self.buffer.push_str("\n\n");
            }
            Stmt::Comment(comment) => {
                self.write_indent();
                self.buffer.push_str("# ");
                self.buffer.push_str(comment);
                self.buffer.push('\n');
            }
        }
    }

    pub fn emit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Literal(s) => {
                self.buffer.push('"');
                self.buffer.push_str(&Self::escape_single_line(s));
                self.buffer.push('"');
            }
            Expr::Multiline(s) => {
                self.buffer.push_str("\"\"\"");
                self.buffer.push_str(&Self::escape_multiline(s));
                self.buffer.push_str("\"\"\"");
            }
            Expr::Int(n) => self.buffer.push_str(&n.to_string()),
            Expr::Bool(b) => self.buffer.push_str(if *b { "True" } else { "False" }),
            Expr::Raw(r) => self.buffer.push_str(r),
            Expr::List(items) => {
                let is_multiline = items.iter().any(|item| {
                    matches!(item, Expr::Call { target, .. } if target.starts_with("Mount."))
                });
                if is_multiline && !items.is_empty() {
                    self.buffer.push_str("[\n");
                    self.indent();
                    for item in items {
                        self.write_indent();
                        self.emit_expr(item);
                        self.buffer.push_str(",\n");
                    }
                    self.dedent();
                    self.write_indent();
                    self.buffer.push(']');
                } else {
                    self.buffer.push('[');
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            self.buffer.push_str(", ");
                        }
                        self.emit_expr(item);
                    }
                    self.buffer.push(']');
                }
            }
            Expr::Dict(entries) => {
                if entries.is_empty() {
                    self.buffer.push_str("{}");
                    return;
                }
                let mut sorted_entries = entries.clone();
                sorted_entries.sort_by(|a, b| a.0.cmp(&b.0));

                self.buffer.push_str("{\n");
                self.indent();
                for (k, v) in &sorted_entries {
                    self.write_indent();
                    self.buffer.push('"');
                    self.buffer.push_str(&Self::escape_single_line(k));
                    self.buffer.push_str("\": ");
                    self.emit_expr(v);
                    self.buffer.push_str(",\n");
                }
                self.dedent();
                self.write_indent();
                self.buffer.push('}');
            }
            Expr::Call {
                target,
                args,
                kwargs,
            } => {
                self.emit_call(target, args, kwargs);
            }
        }
    }

    fn emit_call(&mut self, target: &str, args: &[Expr], kwargs: &[(String, Expr)]) {
        let is_multiline = target == "Service"
            || target == "App"
            || kwargs.len() > 1
            || args
                .iter()
                .any(|a| matches!(a, Expr::Call { .. } | Expr::Multiline(_)));

        self.buffer.push_str(target);
        self.buffer.push('(');

        if !is_multiline {
            let mut first = true;
            for arg in args {
                if !first {
                    self.buffer.push_str(", ");
                }
                self.emit_expr(arg);
                first = false;
            }
            for (k, v) in kwargs {
                if !first {
                    self.buffer.push_str(", ");
                }
                self.buffer.push_str(k);
                self.buffer.push('=');
                self.emit_expr(v);
                first = false;
            }
            self.buffer.push(')');
        } else {
            self.buffer.push('\n');
            self.indent();
            for arg in args {
                self.write_indent();
                self.emit_expr(arg);
                if !kwargs.is_empty() || args.len() > 1 {
                    self.buffer.push_str(",\n");
                } else {
                    self.buffer.push('\n');
                }
            }
            for (k, v) in kwargs {
                self.write_indent();
                self.buffer.push_str(k);
                self.buffer.push('=');
                self.emit_expr(v);
                self.buffer.push_str(",\n");
            }
            self.dedent();
            self.write_indent();
            self.buffer.push(')');
        }
    }

    pub fn escape_single_line(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('\"', "\\\"")
            .replace('\r', "")
            .replace('\n', "\\n")
    }

    pub fn escape_multiline(s: &str) -> String {
        let mut normalised = s.replace("\r\n", "\n").replace("\"\"\"", "\\\"\\\"\\\"");
        if normalised.ends_with('\\') && !normalised.ends_with(r"\\") {
            normalised.push(' ');
        }
        normalised
    }

    fn write_indent(&mut self) {
        for _ in 0..(self.indent_level * self.indent_spaces) {
            self.buffer.push(' ');
        }
    }

    pub fn finish(self) -> String {
        self.buffer
    }
}

/// Convert a Docker Compose YAML file into a Starlark Vedfile.
pub fn convert_compose_to_ved(compose_path: &Path, app_name: &str) -> Result<String> {
    let content = fs::read_to_string(compose_path)?;

    let root: Value = serde_yaml::from_str(&content)?;

    let services = match root.get("services") {
        Some(Value::Mapping(m)) => m,
        _ => {
            return Err(VedError::spec_parse(
                compose_path,
                format!("No 'services' mapping found in {}", compose_path.display()),
            ))
        }
    };

    let mut writer = StarlarkWriter::new();
    writer.emit_stmt(&Stmt::Docstring(
        "Vedfile — converted from docker compose.".to_string(),
    ));
    writer.emit_stmt(&Stmt::Import {
        module: "ved".to_string(),
        symbols: vec![
            "App".to_string(),
            "Health".to_string(),
            "Mount".to_string(),
            "Port".to_string(),
            "Service".to_string(),
        ],
    });
    writer.emit_stmt(&Stmt::Assign {
        target: "app".to_string(),
        value: Expr::Call {
            target: "App".to_string(),
            args: Vec::new(),
            kwargs: vec![
                ("name".to_string(), Expr::Literal(app_name.to_string())),
                ("version".to_string(), Expr::Literal("0.1.0".to_string())),
                (
                    "network".to_string(),
                    Expr::Literal(format!("{app_name}-net")),
                ),
            ],
        },
    });

    let mut sorted_keys: Vec<String> = services
        .keys()
        .filter_map(|k| k.as_str().map(|s| s.to_string()))
        .collect();
    sorted_keys.sort();

    for svc_name in &sorted_keys {
        let key_val = Value::String(svc_name.clone());
        let svc_data = match services.get(&key_val) {
            Some(v) => v,
            None => continue,
        };

        writer.emit_stmt(&Stmt::Comment(format!("Service: {svc_name}")));

        let mut svc_kwargs: Vec<(String, Expr)> = Vec::new();
        svc_kwargs.push(("name".to_string(), Expr::Literal(svc_name.clone())));

        if let Some(img) = svc_data.get("image").and_then(|v| v.as_str()) {
            svc_kwargs.push(("image".to_string(), Expr::Literal(img.to_string())));
        }

        if let Some(build) = svc_data.get("build") {
            if let Some(b_str) = build.as_str() {
                svc_kwargs.push(("build".to_string(), Expr::Literal(b_str.to_string())));
            } else if let Some(b_map) = build.as_mapping() {
                let ctx = b_map
                    .get(Value::String("context".to_string()))
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                let df = b_map
                    .get(Value::String("dockerfile".to_string()))
                    .and_then(|v| v.as_str());
                let args_opt = b_map.get(Value::String("args".to_string()));

                if let Some(args_val) = args_opt {
                    let mut json_obj = serde_json::Map::new();
                    json_obj.insert(
                        "context".to_string(),
                        serde_json::Value::String(ctx.to_string()),
                    );
                    if let Some(d) = df {
                        json_obj.insert(
                            "dockerfile".to_string(),
                            serde_json::Value::String(d.to_string()),
                        );
                    }
                    if let Ok(args_json) = serde_json::to_value(args_val) {
                        json_obj.insert("args".to_string(), args_json);
                    }
                    let json_str = serde_json::to_string(&json_obj).unwrap_or_default();
                    svc_kwargs.push(("build".to_string(), Expr::Literal(json_str)));
                } else if let Some(d) = df {
                    if d == "Dockerfile" && (ctx == "." || ctx == "./") {
                        svc_kwargs.push(("build".to_string(), Expr::Literal(".".to_string())));
                    } else if d == "Dockerfile" {
                        svc_kwargs.push(("build".to_string(), Expr::Literal(ctx.to_string())));
                    } else {
                        svc_kwargs.push(("build".to_string(), Expr::Literal(format!("{ctx}:{d}"))));
                    }
                } else {
                    svc_kwargs.push(("build".to_string(), Expr::Literal(ctx.to_string())));
                }
            }
        }

        if let Some(entry_val) = svc_data.get("entrypoint") {
            if let Some(s) = entry_val.as_str() {
                let cleaned = clean_command_string(s);
                if cleaned.contains('\n') {
                    svc_kwargs.push(("entrypoint".to_string(), Expr::Multiline(cleaned)));
                } else {
                    svc_kwargs.push(("entrypoint".to_string(), Expr::Literal(cleaned)));
                }
            } else if let Some(seq) = entry_val.as_sequence() {
                let items: Vec<Expr> = seq
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| Expr::Literal(clean_command_string(s)))
                    .collect();
                if !items.is_empty() {
                    svc_kwargs.push(("entrypoint".to_string(), Expr::List(items)));
                }
            }
        }

        if let Some(cmd_val) = svc_data.get("command") {
            if let Some(s) = cmd_val.as_str() {
                let cleaned = clean_command_string(s);
                if cleaned.contains('\n') {
                    svc_kwargs.push(("command".to_string(), Expr::Multiline(cleaned)));
                } else {
                    svc_kwargs.push(("command".to_string(), Expr::Literal(cleaned)));
                }
            } else if let Some(seq) = cmd_val.as_sequence() {
                let items: Vec<Expr> = seq
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| Expr::Literal(clean_command_string(s)))
                    .collect();
                if !items.is_empty() {
                    svc_kwargs.push(("command".to_string(), Expr::List(items)));
                }
            }
        }

        if let Some(profiles) = svc_data.get("profiles").and_then(|v| v.as_sequence()) {
            if let Some(first) = profiles.first().and_then(|v| v.as_str()) {
                svc_kwargs.push(("group".to_string(), Expr::Literal(first.to_string())));
            }
        }

        if let Some(ports) = svc_data.get("ports").and_then(|v| v.as_sequence()) {
            let mut port_exprs = Vec::new();
            for p in ports {
                if let Some(s) = p.as_str() {
                    let parts: Vec<&str> = s.split(':').collect();
                    if parts.len() == 2 {
                        let hp = parts[0].trim();
                        let cp = parts[1].split('/').next().unwrap_or(parts[1]).trim();
                        let cp_expr = cp
                            .parse::<i64>()
                            .map(Expr::Int)
                            .unwrap_or_else(|_| Expr::Raw(cp.to_string()));
                        let hp_expr = hp
                            .parse::<i64>()
                            .map(Expr::Int)
                            .unwrap_or_else(|_| Expr::Raw(hp.to_string()));
                        port_exprs.push(Expr::Call {
                            target: "Port".to_string(),
                            args: vec![cp_expr],
                            kwargs: vec![("host".to_string(), hp_expr)],
                        });
                    } else if parts.len() == 3 {
                        let hp = parts[1].trim();
                        let cp = parts[2].split('/').next().unwrap_or(parts[2]).trim();
                        let cp_expr = cp
                            .parse::<i64>()
                            .map(Expr::Int)
                            .unwrap_or_else(|_| Expr::Raw(cp.to_string()));
                        let hp_expr = hp
                            .parse::<i64>()
                            .map(Expr::Int)
                            .unwrap_or_else(|_| Expr::Raw(hp.to_string()));
                        port_exprs.push(Expr::Call {
                            target: "Port".to_string(),
                            args: vec![cp_expr],
                            kwargs: vec![("host".to_string(), hp_expr)],
                        });
                    } else if parts.len() == 1 {
                        let cp = parts[0].split('/').next().unwrap_or(parts[0]).trim();
                        let cp_expr = cp
                            .parse::<i64>()
                            .map(Expr::Int)
                            .unwrap_or_else(|_| Expr::Raw(cp.to_string()));
                        port_exprs.push(Expr::Call {
                            target: "Port".to_string(),
                            args: vec![cp_expr.clone()],
                            kwargs: vec![("host".to_string(), cp_expr)],
                        });
                    }
                } else if let Some(n) = p.as_u64() {
                    let n_expr = Expr::Int(n as i64);
                    port_exprs.push(Expr::Call {
                        target: "Port".to_string(),
                        args: vec![n_expr.clone()],
                        kwargs: vec![("host".to_string(), n_expr)],
                    });
                }
            }
            if !port_exprs.is_empty() {
                svc_kwargs.push(("ports".to_string(), Expr::List(port_exprs)));
            }
        }

        let mut env_map = BTreeMap::new();
        if let Some(env_val) = svc_data.get("environment") {
            if let Some(m) = env_val.as_mapping() {
                for (k, v) in m {
                    if let (Some(k_str), Some(v_str)) = (k.as_str(), v.as_str()) {
                        env_map.insert(k_str.to_string(), v_str.to_string());
                    } else if let (Some(k_str), Some(v_int)) = (k.as_str(), v.as_i64()) {
                        env_map.insert(k_str.to_string(), v_int.to_string());
                    }
                }
            } else if let Some(seq) = env_val.as_sequence() {
                for item in seq {
                    if let Some(s) = item.as_str() {
                        let mut parts = s.splitn(2, '=');
                        if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                            env_map.insert(k.trim().to_string(), v.trim().to_string());
                        }
                    }
                }
            }
        }
        if !env_map.is_empty() {
            let env_entries: Vec<(String, Expr)> = env_map
                .into_iter()
                .map(|(k, v)| {
                    let val_expr = if v.contains('\n') {
                        Expr::Multiline(v)
                    } else {
                        Expr::Literal(v)
                    };
                    (k, val_expr)
                })
                .collect();
            svc_kwargs.push(("env".to_string(), Expr::Dict(env_entries)));
        }

        let mut wait_for = Vec::new();
        if let Some(deps) = svc_data.get("depends_on") {
            if let Some(seq) = deps.as_sequence() {
                for dep in seq {
                    if let Some(d_str) = dep.as_str() {
                        wait_for.push(format!("{d_str}.healthy"));
                    }
                }
            } else if let Some(m) = deps.as_mapping() {
                for (k, v) in m {
                    if let Some(k_str) = k.as_str() {
                        let is_healthy = v
                            .get(Value::String("condition".to_string()))
                            .and_then(|c| c.as_str())
                            .map(|c| c.contains("healthy"))
                            .unwrap_or(true);
                        if is_healthy {
                            wait_for.push(format!("{k_str}.healthy"));
                        } else {
                            wait_for.push(k_str.to_string());
                        }
                    }
                }
            }
        }
        if !wait_for.is_empty() {
            let wait_exprs: Vec<Expr> = wait_for.into_iter().map(Expr::Literal).collect();
            svc_kwargs.push(("wait_for".to_string(), Expr::List(wait_exprs)));
        }

        if let Some(hc) = svc_data.get("healthcheck") {
            if let Some(test_val) = hc.get("test") {
                let cmd_str = if let Some(seq) = test_val.as_sequence() {
                    let parts: Vec<&str> = seq.iter().filter_map(|v| v.as_str()).collect();
                    if parts.len() > 1 && (parts[0] == "CMD" || parts[0] == "CMD-SHELL") {
                        parts[1..].join(" ")
                    } else {
                        parts.join(" ")
                    }
                } else if let Some(s) = test_val.as_str() {
                    s.to_string()
                } else {
                    String::new()
                };

                if !cmd_str.is_empty() {
                    if cmd_str.contains("http://localhost") || cmd_str.contains("http://127.0.0.1")
                    {
                        let path = extract_http_path(&cmd_str);
                        svc_kwargs.push((
                            "health".to_string(),
                            Expr::Call {
                                target: "Health.http".to_string(),
                                args: vec![Expr::Literal(path)],
                                kwargs: Vec::new(),
                            },
                        ));
                    } else if cmd_str.starts_with("nc -z") || cmd_str.starts_with("tcp:") {
                        let port_str = cmd_str.split_whitespace().last().unwrap_or("8080");
                        let port_expr = port_str
                            .parse::<i64>()
                            .map(Expr::Int)
                            .unwrap_or_else(|_| Expr::Raw(port_str.to_string()));
                        svc_kwargs.push((
                            "health".to_string(),
                            Expr::Call {
                                target: "Health.tcp".to_string(),
                                args: vec![port_expr],
                                kwargs: Vec::new(),
                            },
                        ));
                    } else if cmd_str.contains('\n') {
                        svc_kwargs.push((
                            "health".to_string(),
                            Expr::Call {
                                target: "Health.exec".to_string(),
                                args: vec![Expr::Multiline(cmd_str)],
                                kwargs: Vec::new(),
                            },
                        ));
                    } else {
                        svc_kwargs.push((
                            "health".to_string(),
                            Expr::Call {
                                target: "Health.exec".to_string(),
                                args: vec![Expr::Literal(cmd_str)],
                                kwargs: Vec::new(),
                            },
                        ));
                    }
                }
            }
        }

        if let Some(vols) = svc_data.get("volumes").and_then(|v| v.as_sequence()) {
            let mut mount_exprs = Vec::new();
            for vol in vols {
                if let Some(s) = vol.as_str() {
                    let parts: Vec<&str> = s.split(':').collect();
                    if parts.len() >= 2 {
                        let src = parts[0];
                        let dst = parts[1];
                        let mode = parts.get(2).copied().unwrap_or("");
                        let target_fn = if src.starts_with("./") || src.starts_with('/') {
                            if mode == "ro" {
                                "Mount.ro".to_string()
                            } else {
                                "Mount.bind".to_string()
                            }
                        } else {
                            "Mount.volume".to_string()
                        };
                        mount_exprs.push(Expr::Call {
                            target: target_fn,
                            args: vec![
                                Expr::Literal(src.to_string()),
                                Expr::Literal(dst.to_string()),
                            ],
                            kwargs: Vec::new(),
                        });
                    }
                }
            }
            if !mount_exprs.is_empty() {
                svc_kwargs.push(("mounts".to_string(), Expr::List(mount_exprs)));
            }
        }

        writer.emit_stmt(&Stmt::Expr(Expr::Call {
            target: "app.services.add".to_string(),
            args: vec![Expr::Call {
                target: "Service".to_string(),
                args: Vec::new(),
                kwargs: svc_kwargs,
            }],
            kwargs: Vec::new(),
        }));
    }

    Ok(writer.finish())
}

fn extract_http_path(cmd: &str) -> String {
    for token in cmd.split_whitespace() {
        if let Some(pos) = token.find("http://") {
            let url = &token[pos..];
            let after_protocol = &url["http://".len()..];
            if let Some(slash_pos) = after_protocol.find('/') {
                let path = &after_protocol[slash_pos..];
                let clean_path = path.trim_end_matches(&['\"', '\'', ';', '|', '>'][..]);
                return clean_path.to_string();
            }
            return "/".to_string();
        }
    }
    "/".to_string()
}

pub struct DiscoveredDockerfile {
    pub name: String,
    pub path: PathBuf,
    pub context: String,
    pub expose_port: Option<u16>,
}

pub use crate::command::clean_command_string;

/// Recursively find Dockerfiles in a directory and generate a project Vedfile.
pub fn scaffold_from_dockerfiles(
    dir: &Path,
    app_name: &str,
) -> Result<(String, Vec<DiscoveredDockerfile>)> {
    let mut discovered = Vec::new();
    scan_for_dockerfiles(dir, dir, 0, &mut discovered)?;

    let mut writer = StarlarkWriter::new();
    writer.emit_stmt(&Stmt::Docstring(
        "Vedfile — project structure based on Dockerfiles.".to_string(),
    ));
    writer.emit_stmt(&Stmt::Import {
        module: "ved".to_string(),
        symbols: vec![
            "App".to_string(),
            "Health".to_string(),
            "Port".to_string(),
            "Service".to_string(),
        ],
    });
    writer.emit_stmt(&Stmt::Assign {
        target: "app".to_string(),
        value: Expr::Call {
            target: "App".to_string(),
            args: Vec::new(),
            kwargs: vec![
                ("name".to_string(), Expr::Literal(app_name.to_string())),
                ("version".to_string(), Expr::Literal("0.1.0".to_string())),
                (
                    "network".to_string(),
                    Expr::Literal(format!("{app_name}-net")),
                ),
            ],
        },
    });

    if discovered.is_empty() {
        writer.emit_stmt(&Stmt::Comment("Root project service".to_string()));
        writer.emit_stmt(&Stmt::Expr(Expr::Call {
            target: "app.services.add".to_string(),
            args: vec![Expr::Call {
                target: "Service".to_string(),
                args: Vec::new(),
                kwargs: vec![
                    ("name".to_string(), Expr::Literal(app_name.to_string())),
                    ("build".to_string(), Expr::Literal(".".to_string())),
                    (
                        "ports".to_string(),
                        Expr::List(vec![Expr::Call {
                            target: "Port".to_string(),
                            args: vec![Expr::Int(8080)],
                            kwargs: vec![("host".to_string(), Expr::Int(8080))],
                        }]),
                    ),
                    (
                        "health".to_string(),
                        Expr::Call {
                            target: "Health.http".to_string(),
                            args: vec![Expr::Literal("/".to_string())],
                            kwargs: Vec::new(),
                        },
                    ),
                ],
            }],
            kwargs: Vec::new(),
        }));
        return Ok((writer.finish(), discovered));
    }

    discovered.sort_by(|a, b| a.name.cmp(&b.name));

    for (idx, df) in discovered.iter().enumerate() {
        let port = df.expose_port.unwrap_or(8080 + (idx as u16 * 10));
        let host_port = port;

        writer.emit_stmt(&Stmt::Comment(format!(
            "Service: {} ({})",
            df.name,
            df.path.display()
        )));
        writer.emit_stmt(&Stmt::Expr(Expr::Call {
            target: "app.services.add".to_string(),
            args: vec![Expr::Call {
                target: "Service".to_string(),
                args: Vec::new(),
                kwargs: vec![
                    ("name".to_string(), Expr::Literal(df.name.clone())),
                    ("build".to_string(), Expr::Literal(df.context.clone())),
                    (
                        "ports".to_string(),
                        Expr::List(vec![Expr::Call {
                            target: "Port".to_string(),
                            args: vec![Expr::Int(port as i64)],
                            kwargs: vec![("host".to_string(), Expr::Int(host_port as i64))],
                        }]),
                    ),
                    (
                        "health".to_string(),
                        Expr::Call {
                            target: "Health.http".to_string(),
                            args: vec![Expr::Literal("/".to_string())],
                            kwargs: Vec::new(),
                        },
                    ),
                ],
            }],
            kwargs: Vec::new(),
        }));
    }

    Ok((writer.finish(), discovered))
}

fn scan_for_dockerfiles(
    root: &Path,
    current: &Path,
    depth: usize,
    results: &mut Vec<DiscoveredDockerfile>,
) -> Result<()> {
    if depth > 4 {
        return Ok(());
    }

    let entries = match fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        if path.is_dir() {
            if crate::discovery::is_ignored_directory(&name_str) {
                continue;
            }
            scan_for_dockerfiles(root, &path, depth + 1, results)?;
        } else if path.is_file() {
            let is_dockerfile = name_str == "Dockerfile"
                || name_str.to_lowercase() == "dockerfile"
                || name_str.starts_with("Dockerfile.")
                || name_str.ends_with(".Dockerfile");

            if is_dockerfile {
                let (svc_name, context) = if current == root {
                    let s_name = if name_str.starts_with("Dockerfile.") {
                        name_str.trim_start_matches("Dockerfile.").to_string()
                    } else {
                        root.file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "app".to_string())
                    };
                    (s_name, ".".to_string())
                } else {
                    let rel = current
                        .strip_prefix(root)
                        .unwrap_or(current)
                        .to_string_lossy()
                        .to_string();
                    let s_name = if name_str.starts_with("Dockerfile.") {
                        name_str.trim_start_matches("Dockerfile.").to_string()
                    } else {
                        current
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "app".to_string())
                    };
                    (s_name, format!("./{rel}"))
                };

                let expose_port = read_dockerfile_expose(&path);

                results.push(DiscoveredDockerfile {
                    name: svc_name,
                    path: path.clone(),
                    context,
                    expose_port,
                });
            }
        }
    }

    Ok(())
}

fn read_dockerfile_expose(path: &Path) -> Option<u16> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.to_uppercase().starts_with("EXPOSE ") {
            let rest = &trimmed[7..].trim();
            let first_token = rest.split_whitespace().next()?;
            let port_str = first_token.split('/').next()?;
            if let Ok(p) = port_str.parse::<u16>() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_writer_literal_escaping() {
        let mut writer = StarlarkWriter::new();
        writer.emit_stmt(&Stmt::Assign {
            target: "greeting".to_string(),
            value: Expr::Literal(r#"Hello "world" \ special"#.to_string()),
        });
        let output = writer.finish();
        assert_eq!(
            output,
            "greeting = \"Hello \\\"world\\\" \\\\ special\"\n\n"
        );
    }

    #[test]
    fn test_writer_multiline_escaping() {
        let mut writer = StarlarkWriter::new();
        writer.emit_stmt(&Stmt::Assign {
            target: "script".to_string(),
            value: Expr::Multiline("echo \"\"\"nested\"\"\"\necho \\".to_string()),
        });
        let output = writer.finish();
        assert!(output.contains("\"\"\"echo \\\"\\\"\\\"nested\\\"\\\"\\\"\necho \\ \"\"\""));
    }

    #[test]
    fn test_writer_sorted_dict() {
        let mut writer = StarlarkWriter::new();
        writer.emit_stmt(&Stmt::Assign {
            target: "env".to_string(),
            value: Expr::Dict(vec![
                ("ZEBRA".to_string(), Expr::Literal("1".to_string())),
                ("APPLE".to_string(), Expr::Literal("2".to_string())),
                ("MANGO".to_string(), Expr::Literal("3".to_string())),
            ]),
        });
        let output = writer.finish();
        let apple_idx = output.find("\"APPLE\"").unwrap();
        let mango_idx = output.find("\"MANGO\"").unwrap();
        let zebra_idx = output.find("\"ZEBRA\"").unwrap();
        assert!(apple_idx < mango_idx);
        assert!(mango_idx < zebra_idx);
    }

    #[test]
    fn test_writer_nested_indentation() {
        let mut writer = StarlarkWriter::new();
        writer.emit_stmt(&Stmt::Expr(Expr::Call {
            target: "app.services.add".to_string(),
            args: vec![Expr::Call {
                target: "Service".to_string(),
                args: Vec::new(),
                kwargs: vec![
                    ("name".to_string(), Expr::Literal("web".to_string())),
                    (
                        "ports".to_string(),
                        Expr::List(vec![Expr::Call {
                            target: "Port".to_string(),
                            args: vec![Expr::Int(80)],
                            kwargs: vec![("host".to_string(), Expr::Int(8080))],
                        }]),
                    ),
                ],
            }],
            kwargs: Vec::new(),
        }));
        let output = writer.finish();
        assert!(output.starts_with("app.services.add(\n    Service(\n"));
        assert!(output.contains("        name=\"web\",\n"));
        assert!(output.contains("        ports=[Port(80, host=8080)],\n"));
        assert!(output.ends_with("    )\n)\n\n"));
    }

    #[test]
    fn test_convert_compose_to_ved() {
        let yaml = r#"
version: '3.8'
services:
  web:
    image: nginx:alpine
    ports:
      - "8080:80"
    depends_on:
      - db
    healthcheck:
      test: ["CMD-SHELL", "wget -qO- http://localhost:80/ || exit 1"]
  db:
    image: postgres:16-alpine
    ports:
      - "5432:5432"
    environment:
      POSTGRES_DB: testdb
      POSTGRES_USER: user
    volumes:
      - db-data:/var/lib/postgresql/data
"#;
        let tmp = std::env::temp_dir().join(format!("compose_test_{}.yml", std::process::id()));
        fs::write(&tmp, yaml).unwrap();

        let starlark = convert_compose_to_ved(&tmp, "testapp").unwrap();
        let _ = fs::remove_file(&tmp);

        assert!(starlark.contains("app = App("));
        assert!(starlark.contains("name=\"testapp\""));
        assert!(starlark.contains("name=\"web\""));
        assert!(starlark.contains("image=\"nginx:alpine\""));
        assert!(starlark.contains("Port(80, host=8080)"));
        assert!(starlark.contains("wait_for=[\"db.healthy\"]"));
        assert!(starlark.contains("Health.http(\"/\")"));
        assert!(starlark.contains("name=\"db\""));
        assert!(starlark.contains("image=\"postgres:16-alpine\""));
        assert!(starlark.contains("\"POSTGRES_DB\": \"testdb\""));
        assert!(starlark.contains("Mount.volume(\"db-data\", \"/var/lib/postgresql/data\")"));

        let tmp_ved = std::env::temp_dir().join(format!("Vedfile_compose_{}", std::process::id()));
        fs::write(&tmp_ved, &starlark).unwrap();
        let spec = crate::starlark::load_starlark_spec(&tmp_ved)
            .expect("Failed to evaluate generated Starlark from convert_compose_to_ved");
        let _ = fs::remove_file(&tmp_ved);

        assert_eq!(spec.app, "testapp");
        assert!(spec.services.contains_key("web"));
        assert!(spec.services.contains_key("db"));
    }

    #[test]
    fn test_scaffold_from_dockerfiles() {
        let tmp_dir = std::env::temp_dir().join(format!("df_test_{}", std::process::id()));
        let api_dir = tmp_dir.join("api");
        let _ = fs::create_dir_all(&api_dir);

        fs::write(
            api_dir.join("Dockerfile"),
            "FROM alpine:latest\nEXPOSE 3000\nCMD [\"echo\", \"hi\"]\n",
        )
        .unwrap();

        let (starlark, dfs) = scaffold_from_dockerfiles(&tmp_dir, "my-app").unwrap();
        let _ = fs::remove_dir_all(&tmp_dir);

        assert_eq!(dfs.len(), 1);
        assert_eq!(dfs[0].name, "api");
        assert_eq!(dfs[0].expose_port, Some(3000));
        assert!(starlark.contains("name=\"api\""));
        assert!(starlark.contains("build=\"./api\""));
        assert!(starlark.contains("Port(3000, host=3000)"));

        let tmp_ved = std::env::temp_dir().join(format!("Vedfile_scaffold_{}", std::process::id()));
        fs::write(&tmp_ved, &starlark).unwrap();
        let spec = crate::starlark::load_starlark_spec(&tmp_ved)
            .expect("Failed to evaluate generated Starlark from scaffold_from_dockerfiles");
        let _ = fs::remove_file(&tmp_ved);

        assert_eq!(spec.app, "my-app");
        assert!(spec.services.contains_key("api"));
    }

    #[test]
    fn test_convert_compose_multiline_healthcheck() {
        let yaml = r#"
version: '3.8'
services:
  custom-svc:
    image: custom:latest
    healthcheck:
      test:
        - CMD-SHELL
        - |
          sh -c exec 3<>/dev/tcp/localhost/8310 && printf 'GET /api/v1/healthz HTTP/1.0
          \r\nHost: localhost\r\n\r\n' >&3 && read -t 5 r <&3 && echo $$r |grep -q '200'
"#;
        let tmp =
            std::env::temp_dir().join(format!("compose_multiline_{}.yml", std::process::id()));
        fs::write(&tmp, yaml).unwrap();

        let starlark = convert_compose_to_ved(&tmp, "multiline-app").unwrap();
        let _ = fs::remove_file(&tmp);

        assert!(starlark.contains("Health.exec("));
        assert!(starlark.contains("\"\"\"sh -c exec 3<>/dev/tcp/localhost/8310"));

        let tmp_ved =
            std::env::temp_dir().join(format!("Vedfile_multiline_{}", std::process::id()));
        fs::write(&tmp_ved, &starlark).unwrap();
        let spec = crate::starlark::load_starlark_spec(&tmp_ved)
            .expect("Failed to evaluate generated Starlark with multiline healthcheck");
        let _ = fs::remove_file(&tmp_ved);

        let svc = &spec.services["custom-svc"];
        let health = svc.health.as_ref().and_then(|v| v.as_str()).unwrap();
        assert!(health.contains("sh -c exec 3<>/dev/tcp/localhost/8310"));
    }

    #[test]
    fn test_convert_compose_with_multiple_containers_and_docker_dockerfile() {
        let yaml = r#"
version: '3.8'
services:
  web:
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "8080:80"
  worker:
    build:
      context: .
      dockerfile: docker/Dockerfile
    ports:
      - "9090:9090"
"#;
        let tmp =
            std::env::temp_dir().join(format!("compose_docker_df_{}.yml", std::process::id()));
        fs::write(&tmp, yaml).unwrap();

        let starlark = convert_compose_to_ved(&tmp, "docker-df-app").unwrap();
        let _ = fs::remove_file(&tmp);

        assert!(starlark.contains("name=\"web\""));
        assert!(starlark.contains("build=\".\""));
        assert!(starlark.contains("name=\"worker\""));
        assert!(starlark.contains("build=\".:docker/Dockerfile\""));

        let tmp_ved =
            std::env::temp_dir().join(format!("Vedfile_docker_df_{}", std::process::id()));
        fs::write(&tmp_ved, &starlark).unwrap();
        let spec = crate::starlark::load_starlark_spec(&tmp_ved)
            .expect("Failed to evaluate generated Starlark");
        let _ = fs::remove_file(&tmp_ved);

        assert_eq!(spec.services["web"].build.as_deref(), Some("."));
        assert_eq!(
            spec.services["worker"].build.as_deref(),
            Some(".:docker/Dockerfile")
        );

        let generated_compose =
            crate::compose::generate_compose(&spec).expect("Failed to generate compose from spec");

        let root = std::env::current_dir().unwrap();
        assert!(generated_compose.contains("web:"));
        assert!(generated_compose.contains(&format!("context: {}", root.display())));
        assert!(generated_compose.contains("dockerfile: Dockerfile"));

        assert!(generated_compose.contains("worker:"));
        assert!(generated_compose.contains("dockerfile: docker/Dockerfile"));
    }

    #[test]
    fn test_convert_compose_command_with_line_continuations_and_trailing_backslash() {
        let yaml = r#"
version: '3.8'
services:
  kafka-init:
    image: confluentinc/cp-kafka:latest
    command: |
      /bin/sh -c /opt/kafka/bin/kafka-topics.sh --bootstrap-server kafka:9092 \ 
      --create --if-not-exists etc  with \
"#;
        let tmp = std::env::temp_dir().join(format!("compose_kafka_{}.yml", std::process::id()));
        fs::write(&tmp, yaml).unwrap();

        let starlark = convert_compose_to_ved(&tmp, "kafka-app").unwrap();
        let _ = fs::remove_file(&tmp);

        assert!(!starlark.contains("\\ \n"));
        assert!(!starlark.contains("\\\n"));

        let tmp_ved = std::env::temp_dir().join(format!("Vedfile_kafka_{}", std::process::id()));
        fs::write(&tmp_ved, &starlark).unwrap();
        let spec = crate::starlark::load_starlark_spec(&tmp_ved)
            .expect("Failed to evaluate generated Starlark for kafka command with backslashes");
        let _ = fs::remove_file(&tmp_ved);

        let svc = &spec.services["kafka-init"];
        let cmd = svc.command.as_ref().and_then(|v| v.as_str()).unwrap();
        assert!(cmd.contains("/opt/kafka/bin/kafka-topics.sh --bootstrap-server kafka:9092"));
        assert!(cmd.contains("--create --if-not-exists"));
        assert!(!cmd.ends_with('\\'));

        let generated_compose =
            crate::compose::generate_compose(&spec).expect("Failed to generate compose from spec");
        assert!(generated_compose.contains("/opt/kafka/bin/kafka-topics.sh"));
    }

    #[test]
    fn test_convert_compose_deterministic_sorting() {
        let yaml = r#"
version: '3.8'
services:
  zebra:
    image: alpine:latest
  apple:
    image: alpine:latest
  mango:
    image: alpine:latest
"#;
        let tmp = std::env::temp_dir().join(format!("compose_sort_{}.yml", std::process::id()));
        fs::write(&tmp, yaml).unwrap();

        let starlark = convert_compose_to_ved(&tmp, "sort-app").unwrap();
        let _ = fs::remove_file(&tmp);

        let apple_pos = starlark.find("# Service: apple").unwrap();
        let mango_pos = starlark.find("# Service: mango").unwrap();
        let zebra_pos = starlark.find("# Service: zebra").unwrap();

        assert!(apple_pos < mango_pos);
        assert!(mango_pos < zebra_pos);
    }

    #[test]
    fn test_convert_compose_entrypoint() {
        let yaml = r#"
version: '3.8'
services:
  custom-app:
    image: my-app:latest
    entrypoint: ["/docker-entrypoint.sh", "--mode=prod"]
    command: ["run", "server"]
"#;
        let tmp =
            std::env::temp_dir().join(format!("compose_entrypoint_{}.yml", std::process::id()));
        fs::write(&tmp, yaml).unwrap();

        let starlark = convert_compose_to_ved(&tmp, "entrypoint-app").unwrap();
        let _ = fs::remove_file(&tmp);

        assert!(starlark.contains("entrypoint=[\"/docker-entrypoint.sh\", \"--mode=prod\"]"));
        assert!(starlark.contains("command=[\"run\", \"server\"]"));

        let tmp_ved =
            std::env::temp_dir().join(format!("Vedfile_entrypoint_{}", std::process::id()));
        fs::write(&tmp_ved, &starlark).unwrap();
        let spec = crate::starlark::load_starlark_spec(&tmp_ved)
            .expect("Failed to evaluate generated Starlark with entrypoint");
        let _ = fs::remove_file(&tmp_ved);

        let svc = &spec.services["custom-app"];
        let ep = svc.entrypoint.as_ref().unwrap();
        assert_eq!(
            ep,
            &serde_json::json!(["/docker-entrypoint.sh", "--mode=prod"])
        );
        let cmd = svc.command.as_ref().unwrap();
        assert_eq!(cmd, &serde_json::json!(["run", "server"]));
    }
}
