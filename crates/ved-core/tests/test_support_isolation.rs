#![cfg(feature = "host")]
mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn base_configuration_and_explicit_environment_ignore_local_generated_overrides() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("ved-support-poison-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).expect("create fixture");
    let fixture = Fixture(root);
    std::fs::create_dir(fixture.0.join("config")).expect("create config");
    for (file, contents) in [
        (".env", "BASE_ONLY=base\nOVERRIDE=base\n"),
        (".local.env", "LOCAL_ONLY=poison\nOVERRIDE=local-poison\n"),
        (".generated.env", "GENERATED_ONLY=poison\n"),
    ] {
        std::fs::write(fixture.0.join("config").join(file), contents).expect("write config");
    }
    let context = support::context(
        &fixture.0,
        BTreeMap::from([("OVERRIDE".into(), "explicit".into())]),
    );
    assert_eq!(context.config().get_prop("BASE_ONLY", "missing"), "base");
    assert_eq!(context.config().get_prop("OVERRIDE", "missing"), "explicit");
    assert_eq!(context.config().get_prop_opt("LOCAL_ONLY"), None);
    assert_eq!(context.config().get_prop_opt("GENERATED_ONLY"), None);
    assert_eq!(
        support::context(&fixture.0, BTreeMap::new())
            .config()
            .get_prop("OVERRIDE", "missing"),
        "base"
    );
}
