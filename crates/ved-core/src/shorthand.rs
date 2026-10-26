use indexmap::IndexMap;
use std::path::Path;

use crate::error::{Result, SpecLoadPhase, VedError};
use crate::model::{AppSpec, ServiceSpec};

/// Check if a given file path is a shorthand specification.
pub fn is_shorthand(path: &Path) -> bool {
    if path.extension().and_then(|e| e.to_str()) == Some("ved") {
        return true;
    }
    let mut session = match crate::resources::LoadSession::new(&crate::LoadOptions::default()) {
        Ok(s) => s,
        Err(_) => return false,
    };
    session
        .read_source(path)
        .map(|content| is_shorthand_content(path, &content))
        .unwrap_or(false)
}
pub(crate) fn is_shorthand_content(path: &Path, content: &str) -> bool {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext == "ved" {
        return true;
    }
    if content.contains("from ved import") || content.contains("apiVersion:") {
        return false;
    }
    content.lines().any(|line| {
        let t = line.trim();
        t.starts_with("Service ")
            || t.starts_with("service ")
            || (t.contains(" -> ") && !t.contains("=>"))
            || t.contains(" -- ")
    })
}
pub fn load_shorthand_spec(path: &Path) -> Result<AppSpec> {
    let spec = load_shorthand_spec_wire(path)?;
    spec.validate()
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    Ok(spec)
}
pub(crate) fn load_shorthand_spec_wire(path: &Path) -> Result<AppSpec> {
    let context = crate::WorkspaceContext::new(path.parent().unwrap_or(Path::new(".")));
    let mut session =
        crate::resources::LoadSession::new(&crate::LoadOptions::from_config(context.config())?)?;
    let content = session.read_source(path)?;
    let mut spec = parse_source_in_session(path, &content, &mut session)?;
    crate::stacks::apply_stacks_with_context_in_session(
        &spec.stacks.clone(),
        &mut spec,
        &context,
        &mut session,
    )
    .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Evaluate, e))?;
    Ok(spec)
}
// Caller has already charged the bounded source bytes to this session.
pub(crate) fn parse_source_in_session(
    path: &Path,
    content: &str,
    session: &mut crate::resources::LoadSession,
) -> Result<AppSpec> {
    session.check_elapsed(path)?;
    let mut spec = parse_shorthand_unchecked(content)
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Parse, e))?;
    if spec.app == "app" {
        if let Some(folder) = path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        {
            if !folder.is_empty() && folder != "/" {
                spec.app = folder;
            }
        }
    }
    spec.validate_identifiers()
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    session.check_elapsed(path)?;
    Ok(spec)
}

/// Parse shorthand specification text into AppSpec.
pub fn parse_shorthand(input: &str) -> Result<AppSpec> {
    let mut session = crate::resources::LoadSession::new(&crate::LoadOptions::default())?;
    session.check_source(Path::new("<shorthand>"), input.as_bytes())?;
    let result = parse_shorthand_unchecked(input)?;
    session.check_elapsed(Path::new("<shorthand>"))?;
    Ok(result)
}
fn parse_shorthand_unchecked(input: &str) -> Result<AppSpec> {
    let mut app_name = "app".to_string();
    let mut version = "1.0.0".to_string();
    let mut network = "default".to_string();
    let mut services: IndexMap<String, ServiceSpec> = IndexMap::new();
    let mut stacks: Vec<String> = Vec::new();

    for (idx, raw_line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let mut line = raw_line.trim();

        // Strip inline comments if preceded by whitespace
        if let Some(pos) = line.find(" #") {
            line = line[..pos].trim();
        }
        if let Some(pos) = line.find(" //") {
            line = line[..pos].trim();
        }

        if line.starts_with("\"\"\"") {
            line = line[3..].trim();
        }
        if line.ends_with("\"\"\"") && line.len() >= 3 {
            line = line[..line.len() - 3].trim();
        }
        if line.starts_with("'''") {
            line = line[3..].trim();
        }
        if line.ends_with("'''") && line.len() >= 3 {
            line = line[..line.len() - 3].trim();
        }

        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("//")
            || line.starts_with(';')
        {
            continue;
        }

        // App <name> [<version>]
        if line.starts_with("App ") || line.starts_with("app ") {
            let mut parts = line.split_whitespace();
            let _tag = parts.next();
            if let Some(name) = parts.next() {
                app_name = name.to_string();
            }
            if let Some(ver) = parts.next() {
                version = ver.to_string();
            }
            continue;
        }

        // Network <name>
        if line.starts_with("Network ") || line.starts_with("network ") {
            let mut parts = line.split_whitespace();
            let _tag = parts.next();
            if let Some(net) = parts.next() {
                network = net.to_string();
            }
            continue;
        }

        // Stack <name>
        if line.starts_with("Stack ") || line.starts_with("stack ") {
            let mut parts = line.split_whitespace();
            let _tag = parts.next();
            if let Some(stk) = parts.next() {
                stacks.push(stk.to_string());
            }
            continue;
        }

        // Service <name> <image> [<port>]
        if line.starts_with("Service ") || line.starts_with("service ") {
            let mut parts = line.split_whitespace();
            let _tag = parts.next();
            let name = match parts.next() {
                Some(n) => n,
                None => {
                    return Err(VedError::shorthand_syntax(
                        line_no,
                        format!("'Service' requires at least <name> and <image>, found '{line}'"),
                    ));
                }
            };
            let image = match parts.next() {
                Some(img) => img,
                None => {
                    return Err(VedError::shorthand_syntax(
                        line_no,
                        format!("'Service' requires at least <name> and <image>, found '{line}'"),
                    ));
                }
            };
            let svc = get_or_create_service(&mut services, name);
            svc.image = Some(image.to_string());

            if let Some(port_str) = parts.next() {
                apply_port(svc, port_str, line_no)?;
            }
            continue;
        }

        // Environment variables: <name>= (...) or <name> = (...)
        if let Some(open_idx) = line.find('(') {
            let left = line[..open_idx].trim();
            if left.ends_with('=') && line.ends_with(')') {
                let name = left.trim_end_matches('=').trim();
                if !name.is_empty() {
                    let inside = &line[open_idx + 1..line.len() - 1];
                    let svc = get_or_create_service(&mut services, name);
                    parse_env_pairs(&mut svc.env, inside)?;
                    continue;
                }
            }
        }

        // Dependency: <name1> -> <name2> [, <name3> ...]
        if line.contains("->") {
            let mut arrow_iter = line.split("->").map(|s| s.trim());
            let source = arrow_iter.next().unwrap_or_default();
            if source.is_empty() {
                return Err(VedError::shorthand_syntax(
                    line_no,
                    "missing source service before '->'",
                ));
            }
            let mut targets = Vec::new();
            for target_part in arrow_iter {
                for target in target_part.split(',') {
                    let t = target.trim();
                    if !t.is_empty() {
                        targets.push(t);
                    }
                }
            }
            for &t in &targets {
                let _ = get_or_create_service(&mut services, t);
            }
            let svc = get_or_create_service(&mut services, source);
            for t in targets {
                if !svc.wait_for.iter().any(|e| e == t) {
                    svc.wait_for.push(t.to_string());
                }
            }
            continue;
        }

        // Health check: <name> -- <health>
        if let Some((name_part, health_part)) = line.split_once("--") {
            let name = name_part.trim();
            let health_target = health_part.trim();
            if !name.is_empty() && !health_target.is_empty() {
                let svc = get_or_create_service(&mut services, name);
                svc.health = Some(serde_json::Value::String(health_target.to_string()));
                continue;
            }
        }

        // Profile / Group: <group> (<s1>, <s2>, ...)
        if line.ends_with(')') {
            if let Some(open_idx) = line.find('(') {
                let group_name = line[..open_idx].trim();
                let inside = &line[open_idx + 1..line.len() - 1];
                if is_valid_identifier(group_name) {
                    for target in inside.split(',') {
                        let name = target.trim();
                        if !name.is_empty() {
                            let svc = get_or_create_service(&mut services, name);
                            svc.group = group_name.to_string();
                        }
                    }
                    continue;
                }
            }
        }

        return Err(VedError::shorthand_syntax(
            line_no,
            format!("unrecognized shorthand syntax: '{line}'"),
        ));
    }

    Ok(AppSpec {
        app: app_name,
        version,
        network,
        ingress: None,
        config_dir: ".".to_string(),
        env_files: vec![
            crate::config::ENV_FILE.to_string(),
            crate::config::LOCAL_ENV_FILE.to_string(),
        ],
        services,
        jobs: IndexMap::new(),
        profiles: IndexMap::new(),
        tasks: IndexMap::new(),
        stacks,
        networks: IndexMap::new(),
    })
}

/// Check if the file content contains only comments (line comments `#` and/or triple-quoted docstrings)
/// with no other code statements.
pub fn is_only_comments(content: &str) -> bool {
    let mut in_docstring = false;
    let mut quote_marker = "\"\"\"";
    let mut has_docstring = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        if !in_docstring {
            if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
                has_docstring = true;
                let marker = if trimmed.starts_with("\"\"\"") {
                    "\"\"\""
                } else {
                    "'''"
                };
                let rest = &trimmed[3..];
                if let Some(close_idx) = rest.find(marker) {
                    let after = rest[close_idx + 3..].trim();
                    if !after.is_empty() && !after.starts_with('#') && !after.starts_with("//") {
                        return false;
                    }
                    continue;
                } else {
                    in_docstring = true;
                    quote_marker = marker;
                    continue;
                }
            }
            // Non-comment statement outside docstring
            return false;
        } else if let Some(pos) = trimmed.find(quote_marker) {
            in_docstring = false;
            let after = trimmed[pos + 3..].trim();
            if !after.is_empty() && !after.starts_with('#') && !after.starts_with("//") {
                return false;
            }
        }
    }

    has_docstring && !in_docstring
}

/// Convert shorthand text into Starlark Vedfile code.
/// If the input is wrapped in triple quotes, the original docstring comments are preserved at the top.
pub fn shorthand_to_starlark(input: &str) -> Result<String> {
    let spec = parse_shorthand(input)?;
    let mut out = String::new();
    let trimmed = input.trim();

    if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
        out.push_str(trimmed);
        out.push_str("\n\n");
    } else {
        out.push_str("\"\"\"Vedfile — generated from shorthand specification.\"\"\"\n\n");
    }
    out.push_str(&format_starlark_body(&spec));
    Ok(out)
}

/// Format AppSpec model into Starlark application definition.
pub fn format_starlark_body(spec: &AppSpec) -> String {
    let mut out = String::new();
    out.push_str("from ved import App, Service\n\n");
    out.push_str(&format!(
        "app = App(\n    name = \"{}\",\n    version = \"{}\",\n    network = \"{}\",\n)\n\n",
        spec.app, spec.version, spec.network
    ));

    if !spec.services.is_empty() {
        out.push_str("app.services.add(\n");
        for (name, svc) in &spec.services {
            out.push_str("    Service(\n");
            out.push_str(&format!("        name = \"{name}\",\n"));
            if let Some(ref img) = svc.image {
                out.push_str(&format!("        image = \"{img}\",\n"));
            }
            if svc.group != "default" {
                out.push_str(&format!("        group = \"{}\",\n", svc.group));
            }
            if !svc.ports.is_empty() {
                out.push_str("        ports = [\n");
                for p in &svc.ports {
                    if let Some(s) = p.as_str() {
                        out.push_str(&format!("            \"{s}\",\n"));
                    } else {
                        out.push_str(&format!("            {p},\n"));
                    }
                }
                out.push_str("        ],\n");
            }
            if !svc.wait_for.is_empty() {
                out.push_str("        wait_for = [\n");
                for dep in &svc.wait_for {
                    out.push_str(&format!("            \"{dep}\",\n"));
                }
                out.push_str("        ],\n");
            }
            if let Some(h) = svc.health_value() {
                if let Some(cmd) = h.as_str() {
                    out.push_str(&format!("        health = \"{cmd}\",\n"));
                }
            }
            if let Some(ref ep) = svc.entrypoint {
                out.push_str(&format!("        entrypoint = {ep},\n"));
            }
            if let Some(ref cmd) = svc.command {
                out.push_str(&format!("        command = {cmd},\n"));
            }
            if !svc.env.is_empty() {
                out.push_str("        env = {\n");
                for (k, v) in &svc.env {
                    if let Some(s) = v.as_str() {
                        out.push_str(&format!("            \"{k}\": \"{s}\",\n"));
                    } else {
                        out.push_str(&format!("            \"{k}\": {v},\n"));
                    }
                }
                out.push_str("        },\n");
            }
            out.push_str("    ),\n");
        }
        out.push_str(")\n");
    }

    out
}

fn get_or_create_service<'a>(
    services: &'a mut IndexMap<String, ServiceSpec>,
    name: &str,
) -> &'a mut ServiceSpec {
    if !services.contains_key(name) {
        services.insert(name.to_string(), ServiceSpec::default());
    }
    services.get_mut(name).expect("service exists")
}

fn apply_port(svc: &mut ServiceSpec, port_str: &str, line_no: usize) -> Result<()> {
    if let Some((_host, container)) = port_str.split_once(':') {
        if !container.contains(':') {
            let container_port: u16 = container.parse().map_err(|e| {
                VedError::shorthand_syntax(
                    line_no,
                    format!("invalid container port in '{port_str}': {e}"),
                )
            })?;
            svc.port = Some(container_port);
            svc.ports = vec![serde_json::Value::String(port_str.to_string())];
            return Ok(());
        }
    }
    let p: u16 = port_str.parse().map_err(|e| {
        VedError::shorthand_syntax(line_no, format!("invalid port number '{port_str}': {e}"))
    })?;
    svc.port = Some(p);
    svc.ports = vec![serde_json::Value::String(format!("{p}:{p}"))];
    Ok(())
}

fn parse_env_pairs(env: &mut IndexMap<String, serde_json::Value>, s: &str) -> Result<()> {
    let mut in_quotes = false;
    let mut quote_char = '"';
    let mut start = 0;

    let insert_pair = |env: &mut IndexMap<String, serde_json::Value>, pair: &str| {
        if let Some((k_part, v_part)) = pair.split_once('=') {
            let k = k_part.trim();
            let mut v = v_part.trim();
            if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
                || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
            {
                v = &v[1..v.len() - 1];
            }
            if !k.is_empty() {
                env.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
        }
    };

    for (i, c) in s.char_indices() {
        match c {
            '"' | '\'' if !in_quotes => {
                in_quotes = true;
                quote_char = c;
            }
            c if in_quotes && c == quote_char => {
                in_quotes = false;
            }
            ',' if !in_quotes => {
                let pair = s[start..i].trim();
                if !pair.is_empty() {
                    insert_pair(env, pair);
                }
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let pair = s[start..].trim();
    if !pair.is_empty() {
        insert_pair(env, pair);
    }

    Ok(())
}

fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_services_and_ports() {
        let input = r#"
            Service web nginx:alpine 8080:80
            Service db postgres:16 5432
            Service redis redis:alpine
        "#;
        let spec = parse_shorthand(input).expect("parse failed");
        assert_eq!(spec.services.len(), 3);

        let web = &spec.services["web"];
        assert_eq!(web.image.as_deref(), Some("nginx:alpine"));
        assert_eq!(web.port, Some(80));
        assert_eq!(web.ports, vec![json!("8080:80")]);

        let db = &spec.services["db"];
        assert_eq!(db.image.as_deref(), Some("postgres:16"));
        assert_eq!(db.port, Some(5432));
        assert_eq!(db.ports, vec![json!("5432:5432")]);

        let redis = &spec.services["redis"];
        assert_eq!(redis.image.as_deref(), Some("redis:alpine"));
        assert_eq!(redis.port, None);
    }

    #[test]
    fn test_parse_dependencies() {
        let input = r#"
            Service web nginx:alpine
            Service db postgres:16
            Service redis redis:alpine
            web -> db, redis
        "#;
        let spec = parse_shorthand(input).expect("parse failed");
        assert_eq!(spec.services["web"].wait_for, vec!["db", "redis"]);
    }

    #[test]
    fn test_parse_profiles_and_health() {
        let input = r#"
            Service web nginx:alpine 8080:80
            Service db postgres:16 5432
            core (web, db)
            web -- http://localhost:8080/health
            db -- pg_isready -U postgres
        "#;
        let spec = parse_shorthand(input).expect("parse failed");
        assert_eq!(spec.services["web"].group, "core");
        assert_eq!(spec.services["db"].group, "core");
        assert_eq!(
            spec.services["web"].health,
            Some(json!("http://localhost:8080/health"))
        );
        assert_eq!(
            spec.services["db"].health,
            Some(json!("pg_isready -U postgres"))
        );
    }

    #[test]
    fn test_parse_env_vars() {
        let input = r#"
            Service db postgres:16
            db= (POSTGRES_DB=app, POSTGRES_PASSWORD="secret,with,comma", POSTGRES_USER='admin')
        "#;
        let spec = parse_shorthand(input).expect("parse failed");
        let env = &spec.services["db"].env;
        assert_eq!(env.get("POSTGRES_DB"), Some(&json!("app")));
        assert_eq!(
            env.get("POSTGRES_PASSWORD"),
            Some(&json!("secret,with,comma"))
        );
        assert_eq!(env.get("POSTGRES_USER"), Some(&json!("admin")));
    }

    #[test]
    fn test_full_grammar() {
        let input = r#"
            # Multi-service stack
            App my-shop 2.1.0
            Network custom-net

            Service web nginx:alpine 8080:80
            Service db postgres:16 5432
            Service cache redis:alpine 6379

            web -> db, cache
            core (web, db)
            tools (cache)

            web -- http://localhost:8080/health
            db -- pg_isready -U postgres
            cache -- tcp:6379

            db= (POSTGRES_DB=shop, POSTGRES_PASSWORD=secret)
            web= (NODE_ENV=production, API_URL="http://localhost:3000")
        "#;
        let spec = parse_shorthand(input).expect("parse failed");
        assert_eq!(spec.app, "my-shop");
        assert_eq!(spec.version, "2.1.0");
        assert_eq!(spec.network, "custom-net");
        assert_eq!(spec.services.len(), 3);
        assert_eq!(spec.services["web"].wait_for, vec!["db", "cache"]);
        assert_eq!(spec.services["web"].group, "core");
        assert_eq!(spec.services["cache"].group, "tools");
        assert_eq!(spec.services["cache"].health, Some(json!("tcp:6379")));
    }

    #[test]
    fn test_shorthand_to_starlark() {
        let input = r#"
            App demo 1.0.0
            Service web nginx:alpine 8080:80
            Service db postgres:16 5432
            web -> db
            core (web, db)
            db= (POSTGRES_DB=test)
        "#;
        let starlark = shorthand_to_starlark(input).expect("starlark conversion failed");
        assert!(starlark.contains("from ved import App, Service"));
        assert!(starlark.contains("name = \"web\""));
        assert!(starlark.contains("group = \"core\""));
        assert!(starlark.contains("wait_for = ["));
        assert!(starlark.contains("\"POSTGRES_DB\": \"test\""));
    }

    #[test]
    fn test_invalid_syntax_reports_line() {
        let input = "Service web\n??? invalid line here";
        let err = parse_shorthand(input).unwrap_err();
        assert!(err
            .to_string()
            .contains("line 1: 'Service' requires at least <name> and <image>"));
        match err {
            VedError::ShorthandSyntax { line, ref message } => {
                assert_eq!(line, 1);
                assert!(message.contains("'Service' requires at least <name> and <image>"));
            }
            other => panic!("expected ShorthandSyntax error, got: {other:?}"),
        }
    }

    #[test]
    fn test_is_only_comments() {
        let only_docstring = "\"\"\"\nApp demo 1.0.0\nService web nginx:alpine\n\"\"\"";
        assert!(is_only_comments(only_docstring));

        let with_comments = "# Header\n\"\"\"\nService web nginx:alpine\n\"\"\"\n# Footer";
        assert!(is_only_comments(with_comments));

        let with_code =
            "\"\"\"\nService web nginx:alpine\n\"\"\"\nfrom ved import App\napp = App('a', '1')";
        assert!(!is_only_comments(with_code));

        let empty = "   \n\n  ";
        assert!(!is_only_comments(empty));

        let regular_starlark = "\"\"\"Docstring\"\"\"\nfrom ved import App, Service";
        assert!(!is_only_comments(regular_starlark));
    }

    #[test]
    fn test_shorthand_preserves_docstring_comments() {
        let input =
            "\"\"\"\nApp demo 1.0.0\n# User comment\nService web nginx:alpine 8080:80\n\"\"\"";
        let out = shorthand_to_starlark(input).unwrap();
        assert!(out.starts_with(
            "\"\"\"\nApp demo 1.0.0\n# User comment\nService web nginx:alpine 8080:80\n\"\"\""
        ));
        assert!(out.contains("from ved import App, Service"));
        assert!(out.contains("name = \"web\""));
    }
}
