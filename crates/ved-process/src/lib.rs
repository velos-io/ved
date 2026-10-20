//! A small synchronous subprocess supervisor with bounded capture, deadlines,
//! cancellation, and Unix process-group cleanup.
//!
//! Signal policy belongs to the application: runners never register signal handlers.
//! Applications that previously relied on implicit SIGINT/SIGTERM cancellation must
//! register their own handlers and pass an explicit cancellation token.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const INITIAL_POLL_INTERVAL: Duration = Duration::from_millis(1);
const MAX_POLL_INTERVAL: Duration = Duration::from_millis(10);
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const DEFAULT_CAPTURE_LIMIT: usize = 16 * 1024 * 1024;

fn configured_default_timeout() -> Duration {
    std::env::var("VED_PROCESS_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_TIMEOUT)
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    /// Shares cancellation state with an application-owned atomic flag.
    ///
    /// This does not install signal handlers or reset the flag. Applications may
    /// register the same flag with their signal adapter, then pass cloned tokens
    /// to each operation belonging to that invocation. Store `true` to cancel.
    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoMode {
    Inherit,
    Capture,
}

#[derive(Debug)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: ExitStatus,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("failed to spawn {executable}: {source}")]
    Spawn {
        executable: String,
        #[source]
        source: io::Error,
    },
    #[error("{executable} exceeded its {timeout:?} deadline")]
    TimedOut {
        executable: String,
        timeout: Duration,
    },
    #[error("{executable} was cancelled")]
    Cancelled { executable: String },
    #[error("{stream} from {executable} exceeded the {limit}-byte capture limit")]
    OutputLimit {
        executable: String,
        stream: &'static str,
        limit: usize,
    },
    #[error("I/O while running {executable}: {source}")]
    Io {
        executable: String,
        #[source]
        source: io::Error,
    },
    #[error("{executable} exited unsuccessfully with {status}")]
    Exit {
        executable: String,
        status: ExitStatus,
    },
}

#[derive(Clone, Copy, Debug)]
struct Limits {
    stdout: usize,
    stderr: usize,
}

pub struct Runner {
    executable: OsString,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    cwd: Option<PathBuf>,
    stdin: Option<Vec<u8>>,
    mode: IoMode,
    timeout: Duration,
    limits: Limits,
    token: CancellationToken,
    check_status: bool,
}

impl Runner {
    pub fn new(executable: impl AsRef<OsStr>) -> Self {
        Self {
            executable: executable.as_ref().to_owned(),
            args: vec![],
            env: vec![],
            cwd: None,
            stdin: None,
            mode: IoMode::Inherit,
            timeout: configured_default_timeout(),
            limits: Limits {
                stdout: DEFAULT_CAPTURE_LIMIT,
                stderr: DEFAULT_CAPTURE_LIMIT,
            },
            token: CancellationToken::new(),
            check_status: false,
        }
    }
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|v| v.as_ref().to_owned()));
        self
    }
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }
    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = Some(path.into());
        self
    }
    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }
    pub fn mode(mut self, mode: IoMode) -> Self {
        self.mode = mode;
        self
    }
    pub fn capture(self) -> Self {
        self.mode(IoMode::Capture)
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn capture_limits(mut self, stdout: usize, stderr: usize) -> Self {
        self.limits = Limits { stdout, stderr };
        self
    }
    pub fn cancellation_token(mut self, token: CancellationToken) -> Self {
        self.token = token;
        self
    }
    pub fn check_status(mut self, check: bool) -> Self {
        self.check_status = check;
        self
    }
    pub fn run(self) -> Result<Output, RunError> {
        let name = self.executable.to_string_lossy().into_owned();
        if self.token.is_cancelled() {
            return Err(RunError::Cancelled { executable: name });
        }
        let mut command = Command::new(&self.executable);
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        if self.stdin.is_some() {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        match self.mode {
            IoMode::Inherit => {
                command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            }
            IoMode::Capture => {
                command.stdout(Stdio::piped()).stderr(Stdio::piped());
            }
        }
        #[cfg(unix)]
        use std::os::unix::process::CommandExt;
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x00000200); // CREATE_NEW_PROCESS_GROUP
        let mut child = command.spawn().map_err(|source| RunError::Spawn {
            executable: name.clone(),
            source,
        })?;
        let pid = child.id();
        let (event_tx, event_rx) = mpsc::channel();
        let (mut writer, mut writer_done) =
            if let (Some(data), Some(mut pipe)) = (self.stdin, child.stdin.take()) {
                let tx = event_tx.clone();
                (
                    Some(thread::spawn(move || {
                        let result = pipe.write_all(&data);
                        let send_res = match &result {
                            Ok(()) => Ok(()),
                            Err(e) => Err((e.kind(), e.to_string())),
                        };
                        let _ = tx.send(SupervisorEvent::WriterDone(send_res));
                        result
                    })),
                    false,
                )
            } else {
                (None, true)
            };

        let mut readers = Vec::new();
        let mut readers_pending: usize = 0;
        if self.mode == IoMode::Capture {
            if let Some(pipe) = child.stdout.take() {
                readers_pending += 1;
                readers.push(spawn_reader(
                    pipe,
                    self.limits.stdout,
                    "stdout",
                    event_tx.clone(),
                ));
            }
            if let Some(pipe) = child.stderr.take() {
                readers_pending += 1;
                readers.push(spawn_reader(
                    pipe,
                    self.limits.stderr,
                    "stderr",
                    event_tx.clone(),
                ));
            }
        }
        drop(event_tx);

        let mut stdout_data = None;
        let mut stderr_data = None;
        let start = Instant::now();
        let mut child_status = None;
        let mut poll_interval = INITIAL_POLL_INTERVAL;

        loop {
            if self.token.is_cancelled() {
                terminate(&mut child, pid);
                join_threads(&mut writer, &mut readers);
                return Err(RunError::Cancelled { executable: name });
            }
            if start.elapsed() >= self.timeout {
                terminate(&mut child, pid);
                join_threads(&mut writer, &mut readers);
                return Err(RunError::TimedOut {
                    executable: name,
                    timeout: self.timeout,
                });
            }
            if child_status.is_none() {
                match child.try_wait() {
                    Ok(status) => child_status = status,
                    Err(source) => {
                        terminate(&mut child, pid);
                        join_threads(&mut writer, &mut readers);
                        return Err(RunError::Io {
                            executable: name,
                            source,
                        });
                    }
                }
            }
            if child_status.is_some() && writer_done && readers_pending == 0 {
                break;
            }

            let remaining = self.timeout.saturating_sub(start.elapsed());
            let wait_time = poll_interval.min(remaining);

            match event_rx.recv_timeout(wait_time) {
                Ok(SupervisorEvent::WriterDone(result)) => {
                    writer_done = true;
                    if let Err((kind, message)) = result {
                        terminate(&mut child, pid);
                        join_threads(&mut writer, &mut readers);
                        return Err(RunError::Io {
                            executable: name,
                            source: io::Error::new(kind, message),
                        });
                    }
                }
                Ok(SupervisorEvent::ReaderDone { stream, data }) => {
                    if stream == "stdout" {
                        stdout_data = Some(data);
                    } else if stream == "stderr" {
                        stderr_data = Some(data);
                    }
                    readers_pending = readers_pending.saturating_sub(1);
                }
                Ok(SupervisorEvent::LimitExceeded { stream, limit }) => {
                    terminate(&mut child, pid);
                    join_threads(&mut writer, &mut readers);
                    return Err(RunError::OutputLimit {
                        executable: name,
                        stream,
                        limit,
                    });
                }
                Ok(SupervisorEvent::ReaderIoError { kind, message }) => {
                    terminate(&mut child, pid);
                    join_threads(&mut writer, &mut readers);
                    return Err(RunError::Io {
                        executable: name,
                        source: io::Error::new(kind, message),
                    });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    poll_interval = (poll_interval * 2).min(MAX_POLL_INTERVAL);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    thread::sleep(wait_time);
                    poll_interval = (poll_interval * 2).min(MAX_POLL_INTERVAL);
                }
            }
        }

        if let Some(w) = writer.take() {
            match w.join() {
                Ok(Ok(())) => {}
                Ok(Err(source)) => {
                    return Err(RunError::Io {
                        executable: name,
                        source,
                    });
                }
                Err(_) => {
                    return Err(RunError::Io {
                        executable: name,
                        source: io::Error::other("input writer panicked"),
                    });
                }
            }
        }
        for r in readers.drain(..) {
            if r.join().is_err() {
                return Err(RunError::Io {
                    executable: name,
                    source: io::Error::other("output reader panicked"),
                });
            }
        }

        let status = child_status.expect("loop exits only after the child has exited");
        if self.check_status && !status.success() {
            return Err(RunError::Exit {
                executable: name,
                status,
            });
        }
        Ok(Output {
            stdout: stdout_data.unwrap_or_default(),
            stderr: stderr_data.unwrap_or_default(),
            status,
        })
    }
}

enum SupervisorEvent {
    WriterDone(Result<(), (io::ErrorKind, String)>),
    ReaderDone {
        stream: &'static str,
        data: Vec<u8>,
    },
    LimitExceeded {
        stream: &'static str,
        limit: usize,
    },
    ReaderIoError {
        kind: io::ErrorKind,
        message: String,
    },
}

fn spawn_reader(
    mut pipe: impl Read + Send + 'static,
    limit: usize,
    stream: &'static str,
    event_tx: mpsc::Sender<SupervisorEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut data = Vec::with_capacity(limit.min(64 * 1024));
        let mut buf = [0u8; 8192];
        loop {
            let n = match pipe.read(&mut buf) {
                Ok(n) => n,
                Err(error) => {
                    let _ = event_tx.send(SupervisorEvent::ReaderIoError {
                        kind: error.kind(),
                        message: error.to_string(),
                    });
                    return;
                }
            };
            if n == 0 {
                let _ = event_tx.send(SupervisorEvent::ReaderDone { stream, data });
                return;
            }
            if n > limit.saturating_sub(data.len()) {
                let _ = event_tx.send(SupervisorEvent::LimitExceeded { stream, limit });
                return;
            }
            data.extend_from_slice(&buf[..n]);
        }
    })
}

fn join_threads(
    writer: &mut Option<thread::JoinHandle<io::Result<()>>>,
    readers: &mut Vec<thread::JoinHandle<()>>,
) {
    if let Some(w) = writer.take() {
        let _ = w.join();
    }
    for r in readers.drain(..) {
        let _ = r.join();
    }
}

fn terminate(child: &mut Child, pid: u32) {
    #[cfg(unix)]
    {
        use rustix::process::{kill_process_group, Pid, Signal};
        let group_stopped = Pid::from_raw(pid as i32)
            .is_some_and(|pid| kill_process_group(pid, Signal::KILL).is_ok());
        if !group_stopped {
            let _ = child.kill();
        }
    }
    #[cfg(windows)]
    {
        let _ = child.kill();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}
