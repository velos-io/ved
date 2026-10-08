#![cfg(feature = "host")]
use std::fs;
use ved_core::{
    generate_compose, generate_k8s, load_shorthand_spec, load_starlark_spec, parse_shorthand,
    shorthand_to_starlark,
};

#[test]
fn test_shorthand_end_to_end_compose() {
    let shorthand = r#"
        App e2e-stack 1.2.0
        Network internal-mesh

        Service web nginx:alpine 8080:80
        Service api node:20-alpine 3000
        Service db postgres:16 5432

        web -> api
        api -> db

        core (web, api)
        infra (db)

        web -- http://localhost:8080/health
        db -- pg_isready -U postgres

        db= (POSTGRES_DB=app_db, POSTGRES_PASSWORD="secure_pass")
        api= (NODE_ENV=production, DB_HOST=db)
    "#;

    let spec = parse_shorthand(shorthand).expect("Failed to parse shorthand");
    assert_eq!(spec.app, "e2e-stack");
    assert_eq!(spec.version, "1.2.0");
    assert_eq!(spec.network, "internal-mesh");
    assert_eq!(spec.services.len(), 3);

    let compose_yaml = generate_compose(&spec).expect("Failed to generate compose");
    assert!(compose_yaml.contains("image: nginx:alpine"));
    assert!(compose_yaml.contains("image: node:20-alpine"));
    assert!(compose_yaml.contains("image: postgres:16"));
    assert!(compose_yaml.contains("internal-mesh:"));
    assert!(compose_yaml.contains("POSTGRES_DB: app_db"));
    assert!(compose_yaml.contains("NODE_ENV: production"));
    assert!(compose_yaml.contains("service_healthy"));

    let tmp = std::env::temp_dir().join(format!("ved-test-k8s-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    generate_k8s(&spec, &tmp).expect("Failed to generate k8s manifests");
    let base_infra = tmp.join("base/infra/kustomization.yaml");
    assert!(base_infra.exists(), "K8s base infra kustomization missing");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_shorthand_to_starlark_evaluation_parity() {
    let shorthand = r#"
        App parity-stack 1.0.0
        Service web nginx:alpine 8080:80
        Service db postgres:16 5432
        web -> db
        core (web, db)
        db= (POSTGRES_DB=mydb)
    "#;

    let starlark_code = shorthand_to_starlark(shorthand).expect("Failed to generate starlark");
    let tmp = std::env::temp_dir().join(format!("ved-test-parity-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let vedfile_path = tmp.join("Vedfile");
    fs::write(&vedfile_path, starlark_code).expect("Failed to write Vedfile");

    let star_spec = load_starlark_spec(&vedfile_path).expect("Failed to load generated Vedfile");
    let direct_spec = parse_shorthand(shorthand).expect("Failed direct parse");

    assert_eq!(star_spec.app, direct_spec.app);
    assert_eq!(star_spec.version, direct_spec.version);
    assert_eq!(star_spec.services.len(), direct_spec.services.len());
    assert_eq!(
        star_spec.services["web"].wait_for,
        direct_spec.services["web"].wait_for
    );
    assert_eq!(
        star_spec.services["web"].group,
        direct_spec.services["web"].group
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_load_shorthand_spec_validates_successfully() {
    let tmp = std::env::temp_dir().join(format!("ved-test-shorthand-load-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create dir");

    let spec_path = tmp.join("app.ved");
    let content = r#"
App test-app 1.0.0
Service web nginx:alpine 8080:80
"#;
    fs::write(&spec_path, content).expect("write app.ved");

    let res = load_shorthand_spec(&spec_path);
    let _ = fs::remove_dir_all(&tmp);
    assert!(res.is_ok());
    let spec = res.unwrap();
    assert_eq!(spec.app, "test-app");
    assert_eq!(spec.services.len(), 1);
}
