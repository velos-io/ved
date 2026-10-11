#[path = "../build_support.rs"]
mod build_support;
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn bounded_output(command: &mut Command) -> std::process::Output {
    use std::process::Stdio;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("build metadata regression subprocess exceeded 10 seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ved-build-metadata-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [self.0.join("crates/ved/assets"), self.0.join("crates/ved")] {
                if path.exists() {
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn epoch_parser_rejects_invalid_or_out_of_range_values() {
    for value in [
        "",
        "-1",
        "+1",
        "1.5",
        " 1",
        "1 ",
        "１２",
        "253402300800",
        "18446744073709551616",
    ] {
        let error = build_support::parse_epoch(value).unwrap_err();
        assert!(error.contains("SOURCE_DATE_EPOCH"), "{value}: {error}");
    }
    assert_eq!(build_support::parse_epoch("0001").unwrap(), 1);
}

#[test]
fn timestamp_formats_supported_boundaries_and_leap_day() {
    for (epoch, expected) in [
        (0, "1970-01-01T00:00:00Z"),
        (951_782_400, "2000-02-29T00:00:00Z"),
        (1_700_000_000, "2023-11-14T22:13:20Z"),
        (build_support::MAX_EPOCH, "9999-12-31T23:59:59Z"),
    ] {
        assert_eq!(build_support::format_utc_timestamp(epoch), expected);
    }
}

#[test]
fn build_script_tracks_epochs_and_never_changes_source_assets() {
    let fixture = Fixture::new();
    let crate_dir = fixture.0.join("crates/ved");
    let assets = crate_dir.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in ["build.rs", "build_support.rs"] {
        std::fs::copy(manifest.join(file), crate_dir.join(file)).unwrap();
    }
    // Different root bytes expose the previous build-time copy even when run as root.
    std::fs::write(fixture.0.join("vew"), "root wrapper must not be copied").unwrap();
    let source_bytes = b"committed package wrapper";
    std::fs::write(assets.join("vew"), source_bytes).unwrap();
    let binary = fixture.0.join("build-script");
    let compile = bounded_output(
        Command::new("rustc")
            .args(["--edition=2021", "build.rs", "-o"])
            .arg(&binary)
            .current_dir(&crate_dir),
    );
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [
            crate_dir.join("build.rs"),
            crate_dir.join("build_support.rs"),
            assets.join("vew"),
        ] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        for path in [&assets, &crate_dir] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
    }
    for (epoch, timestamp) in [
        ("0", "1970-01-01T00:00:00Z"),
        ("1700000000", "2023-11-14T22:13:20Z"),
    ] {
        let output = bounded_output(
            Command::new(&binary)
                .current_dir(&crate_dir)
                .env("SOURCE_DATE_EPOCH", epoch),
        );
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH"));
        assert!(stdout.contains(&format!("cargo:rustc-env=BUILD_TIMESTAMP={timestamp}")));
        assert_eq!(std::fs::read(assets.join("vew")).unwrap(), source_bytes);
    }
    let invalid = bounded_output(
        Command::new(&binary)
            .current_dir(&crate_dir)
            .env("SOURCE_DATE_EPOCH", "invalid"),
    );
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("SOURCE_DATE_EPOCH"));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let output = bounded_output(Command::new(&binary).current_dir(&crate_dir).env(
            "SOURCE_DATE_EPOCH",
            std::ffi::OsString::from_vec(vec![0xff]),
        ));
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("SOURCE_DATE_EPOCH"));
    }
    let unset = bounded_output(
        Command::new(&binary)
            .current_dir(&crate_dir)
            .env_remove("SOURCE_DATE_EPOCH"),
    );
    assert!(unset.status.success());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&assets, &crate_dir] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[test]
fn init_emits_committed_wrapper_bytes() {
    let fixture = Fixture::new();
    let output = bounded_output(
        support::command(env!("CARGO_BIN_EXE_ved"), &fixture.0)
            .arg("--dir")
            .arg(&fixture.0)
            .arg("init"),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(fixture.0.join("vew")).unwrap(),
        include_bytes!("../assets/vew")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            std::fs::metadata(fixture.0.join("vew"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }
}

#[test]
fn repository_wrapper_matches_packaged_asset_when_present() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vew");
    if !root.exists() {
        eprintln!("repository wrapper parity skipped: package archive has no repository-root vew");
        return;
    }
    assert_eq!(
        std::fs::read(root).unwrap(),
        include_bytes!("../assets/vew")
    );
}

#[test]
fn missing_committed_asset_is_a_compilation_error() {
    let fixture = Fixture::new();
    let source = fixture.0.join("missing-asset.rs");
    std::fs::write(&source, "const WRAPPER: &str = include_str!(\"assets/vew\"); fn main() { println!(\"{WRAPPER}\"); }").unwrap();
    let output = bounded_output(
        Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(fixture.0.join("missing-asset")),
    );
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("assets/vew"), "{error}");
    assert!(error.contains("couldn't read"), "{error}");
}
