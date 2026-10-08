mod build_support;

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=build_support.rs");
    println!("cargo:rerun-if-changed=assets/vew");

    let seconds = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => build_support::parse_epoch(&value).unwrap_or_else(|error| panic!("{error}")),
        Err(std::env::VarError::NotPresent) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be at or after the Unix epoch")
            .as_secs(),
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("SOURCE_DATE_EPOCH must contain valid Unicode ASCII decimal digits")
        }
    };
    let timestamp = build_support::format_utc_timestamp(seconds);
    println!("cargo:rustc-env=BUILD_TIMESTAMP={timestamp}");
}
