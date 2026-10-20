#![no_main]
use libfuzzer_sys::fuzz_target;
use ved_core::{AppSpec, ServiceSpec, ValidatedSpec};
fuzz_target!(|name: &str| {
    let mut spec = AppSpec::default();
    spec.services.insert(
        name.into(),
        ServiceSpec {
            image: Some("nginx".into()),
            ..Default::default()
        },
    );
    let safe = name
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
    assert_eq!(ValidatedSpec::try_from(spec).is_ok(), safe);
});
