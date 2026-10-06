use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use ved_process::{CancellationToken, IoMode, RunError, Runner};

fn fixture() -> &'static str {
    env!("CARGO_BIN_EXE_process-fixture")
}

#[test]
fn deadline_terminates_and_reaps_child() {
    let started = Instant::now();
    let result = Runner::new(fixture())
        .arg("sleep")
        .timeout(Duration::from_millis(120))
        .run();
    assert!(matches!(result, Err(RunError::TimedOut { .. })));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn cancellation_is_distinct_from_timeout() {
    let token = CancellationToken::new();
    let cancel = token.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        cancel.cancel();
    });
    let result = Runner::new(fixture())
        .arg("sleep")
        .timeout(Duration::from_secs(3))
        .cancellation_token(token)
        .run();
    assert!(matches!(result, Err(RunError::Cancelled { .. })));
}

#[test]
fn drains_large_stdin_stdout_and_stderr_concurrently() {
    let input = vec![b'i'; 2 * 1024 * 1024];
    let result = Runner::new(fixture())
        .args(["duplex", "2097152"])
        .stdin(input)
        .capture()
        .capture_limits(3 * 1024 * 1024, 3 * 1024 * 1024)
        .timeout(Duration::from_secs(10))
        .run()
        .unwrap();
    assert_eq!(result.stdout.len(), 2 * 1024 * 1024 + 8);
    assert_eq!(result.stderr.len(), 2 * 1024 * 1024);
    assert_eq!(
        u64::from_le_bytes(result.stdout[result.stdout.len() - 8..].try_into().unwrap()),
        2 * 1024 * 1024
    );
}

#[test]
fn output_limits_fail_and_kill() {
    let result = Runner::new(fixture())
        .args(["write", "1000000"])
        .capture()
        .capture_limits(1_500_000, 4096)
        .timeout(Duration::from_secs(3))
        .run();
    assert!(matches!(
        result,
        Err(RunError::OutputLimit {
            stream: "stderr",
            limit: 4096,
            ..
        })
    ));
}

#[cfg(unix)]
#[test]
fn deadline_remains_active_while_descendant_holds_capture_pipes() {
    let started = Instant::now();
    let result = Runner::new(fixture())
        .arg("orphan-pipes")
        .capture()
        .timeout(Duration::from_millis(150))
        .run();
    assert!(matches!(result, Err(RunError::TimedOut { .. })));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn spawn_failure_and_nonzero_exit_are_typed() {
    assert!(matches!(
        Runner::new("/definitely/missing/ved-process-fixture").run(),
        Err(RunError::Spawn { .. })
    ));
    let result = Runner::new(fixture())
        .args(["exit", "42"])
        .mode(IoMode::Capture)
        .check_status(true)
        .run();
    assert!(matches!(result, Err(RunError::Exit { .. })));
}

#[test]
fn stdin_pipe_failure_reaps_the_child() {
    let result = Runner::new(fixture())
        .arg("exit-immediately")
        .stdin(vec![b'i'; 4 * 1024 * 1024])
        .timeout(Duration::from_secs(3))
        .run();
    assert!(matches!(result, Err(RunError::Io { .. })));
}

#[cfg(unix)]
#[test]
fn timeout_kills_descendant_process_group() {
    let directory = std::env::temp_dir().join(format!("ved-process-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let pid_path = directory.join("child.pid");
    let result = Runner::new(fixture())
        .args(["descendant", pid_path.to_str().unwrap()])
        .timeout(Duration::from_secs(2))
        .run();
    assert!(matches!(result, Err(RunError::TimedOut { .. })));
    let pid: i32 = std::fs::read_to_string(&pid_path).unwrap().parse().unwrap();
    thread::sleep(Duration::from_millis(100));
    let probe = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(
        !probe.success(),
        "descendant pid {pid} survived process-group termination"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn test_directory(name: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "ved-process-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn wait_ready(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(Instant::now() < deadline, "fixture did not become ready");
        thread::sleep(Duration::from_millis(10));
    }
}

fn start_ready_runner(
    path: std::path::PathBuf,
    token: CancellationToken,
) -> thread::JoinHandle<Result<ved_process::Output, RunError>> {
    thread::spawn(move || {
        Runner::new(fixture())
            .arg("ready-sleep")
            .arg(path)
            .timeout(Duration::from_secs(3))
            .cancellation_token(token)
            .run()
    })
}

#[test]
fn cancelling_one_runner_does_not_cancel_an_independent_runner() {
    let directory = test_directory("independent");
    let cancelled = CancellationToken::new();
    let independent = CancellationToken::new();
    let first = start_ready_runner(directory.join("first"), cancelled.clone());
    let second = start_ready_runner(directory.join("second"), independent.clone());
    wait_ready(&directory.join("first"));
    wait_ready(&directory.join("second"));
    cancelled.cancel();
    assert!(matches!(
        first.join().unwrap(),
        Err(RunError::Cancelled { .. })
    ));
    assert!(second.join().unwrap().unwrap().status.success());
    assert!(!independent.is_cancelled());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn application_flag_cancels_all_runners_sharing_the_token() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let directory = test_directory("shared");
    let flag = Arc::new(AtomicBool::new(false));
    let token = CancellationToken::from_flag(flag.clone());
    let first = start_ready_runner(directory.join("first"), token.clone());
    let second = start_ready_runner(directory.join("second"), token.clone());
    wait_ready(&directory.join("first"));
    wait_ready(&directory.join("second"));
    let started = Instant::now();
    flag.store(true, Ordering::Release);
    assert!(matches!(
        first.join().unwrap(),
        Err(RunError::Cancelled { .. })
    ));
    assert!(matches!(
        second.join().unwrap(),
        Err(RunError::Cancelled { .. })
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(token.is_cancelled());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pre_cancelled_application_flag_prevents_spawn() {
    let directory = test_directory("pre-cancelled");
    let marker = directory.join("spawned");
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let result = Runner::new(fixture())
        .arg("mark")
        .arg(&marker)
        .cancellation_token(CancellationToken::from_flag(flag))
        .run();
    assert!(matches!(result, Err(RunError::Cancelled { .. })));
    assert!(!marker.exists(), "pre-cancelled runner spawned its child");
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn runner_preserves_default_signal_policy_in_dedicated_children() {
    use rustix::process::{kill_process, Pid, Signal};
    use std::os::unix::process::ExitStatusExt;
    for signal in [Signal::INT, Signal::TERM] {
        let directory = test_directory("signal-policy");
        let ready = directory.join("ready");
        let mut child = Command::new(fixture())
            .arg("signal-policy")
            .arg(&ready)
            .spawn()
            .unwrap();
        wait_ready(&ready);
        kill_process(Pid::from_raw(child.id() as i32).unwrap(), signal).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("signal-policy fixture failed to exit");
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.signal(), Some(signal.as_raw()));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
