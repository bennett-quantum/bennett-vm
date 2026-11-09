//! Test-only deterministic work probes; each thread owns its measurement scope.
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Snapshot {
    pub steps: u64,
    pub append_nodes: u64,
    pub append_payload_bytes: u64,
    pub machine_checkpoints: u64,
    pub instance_tree_updates: u64,
    pub map_nodes: u64,
    pub path_copies: u64,
    pub target_history_copies: u64,
    pub accounting_comparisons: u64,
    /// Retained Frame/Json values copied with a search-path node.
    pub payload_clones: u64,
    pub logical_value_clones: u64,
    pub lock_scans: u64,
}
pub(crate) enum Operation {
    Step,
    AppendNode,
    AppendPayloadBytes,
    MachineCheckpoint,
    InstanceTreeUpdate,
    MapNode,
    PathCopy,
    TargetHistoryCopy,
    AccountingComparison,
    PayloadClone,
    LogicalValueClone,
    LockScan,
}
thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static INSTANCE_UPDATE: Cell<bool> = const { Cell::new(false) };
    static PATH_COPY: Cell<bool> = const { Cell::new(false) };
    static WORK: Cell<Snapshot> = Cell::new(Snapshot::default());
}
pub(crate) fn record(operation: Operation, amount: u64) {
    if ENABLED.with(Cell::get) {
        WORK.with(|cell| {
            let mut work = cell.get();
            let count = match operation {
                Operation::Step => &mut work.steps,
                Operation::AppendNode => &mut work.append_nodes,
                Operation::AppendPayloadBytes => &mut work.append_payload_bytes,
                Operation::MachineCheckpoint => &mut work.machine_checkpoints,
                Operation::InstanceTreeUpdate => &mut work.instance_tree_updates,
                Operation::MapNode => &mut work.map_nodes,
                Operation::PathCopy => &mut work.path_copies,
                Operation::TargetHistoryCopy => &mut work.target_history_copies,
                Operation::AccountingComparison => &mut work.accounting_comparisons,
                Operation::PayloadClone => &mut work.payload_clones,
                Operation::LogicalValueClone => &mut work.logical_value_clones,
                Operation::LockScan => &mut work.lock_scans,
            };
            *count += amount;
            cell.set(work);
        });
    }
}
pub(crate) struct PathCopyScope(bool);
impl PathCopyScope {
    pub(crate) fn enter() -> Self {
        Self(PATH_COPY.with(|copying| copying.replace(true)))
    }
}
impl Drop for PathCopyScope {
    fn drop(&mut self) {
        PATH_COPY.with(|copying| copying.set(self.0));
    }
}
pub(crate) fn payload_clone() {
    let operation = if PATH_COPY.with(Cell::get) {
        Operation::PayloadClone
    } else {
        Operation::LogicalValueClone
    };
    record(operation, 1);
}
pub(crate) struct Probe;
impl Probe {
    pub(crate) fn start() -> Self {
        ENABLED.with(|enabled| assert!(!enabled.replace(true), "nested work probe"));
        WORK.with(|work| work.set(Snapshot::default()));
        Self
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        WORK.with(Cell::get)
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        ENABLED.with(|enabled| enabled.set(false));
    }
}
impl Clone for crate::json::Json {
    fn clone(&self) -> Self {
        payload_clone();
        match self {
            Self::Null => Self::Null,
            Self::Bool(value) => Self::Bool(*value),
            Self::Num(value) => Self::Num(value.clone()),
            Self::Str(value) => Self::Str(value.clone()),
            Self::Arr(value) => Self::Arr(value.clone()),
            Self::Obj(value) => Self::Obj(value.clone()),
        }
    }
}
impl Clone for crate::target::Event {
    fn clone(&self) -> Self {
        record(Operation::TargetHistoryCopy, 1);
        Self {
            start: self.start.clone(),
            duration: self.duration.clone(),
            wires: self.wires.clone(),
            cost: self.cost.clone(),
        }
    }
}

// Scope only the per-name stack update, separately from the outer name index.
pub(crate) struct InstanceScope(bool);
impl InstanceScope {
    pub(crate) fn enter() -> Self {
        Self(INSTANCE_UPDATE.with(|active| active.replace(true)))
    }
}
impl Drop for InstanceScope {
    fn drop(&mut self) {
        INSTANCE_UPDATE.with(|active| active.set(self.0));
    }
}
pub(crate) fn instance_tree_update() {
    if INSTANCE_UPDATE.with(Cell::get) {
        record(Operation::InstanceTreeUpdate, 1);
    }
}
