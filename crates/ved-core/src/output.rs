//! Output sinking abstractions for library events and progress reporting.

use std::sync::Mutex;

/// Trait for receiving line-oriented output from library operations.
pub trait OutputSink: Send + Sync {
    /// Writes a line of output.
    fn write_line(&self, msg: &str);
}

/// Standard output sink that writes to stdout.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdoutSink;

impl OutputSink for StdoutSink {
    fn write_line(&self, msg: &str) {
        println!("{msg}");
    }
}

/// No-op output sink that discards all messages.
#[derive(Debug, Default, Clone, Copy)]
pub struct SilentSink;

impl OutputSink for SilentSink {
    fn write_line(&self, _msg: &str) {}
}

/// In-memory buffer sink that records messages for inspection.
#[derive(Debug, Default)]
pub struct BufferSink {
    lines: Mutex<Vec<String>>,
}

impl BufferSink {
    /// Creates a new empty buffer sink.
    pub fn new() -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
        }
    }

    /// Returns a copy of all recorded lines. If a writer panicked while holding
    /// the mutex, the surviving buffer is recovered and remains usable.
    pub fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl OutputSink for BufferSink {
    fn write_line(&self, msg: &str) {
        self.lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(msg.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_sink() {
        let sink = BufferSink::new();
        sink.write_line("hello");
        sink.write_line("world");
        assert_eq!(sink.lines(), vec!["hello", "world"]);
    }

    #[test]
    fn test_silent_sink() {
        let sink = SilentSink;
        sink.write_line("silence");
    }

    #[test]
    fn buffer_sink_recovers_after_poison_for_reads_and_writes() {
        let sink = std::sync::Arc::new(BufferSink::new());
        sink.write_line("before");
        let poisoning = sink.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoning.lines.lock().unwrap();
            panic!("deliberately poison the output buffer");
        })
        .join();
        assert_eq!(sink.lines(), vec!["before"]);
        sink.write_line("after");
        assert_eq!(sink.lines(), vec!["before", "after"]);
    }
}
