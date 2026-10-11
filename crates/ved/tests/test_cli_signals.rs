#![cfg(unix)]
use rustix::process::{kill_process, Pid, Signal};
mod support;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture {
    root: std::path::PathBuf,
    child: Child,
    descendants: Vec<i32>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if self.descendants.is_empty() {
            if let Ok(ready) = fs::read_to_string(self.root.join("ready")) {
                self.descendants = ready
                    .split_whitespace()
                    .filter_map(|pid| pid.parse().ok())
                    .collect();
            }
        }
        for pid in &self.descendants {
            if let Some(pid) = Pid::from_raw(*pid) {
                let _ = kill_process(pid, Signal::KILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready() {
        assert!(Instant::now() < deadline, "CLI signal fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn interrupted_build(signal: Signal, suffix: &str, up: bool) {
    let root = std::env::temp_dir().join(format!("ved-cli-signal-{}-{suffix}", std::process::id()));
    fs::create_dir_all(root.join(".ved")).unwrap();
    let modules: Vec<_> = (0..if up { 1 } else { 3 }).map(|i| {
        let name = format!("module{i}");
        let dir = root.join(&name);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("gradlew");
        fs::write(&script, "#!/bin/sh\ntrap '' INT TERM\necho started > started\nsleep 60 &\necho \"$$ $!\" > \"$VED_SIGNAL_READY\"\nwait\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        serde_json::json!({"name":name,"path":name,"kinds":["Gradle"],"build_cmd":"","has_dockerfile":false,"default_port":8080})
    }).collect();
    fs::write(
        root.join(".ved/state.json"),
        serde_json::to_vec(&serde_json::json!({"updated_at":"","root":root,"modules":modules}))
            .unwrap(),
    )
    .unwrap();
    let ready = root.join("ready");
    let mut command = support::command(env!("CARGO_BIN_EXE_ved"), &root);
    command.env(
        "PATH",
        format!("{}:/usr/bin:/bin", root.join("test-tools").display()),
    );
    command.args(["--dir"]).arg(&root);
    if up {
        command.args(["up", "--build"]);
    } else {
        command.args(["build", "--concurrency", "1"]);
    }
    let child = command
        .env("VED_SIGNAL_READY", &ready)
        .env("VED_PROCESS_TIMEOUT_SECS", "30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut fixture = Fixture {
        root,
        child,
        descendants: Vec::new(),
    };
    wait_until(|| {
        fs::read_to_string(&ready).is_ok_and(|pids| pids.split_whitespace().count() == 2)
    });
    let pids: Vec<i32> = fs::read_to_string(&ready)
        .unwrap()
        .split_whitespace()
        .map(|p| p.parse().unwrap())
        .collect();
    fixture.descendants = pids.clone();
    let initial_ready = fs::read_to_string(&ready).unwrap();
    kill_process(Pid::from_raw(fixture.child.id() as i32).unwrap(), signal).unwrap();
    let mut status = None;
    wait_until(|| {
        status = fixture.child.try_wait().unwrap();
        status.is_some()
    });
    assert!(!status.unwrap().success(), "cancelled build must fail");
    for pid in pids {
        wait_until(|| {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&output.stdout);
            state.trim().is_empty() || state.trim().starts_with('Z')
        });
    }
    if !up {
        assert!(!fixture.root.join("module1/started").exists());
        assert!(!fixture.root.join("module2/started").exists());
    }
    assert!(
        !fixture.root.join("test-tools.log").exists(),
        "deployment must not start after cancelled auto-build"
    );
    assert_eq!(fs::read_to_string(ready).unwrap(), initial_ready);
}

#[test]
fn sigint_cancels_active_build_and_descendants() {
    interrupted_build(Signal::INT, "int", false);
}
#[test]
fn sigterm_cancels_active_build_and_descendants() {
    interrupted_build(Signal::TERM, "term", false);
}

#[test]
fn sigterm_cancels_up_auto_build_before_deployment() {
    interrupted_build(Signal::TERM, "up", true);
}

fn interrupted_adapter(signal: Signal, suffix: &str) {
    let root =
        std::env::temp_dir().join(format!("ved-cli-adapter-{}-{suffix}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut command = support::command(env!("CARGO_BIN_EXE_ved"), &root);
    let ready = root.join("ready");
    fs::write(root.join("test-tools/docker"), "#!/bin/sh\ntrap '' INT TERM\n/bin/sleep 60 &\necho \"$$ $!\" > \"$VED_SIGNAL_READY\"\nwait\n").unwrap();
    let child = command
        .args(["--dir"])
        .arg(&root)
        .args(["ps", "--target", "compose"])
        .env("VED_SIGNAL_READY", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut fixture = Fixture {
        root,
        child,
        descendants: Vec::new(),
    };
    wait_until(|| {
        fs::read_to_string(&ready).is_ok_and(|pids| pids.split_whitespace().count() == 2)
    });
    fixture.descendants = fs::read_to_string(&ready)
        .unwrap()
        .split_whitespace()
        .map(|p| p.parse().unwrap())
        .collect();
    kill_process(Pid::from_raw(fixture.child.id() as i32).unwrap(), signal).unwrap();
    let mut status = None;
    wait_until(|| {
        status = fixture.child.try_wait().unwrap();
        status.is_some()
    });
    assert!(!status.unwrap().success());
    for pid in &fixture.descendants {
        wait_until(|| {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&output.stdout);
            state.trim().is_empty() || state.trim().starts_with('Z')
        });
    }
}

#[test]
fn sigint_cancels_context_command_adapter() {
    interrupted_adapter(Signal::INT, "int");
}
#[test]
fn sigterm_cancels_context_command_adapter() {
    interrupted_adapter(Signal::TERM, "term");
}
