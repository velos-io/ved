use crate::error::{Result, SpecLoadPhase, VedError};
use crate::model::AppSpec;
use crate::resources::{LoadOptions, LoadSession};
use regex::Regex;
use std::{
    env, fs,
    path::{Path, PathBuf},
};
static ENV_VAR_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)(?::-([^}]*))?\}").expect("valid env var regex")
});
/// A parsed specification and observed source dependencies, with unpublished assets.
#[derive(Debug, Clone)]
pub struct LoadedSpec {
    pub spec: AppSpec,
    pub dependencies: Vec<crate::stacks::LoadedDependency>,
    pub stack_assets: Vec<crate::stacks::StackAsset>,
}

/// Fluent builder for configuring specification loading with explicit context and resource options.
#[derive(Debug, Clone, Default)]
pub struct SpecLoader<'a> {
    context: Option<&'a crate::WorkspaceContext>,
    options: Option<&'a LoadOptions>,
}

impl<'a> SpecLoader<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn context(mut self, context: &'a crate::WorkspaceContext) -> Self {
        self.context = Some(context);
        self
    }

    pub fn options(mut self, options: &'a LoadOptions) -> Self {
        self.options = Some(options);
        self
    }

    pub fn load(&self, path: &Path) -> Result<AppSpec> {
        let default_ctx;
        let context = match self.context {
            Some(ctx) => ctx,
            None => {
                default_ctx = context_for(path);
                &default_ctx
            }
        };
        let default_opt;
        let options = match self.options {
            Some(opt) => opt,
            None => {
                default_opt = LoadOptions::from_config(context.config())?;
                &default_opt
            }
        };
        load_any_spec_with_context_and_options(path, context, options)
    }

    pub fn load_validated(&self, path: &Path) -> Result<crate::ValidatedSpec> {
        let default_ctx;
        let context = match self.context {
            Some(ctx) => ctx,
            None => {
                default_ctx = context_for(path);
                &default_ctx
            }
        };
        let default_opt;
        let options = match self.options {
            Some(opt) => opt,
            None => {
                default_opt = LoadOptions::from_config(context.config())?;
                &default_opt
            }
        };
        load_validated_spec_with_context_and_options(path, context, options)
    }

    pub fn load_with_dependencies(&self, path: &Path) -> Result<LoadedSpec> {
        let default_ctx;
        let context = match self.context {
            Some(ctx) => ctx,
            None => {
                default_ctx = context_for(path);
                &default_ctx
            }
        };
        let default_opt;
        let options = match self.options {
            Some(opt) => opt,
            None => {
                default_opt = LoadOptions::from_config(context.config())?;
                &default_opt
            }
        };
        load_any_spec_with_context_and_dependencies_and_options(path, context, options)
    }
}

pub fn load_any_spec_with_context_and_dependencies(
    path: &Path,
    context: &crate::WorkspaceContext,
) -> Result<LoadedSpec> {
    SpecLoader::new()
        .context(context)
        .load_with_dependencies(path)
}
pub fn load_any_spec_with_context_and_dependencies_and_options(
    path: &Path,
    context: &crate::WorkspaceContext,
    options: &LoadOptions,
) -> Result<LoadedSpec> {
    let mut session = LoadSession::new(options)?;
    let content = session.read_source(path)?;
    let (mut spec, mut dependencies, stack_assets) = if is_starlark(path, &content) {
        let loaded =
            crate::starlark::load_starlark_spec_wire_with_context_and_dependencies_in_session(
                path,
                context,
                &content,
                &mut session,
            )?;
        (loaded.spec, loaded.dependencies, loaded.assets)
    } else {
        let spec = if crate::shorthand::is_shorthand_content(path, &content) {
            crate::shorthand::parse_source_in_session(path, &content, &mut session)?
        } else {
            parse_yaml(path, &content, context, &session)?
        };
        let applied = crate::stacks::load_stacks_with_context_in_session(
            &spec.stacks.clone(),
            &spec,
            context,
            &mut session,
        )
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Evaluate, e))?;
        (applied.spec, applied.dependencies, applied.assets)
    };
    dependencies.push(crate::stacks::LoadedDependency {
        path: canonical_dependency(path),
        bytes: content.into_bytes(),
    });
    for input in [
        context.locator().env_file(),
        context.locator().local_env_file(),
    ] {
        // Configuration capture predates loading; these bounded reads collect fingerprint inputs.
        match fs::File::open(&input) {
            Ok(_) => {
                let content = session.read_source(&input)?;
                dependencies.push(crate::stacks::LoadedDependency {
                    path: canonical_dependency(&input),
                    bytes: content.into_bytes(),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(VedError::spec_load(&input, SpecLoadPhase::Read, e)),
        }
    }
    let snapshot = context.config().effective_environment();
    let bytes = serde_json::to_vec(&snapshot)
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Convert, e))?;
    dependencies.push(crate::stacks::LoadedDependency {
        path: PathBuf::from("<config-snapshot>"),
        bytes,
    });
    dependencies.sort_by(|a, b| a.path.cmp(&b.path));
    dependencies.dedup_by(|a, b| a.path == b.path && a.bytes == b.bytes);
    spec = context.prepare_spec(&spec);
    spec.validate()
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    session.check_elapsed(path)?;
    Ok(LoadedSpec {
        spec,
        dependencies,
        stack_assets,
    })
}
fn canonical_dependency(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
fn is_starlark(path: &Path, content: &str) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    content.contains("from ved import")
        || content.contains("app = App(")
        || name == crate::r::files::VEDFILE
        || name.ends_with(".star")
        || name.ends_with(".py")
}
fn parse_yaml(
    path: &Path,
    content: &str,
    context: &crate::WorkspaceContext,
    session: &LoadSession,
) -> Result<AppSpec> {
    let expanded = expand_env_vars_with(content, |key| context.config().interpolation_value(key));
    session.check_representation(path, expanded.as_bytes())?;
    serde_yaml::from_str(&expanded).map_err(|e| VedError::spec_load(path, SpecLoadPhase::Parse, e))
}
pub fn expand_env_vars_with(raw: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    ENV_VAR_RE
        .replace_all(raw, |caps: &regex::Captures| {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            lookup(name).unwrap_or_else(|| {
                caps.get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| format!("${{{name}}}"))
            })
        })
        .to_string()
}
pub fn expand_env_vars(raw: &str) -> String {
    expand_env_vars_with(raw, |name| env::var(name).ok())
}
fn context_for(path: &Path) -> crate::WorkspaceContext {
    crate::WorkspaceContext::new(path.parent().unwrap_or(Path::new(".")))
}
pub fn load_any_spec(path: &Path) -> Result<AppSpec> {
    SpecLoader::new().load(path)
}
pub fn load_validated_spec(path: &Path) -> Result<crate::ValidatedSpec> {
    SpecLoader::new().load_validated(path)
}
pub fn load_any_spec_with_context(
    path: &Path,
    context: &crate::WorkspaceContext,
) -> Result<AppSpec> {
    SpecLoader::new().context(context).load(path)
}
pub fn load_any_spec_with_context_and_options(
    path: &Path,
    context: &crate::WorkspaceContext,
    options: &LoadOptions,
) -> Result<AppSpec> {
    let mut session = LoadSession::new(options)?;
    let spec = context.prepare_spec(&load_any_wire_in_session(path, context, &mut session)?);
    spec.validate()
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    session.check_elapsed(path)?;
    Ok(spec)
}
pub fn load_validated_spec_with_context(
    path: &Path,
    context: &crate::WorkspaceContext,
) -> Result<crate::ValidatedSpec> {
    SpecLoader::new().context(context).load_validated(path)
}
pub fn load_validated_spec_with_context_and_options(
    path: &Path,
    context: &crate::WorkspaceContext,
    options: &LoadOptions,
) -> Result<crate::ValidatedSpec> {
    let mut session = LoadSession::new(options)?;
    let wire = context.prepare_spec(&load_any_wire_in_session(path, context, &mut session)?);
    let spec = crate::ValidatedSpec::try_from(wire)
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    session.check_elapsed(path)?;
    Ok(spec)
}
fn load_any_wire_in_session(
    path: &Path,
    context: &crate::WorkspaceContext,
    session: &mut LoadSession,
) -> Result<AppSpec> {
    let content = session.read_source(path)?;
    if is_starlark(path, &content) {
        crate::starlark::load_starlark_spec_wire_from_source_in_session(
            path, context, &content, session,
        )
    } else if crate::shorthand::is_shorthand_content(path, &content) {
        let mut spec = crate::shorthand::parse_source_in_session(path, &content, session)?;
        crate::stacks::apply_stacks_with_context_in_session(
            &spec.stacks.clone(),
            &mut spec,
            context,
            session,
        )
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Evaluate, e))?;
        Ok(spec)
    } else {
        parse_yaml(path, &content, context, session)
    }
}
pub fn load_spec(path: &Path) -> Result<AppSpec> {
    let context = context_for(path);
    let mut session = LoadSession::new(&LoadOptions::from_config(context.config())?)?;
    let content = session.read_source(path)?;
    let spec = parse_yaml(path, &content, &context, &session)?;
    spec.validate()
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    session.check_elapsed(path)?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_env_vars() {
        let input = "host: ${TEST_DB_HOST}\npass: ${NON_EXISTENT_VAR:-default_pass}";
        let lookup = |name: &str| -> Option<String> {
            match name {
                "TEST_DB_HOST" => Some("my-postgres".to_string()),
                _ => None,
            }
        };
        let output = expand_env_vars_with(input, lookup);
        assert!(output.contains("host: my-postgres"));
        assert!(output.contains("pass: default_pass"));
    }

    #[test]
    fn test_load_spec_validates_mounts() {
        let temp_dir =
            std::env::temp_dir().join(format!("ved-test-parser-mounts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let spec_file = temp_dir.join(crate::r::files::VED_YAML);
        let invalid_yaml = r#"
app: myapp
version: 1.0.0
services:
  web:
    image: nginx
    mounts:
      - bad-mount-syntax
"#;
        fs::write(&spec_file, invalid_yaml).unwrap();
        let res = load_spec(&spec_file);
        let _ = fs::remove_dir_all(&temp_dir);
        assert!(res.is_err());
        let error = res.unwrap_err();
        let load = error.spec_load_error().expect("file context retained");
        assert_eq!(load.phase(), SpecLoadPhase::Validate);
        assert_eq!(load.path(), spec_file);
        assert!(error.validation_error().is_some());
        assert!(!error.to_string().contains("bad-mount-syntax"));
    }

    #[test]
    fn dependency_load_returns_exact_spec_and_config_inputs_without_workdir() {
        let root =
            std::env::temp_dir().join(format!("ved-parser-dependencies-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("config")).unwrap();
        let spec_path = root.join("ved.yaml");
        let spec_bytes = b"app: sample\nversion: 1.0.0\nservices:\n  web:\n    image: nginx\n";
        fs::write(&spec_path, spec_bytes).unwrap();
        let env_bytes = b"IMAGE=nginx\n";
        fs::write(root.join("config/.env"), env_bytes).unwrap();
        let context = crate::WorkspaceContext::new(root.clone());

        let loaded = load_any_spec_with_context_and_dependencies(&spec_path, &context).unwrap();
        assert_eq!(loaded.spec.app, "sample");
        assert!(loaded.spec.services.contains_key("web"));
        assert!(!root.join(".ved").exists());
        assert_eq!(
            loaded
                .dependencies
                .iter()
                .find(|d| d.path == spec_path.canonicalize().unwrap())
                .unwrap()
                .bytes,
            spec_bytes
        );
        assert_eq!(
            loaded
                .dependencies
                .iter()
                .find(|d| d.path.ends_with("config/.env"))
                .unwrap()
                .bytes,
            env_bytes
        );
        assert!(loaded
            .dependencies
            .iter()
            .any(|d| d.path == Path::new("<config-snapshot>")));
        let shorthand_path = root.join("app.ved");
        fs::write(
            &shorthand_path,
            "App short-app 1.0.0\nService api nginx 8080\n",
        )
        .unwrap();
        let shorthand =
            load_any_spec_with_context_and_dependencies(&shorthand_path, &context).unwrap();
        assert_eq!(shorthand.spec.app, "short-app");
        assert!(shorthand.spec.services.contains_key("api"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn spec_loader_builder_supports_all_permutations() {
        let root =
            std::env::temp_dir().join(format!("ved-spec-loader-builder-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let spec_path = root.join("ved.yaml");
        fs::write(
            &spec_path,
            "app: builder-test\nversion: '1.0'\nservices:\n  web:\n    image: nginx\n",
        )
        .unwrap();
        let context = crate::WorkspaceContext::new(root.clone());
        let options = LoadOptions::default();

        let app_spec = SpecLoader::new()
            .context(&context)
            .options(&options)
            .load(&spec_path)
            .unwrap();
        assert_eq!(app_spec.app, "builder-test");

        let validated = SpecLoader::new()
            .context(&context)
            .load_validated(&spec_path)
            .unwrap();
        assert_eq!(validated.as_spec().app, "builder-test");

        let loaded = SpecLoader::new()
            .load_with_dependencies(&spec_path)
            .unwrap();
        assert_eq!(loaded.spec.app, "builder-test");
        assert!(!loaded.dependencies.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
