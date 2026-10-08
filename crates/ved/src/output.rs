//! Line-oriented output boundary for CLI messages.

#[cfg(test)]
use std::sync::Mutex;

pub(crate) trait Outputter: Send + Sync {
    fn println(&self, msg: &str);
    fn eprintln(&self, msg: &str);
    fn write_stdout(&self, msg: &str) {
        self.println(msg);
    }

    #[cfg(test)]
    fn info(&self, msg: &str) {
        self.println(msg);
    }
    #[cfg(test)]
    fn hint(&self, msg: &str) {
        self.println(&format!("hint: {msg}"));
    }
    fn error(&self, msg: &str) {
        self.eprintln(&format!("error: {msg}"));
    }
}

#[derive(Default, Clone, Copy)]
pub(crate) struct TerminalOutputter;

impl Outputter for TerminalOutputter {
    fn println(&self, msg: &str) {
        println!("{msg}");
    }
    fn eprintln(&self, msg: &str) {
        eprintln!("{msg}");
    }
    fn write_stdout(&self, msg: &str) {
        print!("{msg}");
    }
}

/// Thread-safe capture sink. Poisoned state is recovered so output is not lost.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct BufferOutputter {
    stdout: Mutex<Vec<String>>,
    stderr: Mutex<Vec<String>>,
}

#[cfg(test)]
impl BufferOutputter {
    pub(crate) fn stdout_lines(&self) -> Vec<String> {
        self.stdout
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(crate) fn stderr_lines(&self) -> Vec<String> {
        self.stderr
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[cfg(test)]
impl Outputter for BufferOutputter {
    fn println(&self, msg: &str) {
        self.stdout
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(msg.to_owned());
    }
    fn eprintln(&self, msg: &str) {
        self.stderr
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(msg.to_owned());
    }
    fn write_stdout(&self, msg: &str) {
        self.stdout
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(msg.to_owned());
    }
}

/// Adapter for library output events; CLI stdout/stderr retain their own routing.
pub(crate) struct CoreOutputAdapter<'a>(pub(crate) &'a dyn Outputter);
impl ved_core::OutputSink for CoreOutputAdapter<'_> {
    fn write_line(&self, msg: &str) {
        self.0.println(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_streams_and_prefixes() {
        let sink = BufferOutputter::default();
        sink.info("ready");
        sink.hint("compile");
        sink.error("failed");
        assert_eq!(sink.stdout_lines(), ["ready", "hint: compile"]);
        assert_eq!(sink.stderr_lines(), ["error: failed"]);
        let adapter = CoreOutputAdapter(&sink);
        ved_core::OutputSink::write_line(&adapter, "core event");
        assert_eq!(sink.stdout_lines().last().unwrap(), "core event");
    }
}
