//! Test-only traversal measurements; scopes are local to each libtest thread.
use std::cell::Cell;
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Snapshot {
    pub prefix_frame_visits: u64,
    pub index_probes: u64,
    pub undo_frames: u64,
    pub event_refs: u64,
    pub skipped_tape_prefix: u64,
}
pub(crate) enum Operation { PrefixFrame, IndexProbe, UndoFrame, EventRef, SkippedTapePrefix }
thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static PREFIX: Cell<usize> = const { Cell::new(0) };
    static TAPE_PREFIX: Cell<usize> = const { Cell::new(0) };
    static WORK: Cell<Snapshot> = Cell::new(Snapshot::default());
}
pub(crate) fn record(operation: Operation, amount: u64) {
    if !ENABLED.with(Cell::get) { return; }
    WORK.with(|work| {
        let mut value = work.get();
        let count = match operation {
            Operation::PrefixFrame => &mut value.prefix_frame_visits,
            Operation::IndexProbe => &mut value.index_probes,
            Operation::UndoFrame => &mut value.undo_frames,
            Operation::EventRef => &mut value.event_refs,
            Operation::SkippedTapePrefix => &mut value.skipped_tape_prefix,
        };
        *count += amount;
        work.set(value);
    });
}
pub(crate) fn visit_frame(row: usize) {
    if row < PREFIX.with(Cell::get) { record(Operation::PrefixFrame, 1); }
}
pub(crate) fn tape_prefix(row: usize) { TAPE_PREFIX.with(|prefix| prefix.set(row)); }
pub(crate) fn visit_sequence<T>(row: usize) {
    let name = std::any::type_name::<T>();
    if name.ends_with("cc::Frame") { visit_frame(row); }
    if name == std::any::type_name::<crate::json::Json>() && row < TAPE_PREFIX.with(Cell::get) {
        record(Operation::SkippedTapePrefix, 1);
    }
}
pub(crate) struct Probe;
impl Probe {
    pub(crate) fn start() -> Self {
        ENABLED.with(|enabled| assert!(!enabled.replace(true), "nested history probe"));
        WORK.with(|work| work.set(Snapshot::default()));
        PREFIX.with(|prefix| prefix.set(0));
        TAPE_PREFIX.with(|prefix| prefix.set(0));
        Self
    }
    pub(crate) fn query_prefix(&self, row: usize) { PREFIX.with(|prefix| prefix.set(row)); }
    pub(crate) fn snapshot(&self) -> Snapshot { WORK.with(Cell::get) }
}
impl Drop for Probe {
    fn drop(&mut self) { ENABLED.with(|enabled| enabled.set(false)); }
}
