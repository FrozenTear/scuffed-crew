//! One in-memory log writer for the daemon tests that read back what the
//! daemon logged: hand a clone to `tracing_subscriber::fmt().with_writer`,
//! then read the bytes from the shared buffer.

use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct TestLog(pub(crate) Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TestLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TestLog {
    type Writer = TestLog;
    fn make_writer(&'a self) -> TestLog {
        self.clone()
    }
}
