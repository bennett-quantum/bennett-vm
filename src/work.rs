//! Deterministic work accounting for execution-scaling tests.
//!
//! One counter per unit of repeated work whose per-step cost must stay flat
//! as retained history grows. Tests snapshot and reset these counters around
//! cooperative stepping and assert work ratios; seconds are never asserted.
use crate::target::Event;

thread_local! {
    /// Incremented once per actual `target::Event` clone on this thread.
    /// With a target installed, every executed row records one classical-step
    /// event in the run's accounting journal; cloning that journal must not
    /// copy it, so clones per executed row must stay flat in history length.
    pub static EVENT_CLONE_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl Clone for Event {
    fn clone(&self) -> Self {
        EVENT_CLONE_COUNT.with(|n| n.set(n.get() + 1));
        Self {
            start: self.start.clone(),
            duration: self.duration.clone(),
            wires: self.wires.clone(),
            cost: self.cost.clone(),
        }
    }
}
