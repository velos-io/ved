//! Application-owned, scoped signal policy for one CLI invocation.
use ved_process::CancellationToken;

pub(crate) struct InvocationSignals {
    token: CancellationToken,
    #[cfg(unix)]
    registrations: Vec<signal_hook::SigId>,
}

impl InvocationSignals {
    pub(crate) fn new() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            Self::register_with(signal_hook::flag::register)
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                token: CancellationToken::new(),
            })
        }
    }

    #[cfg(unix)]
    fn register_with(
        mut register: impl FnMut(
            i32,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ) -> std::io::Result<signal_hook::SigId>,
    ) -> std::io::Result<Self> {
        use std::sync::{atomic::AtomicBool, Arc};
        let flag = Arc::new(AtomicBool::new(false));
        let mut guard = Self {
            token: CancellationToken::from_flag(flag.clone()),
            registrations: Vec::new(),
        };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            // A failed second registration drops the first registration too.
            guard
                .registrations
                .push(register(signal, flag.clone()).map_err(|error| {
                    std::io::Error::new(
                        error.kind(),
                        format!("Failed to register CLI signal {signal}: {error}"),
                    )
                })?);
        }
        Ok(guard)
    }

    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for InvocationSignals {
    fn drop(&mut self) {
        #[cfg(unix)]
        for registration in self.registrations.drain(..) {
            signal_hook::low_level::unregister(registration);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn signal_guard_lifetime_child() {
        if std::env::var_os("VED_SIGNAL_GUARD_CHILD").is_none() {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "signals::tests::signal_guard_lifetime_child"])
                .env("VED_SIGNAL_GUARD_CHILD", "1")
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("guard child timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            return;
        }
        let guard = InvocationSignals::new().unwrap();
        let old = guard.token();
        drop(guard);
        let mut failed_flag: Option<Arc<AtomicBool>> = None;
        let mut calls = 0;
        let error = InvocationSignals::register_with(|signal, flag| {
            calls += 1;
            if calls == 2 {
                return Err(std::io::Error::other("injected registration failure"));
            }
            failed_flag = Some(flag.clone());
            signal_hook::flag::register(signal, flag)
        });
        let error = error.err().expect("injected setup failure");
        assert!(error.to_string().contains("Failed to register CLI signal"));
        assert!(error.to_string().contains("injected registration failure"));
        let current = InvocationSignals::new().unwrap();
        signal_hook::low_level::raise(signal_hook::consts::SIGINT).unwrap();
        assert!(current.token().is_cancelled());
        assert!(!old.is_cancelled());
        assert!(!failed_flag.unwrap().load(Ordering::SeqCst));
    }
}
