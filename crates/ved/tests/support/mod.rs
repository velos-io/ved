use std::path::Path;
use std::process::Command;

pub fn command(binary: impl AsRef<std::ffi::OsStr>, root: &Path) -> Command {
    let bin = root.join("test-tools");
    std::fs::create_dir_all(&bin).unwrap();
    let home = root.join("test-home");
    std::fs::create_dir_all(&home).unwrap();
    for name in ["docker", "kubectl"] {
        let path = bin.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{name}' \"$@\" >> \"$VED_TEST_TOOL_LOG\"\ncat > /dev/null 2>&1 || true\nexit 0\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", bin)
        .env("VED_TEST_TOOL_LOG", root.join("test-tools.log"))
        .env("VED_PROCESS_TIMEOUT_SECS", "10");
    command
}
