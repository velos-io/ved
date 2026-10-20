#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|text: &str| {
    let _ = ved_core::parse_shorthand(text);
    if let Ok(spec) = serde_yaml::from_str::<ved_core::AppSpec>(text) {
        let _ = ved_core::ValidatedSpec::try_from(spec);
    }
    let _ = ved_core::expand_env_vars_with(text, |_| None);
});
