use std::{collections::BTreeMap, path::PathBuf};
use ved_core::{
    AppSpec, CheckedPortSpec, CompilationTarget, ConfigSnapshot, PortSpec, PureCompiler,
    ServiceSpec, ValidatedSpec, WorkspaceContext,
};

#[cfg(feature = "host")]
fn build_api_example(context: &WorkspaceContext) -> ved_core::Result<()> {
    use ved_core::{
        build::{build_workspace, BuildOptions},
        SilentSink,
    };
    let options = BuildOptions::new()
        .module("api")
        .concurrency(2)?
        .timeout(std::time::Duration::from_secs(30))?;
    build_workspace(context, context.root(), &options, &SilentSink)
}

fn main() -> ved_core::Result<()> {
    #[cfg(feature = "host")]
    {
        // Compile the downstream build call without executing host tools.
        let _example: fn(&WorkspaceContext) -> ved_core::Result<()> = build_api_example;
        let options = ved_core::build::BuildOptions::new().concurrency(2)?;
        assert_eq!(options.concurrency_limit(), 2);
        assert!(ved_core::build::BuildOptions::new().concurrency(0).is_err());
    }
    let root = PathBuf::from(if cfg!(windows) {
        r"C:\explicit\workspace"
    } else {
        "/explicit/workspace"
    });
    let context = WorkspaceContext::from_explicit_paths(
        &root,
        root.join("config"),
        ConfigSnapshot::from_values(BTreeMap::from([
            ("K8S_APP_SLOT".into(), "7".into()),
            ("INGRESS_SUFFIX".into(), "example.test".into()),
        ])),
    )?;
    let checked = CheckedPortSpec::new(9000, 8080).expect("valid consumer ports");
    let legacy: PortSpec = checked.into();
    assert_eq!(CheckedPortSpec::try_from(legacy).unwrap(), checked);
    assert!(CheckedPortSpec::single(0).is_err());
    let spec = ValidatedSpec::try_from(AppSpec {
        app: "consumer".into(),
        version: "1".into(),
        services: [(
            "api".into(),
            ServiceSpec {
                image: Some("api:v1".into()),
                ports: vec![legacy.into()],
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    })?;
    assert_eq!(
        spec.workload(None, "api").unwrap().checked_ports(),
        &[checked]
    );
    let mut invalid = spec.as_spec().clone();
    invalid.services["api"].port = Some(0);
    let error = ValidatedSpec::try_from(invalid).unwrap_err();
    let reason = error
        .validation_error()
        .expect("structured downstream validation");
    assert_eq!(reason.reason(), ved_core::ValidationReason::InvalidPort);
    assert_eq!(reason.field(), "port");
    assert!(error.is_validation_error());
    let bundle = PureCompiler::new(context)?.compile(&spec, CompilationTarget::All)?;
    for (path, bytes) in bundle.files() {
        print!("{}\t", path.display());
        for byte in bytes {
            print!("{byte:02x}");
        }
        println!();
    }
    Ok(())
}
