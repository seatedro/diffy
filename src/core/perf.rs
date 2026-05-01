use std::time::Instant;

#[derive(Debug)]
pub struct PerfSpan {
    name: &'static str,
    details: String,
    started_at: Instant,
}

impl PerfSpan {
    pub fn new(name: &'static str, details: impl Into<String>) -> Self {
        Self {
            name,
            details: details.into(),
            started_at: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }
}

impl Drop for PerfSpan {
    fn drop(&mut self) {
        tracing::debug!(
            target: "diffy::perf",
            span = self.name,
            elapsed_ms = self.elapsed_ms() as u64,
            details = %self.details,
            "perf span"
        );
    }
}
