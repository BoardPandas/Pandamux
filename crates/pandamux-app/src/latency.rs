//! Launch-latency instrumentation (spec 1.6).
//!
//! A [`LaunchTimeline`] is created when a session spawn begins and marks the
//! phases until the shell is interactive. When the `PANDAMUX_TIMING=1`
//! environment variable is set, each completed timeline prints one line to
//! stderr, giving comparable before/after numbers for local spawns, first SSH
//! connections, and additional sessions on an already-connected host.

use std::time::Instant;

/// Whether timing lines should print (checked once per report; cheap).
fn timing_enabled() -> bool {
    std::env::var("PANDAMUX_TIMING").is_ok_and(|value| value == "1")
}

#[derive(Clone, Debug)]
pub struct LaunchTimeline {
    /// What launched: "local pwsh", "ssh host", ... (shown in the report).
    label: String,
    t0: Instant,
    marks: Vec<(&'static str, Instant)>,
    reported: bool,
}

impl LaunchTimeline {
    pub fn start(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            t0: Instant::now(),
            marks: Vec::new(),
            reported: false,
        }
    }

    /// The short label this launch reports as ("pwsh", "ssh devbox", ...).
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Time since the launch began.
    pub fn age(&self) -> std::time::Duration {
        self.t0.elapsed()
    }

    /// Record a phase transition ("ready", "first_output", ...). Duplicate
    /// marks are ignored so callers can mark unconditionally.
    pub fn mark(&mut self, phase: &'static str) {
        if self.marks.iter().any(|(name, _)| *name == phase) {
            return;
        }
        self.marks.push((phase, Instant::now()));
    }

    /// Print the timeline once: `PANDAMUX_TIMING label: phase=+ms ...`.
    pub fn report(&mut self) {
        if self.reported {
            return;
        }
        self.reported = true;
        if !timing_enabled() {
            return;
        }
        let mut line = format!("PANDAMUX_TIMING {}:", self.label);
        for (phase, at) in &self.marks {
            line.push_str(&format!(
                " {phase}=+{}ms",
                at.duration_since(self.t0).as_millis()
            ));
        }
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_dedupe_and_report_is_idempotent() {
        let mut timeline = LaunchTimeline::start("test");
        timeline.mark("spawned");
        timeline.mark("spawned");
        timeline.mark("first_output");
        assert_eq!(timeline.marks.len(), 2);
        assert_eq!(timeline.label(), "test");
        assert!(timeline.age().as_secs() < 60);
        timeline.report();
        timeline.report();
        assert!(timeline.reported);
    }
}
