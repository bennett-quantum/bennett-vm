//! Run-owned append storage. Checkpoints retain cursors, never chunk directories.
use std::{cell::RefCell, rc::Rc};

// Storage granularity only, not a calibrated work or timing threshold.
const CHUNK: usize = 256;
struct Chunks<T> {
    directory: Vec<Vec<Rc<T>>>,
    len: usize,
}
impl<T> Chunks<T> {
    fn new() -> Self {
        Self {
            directory: vec![],
            len: 0,
        }
    }
    fn push(&mut self, value: T) {
        if self.len.is_multiple_of(CHUNK) {
            self.directory.push(Vec::with_capacity(CHUNK));
        }
        self.directory.last_mut().unwrap().push(Rc::new(value));
        self.len += 1;
    }
    fn get(&self, row: usize) -> Rc<T> {
        assert!(row < self.len, "retained row beyond append cursor");
        self.directory[row / CHUNK][row % CHUNK].clone()
    }
    fn truncate(&mut self, len: usize) {
        assert!(len <= self.len);
        self.directory.truncate(len.div_ceil(CHUNK));
        if !len.is_multiple_of(CHUNK) {
            self.directory.last_mut().unwrap().truncate(len % CHUNK);
        }
        self.len = len;
    }
}
pub(crate) struct Completed<S> {
    start: usize,
    end: usize,
    pub(crate) value: S,
}
enum SegmentEdit {
    LivePush(usize),
    LivePop(usize),
    CancelledPush,
    CancelledPop((usize, usize)),
}
struct Store<F, S> {
    frames: Chunks<F>,
    reversed: Vec<bool>,
    marks: Vec<(usize, bool)>,
    impure: Vec<usize>,
    measured: Vec<usize>,
    segments: Chunks<Completed<S>>,
    live: Vec<usize>,
    segment_edits: Vec<SegmentEdit>,
    cancelled: Vec<(usize, usize)>,
}
/// Cloning this descriptor shares the single writer's directory. Restoring a
/// checkpoint must call restore before assigning the enclosing machine state.
pub(crate) struct History<F, S> {
    owner: Rc<RefCell<Store<F, S>>>,
    len: usize,
    segments: usize,
    impure: usize,
    measured: usize,
    marks: usize,
    segment_edits: usize,
}
impl<F, S> Clone for History<F, S> {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            len: self.len,
            segments: self.segments,
            impure: self.impure,
            measured: self.measured,
            marks: self.marks,
            segment_edits: self.segment_edits,
        }
    }
}
fn lower_bound(rows: &[usize], row: usize) -> usize {
    rows.partition_point(|entry| {
        #[cfg(test)]
        crate::history_work::record(crate::history_work::Operation::IndexProbe, 1);
        *entry < row
    })
}
impl<F, S> History<F, S> {
    pub(crate) fn new() -> Self {
        Self {
            owner: Rc::new(RefCell::new(Store {
                frames: Chunks::new(),
                reversed: vec![],
                marks: vec![],
                impure: vec![],
                measured: vec![],
                segments: Chunks::new(),
                live: vec![],
                segment_edits: vec![],
                cancelled: vec![],
            })),
            len: 0,
            segments: 0,
            impure: 0,
            measured: 0,
            marks: 0,
            segment_edits: 0,
        }
    }
    #[cfg(test)]
    pub(crate) fn edit_last(&mut self, edit: impl FnOnce(&mut F))
    where
        F: Clone,
    {
        let mut store = self.owner.borrow_mut();
        let last = store
            .frames
            .directory
            .last_mut()
            .unwrap()
            .last_mut()
            .unwrap();
        edit(Rc::make_mut(last));
    }
    pub(crate) fn prepare_push(&mut self, pure: bool) -> Result<(), String> {
        let mut store = self.owner.borrow_mut();
        if store.frames.len.is_multiple_of(CHUNK) {
            store
                .frames
                .directory
                .try_reserve(1)
                .map_err(|e| format!("history directory capacity: {e}"))?;
        }
        store
            .reversed
            .try_reserve(1)
            .map_err(|e| format!("history mark capacity: {e}"))?;
        if !pure {
            store
                .impure
                .try_reserve(1)
                .map_err(|e| format!("history fault index capacity: {e}"))?;
        }
        Ok(())
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    pub(crate) fn get(&self, row: usize) -> Rc<F> {
        assert!(row < self.len, "frame beyond checkpoint cursor");
        #[cfg(test)]
        crate::history_work::visit_frame(row);
        self.owner.borrow().frames.get(row)
    }
    pub(crate) fn last(&self) -> Option<Rc<F>> {
        self.len.checked_sub(1).map(|row| self.get(row))
    }
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = Rc<F>> + ExactSizeIterator + '_ {
        (0..self.len).map(|row| self.get(row))
    }
    pub(crate) fn reversed(&self, row: usize) -> bool {
        self.owner.borrow().reversed[row]
    }
    pub(crate) fn mark_reversed(&mut self, row: usize) {
        let mut store = self.owner.borrow_mut();
        let old = store.reversed[row];
        store.marks.push((row, old));
        store.reversed[row] = true;
        self.marks = store.marks.len();
    }
    pub(crate) fn push(&mut self, frame: F, pure: bool, measured: bool) {
        let mut store = self.owner.borrow_mut();
        assert_eq!(
            store.frames.len, self.len,
            "append through stale checkpoint"
        );
        if !pure {
            store.impure.push(self.len);
        }
        if measured {
            store.measured.push(self.len);
        }
        store.frames.push(frame);
        store.reversed.push(false);
        self.len += 1;
        self.impure = store.impure.len();
        self.measured = store.measured.len();
    }
    pub(crate) fn impure_from(&self, start: usize) -> bool {
        let store = self.owner.borrow();
        lower_bound(&store.impure, start) < store.impure.len()
    }
    pub(crate) fn measured_in(&self, start: usize, end: usize) -> bool {
        let store = self.owner.borrow();
        store
            .measured
            .get(lower_bound(&store.measured, start))
            .is_some_and(|row| *row < end)
    }
    pub(crate) fn complete(&mut self, start: usize, end: usize, value: S) {
        let mut store = self.owner.borrow_mut();
        let index = store.segments.len;
        store.segments.push(Completed { start, end, value });
        store.live.push(index);
        store.segment_edits.push(SegmentEdit::LivePush(index));
        self.segments = store.segments.len;
        self.segment_edits = store.segment_edits.len();
    }
    pub(crate) fn latest_segment(&self) -> Option<Rc<Completed<S>>> {
        let store = self.owner.borrow();
        store.live.last().map(|index| store.segments.get(*index))
    }
    /// Cancelling an outer range also retires its completed nested segments.
    pub(crate) fn retire(&mut self, start: usize) {
        let mut store = self.owner.borrow_mut();
        let segment = store.segments.get(
            *store
                .live
                .last()
                .expect("retire requires a completed segment"),
        );
        assert_eq!(segment.start, start);
        while store
            .live
            .last()
            .is_some_and(|i| store.segments.get(*i).end >= start)
        {
            let index = store.live.pop().unwrap();
            store.segment_edits.push(SegmentEdit::LivePop(index));
        }
        // Nested cancellations are absorbed by the enclosing cancellation.
        // Distinct ranges stay sorted; each absorbed range is removed once.
        while store.cancelled.last().is_some_and(|(_, end)| *end >= start) {
            let range = store.cancelled.pop().unwrap();
            assert!(range.1 <= segment.end);
            store.segment_edits.push(SegmentEdit::CancelledPop(range));
        }
        store.cancelled.push((start, segment.end));
        store.segment_edits.push(SegmentEdit::CancelledPush);
        self.segment_edits = store.segment_edits.len();
    }
    pub(crate) fn cuts_cancelled_segment(&self, row: usize) -> bool {
        let store = self.owner.borrow();
        let next = store.cancelled.partition_point(|(start, _)| {
            #[cfg(test)]
            crate::history_work::record(crate::history_work::Operation::IndexProbe, 1);
            *start < row
        });
        next > 0 && row <= store.cancelled[next - 1].1
    }
    /// Roll back failed publication, including edits to earlier reversal marks
    /// and completed live segments. Only this owner's unpublished tail is cut.
    pub(crate) fn restore(&self) {
        let mut store = self.owner.borrow_mut();
        while store.marks.len() > self.marks {
            let (row, old) = store.marks.pop().unwrap();
            store.reversed[row] = old;
        }
        while store.segment_edits.len() > self.segment_edits {
            match store.segment_edits.pop().unwrap() {
                SegmentEdit::LivePush(index) => assert_eq!(store.live.pop(), Some(index)),
                SegmentEdit::LivePop(index) => store.live.push(index),
                SegmentEdit::CancelledPush => {
                    store.cancelled.pop().expect("cancelled interval edit");
                }
                SegmentEdit::CancelledPop(range) => store.cancelled.push(range),
            }
        }
        store.segments.truncate(self.segments);
        store.frames.truncate(self.len);
        store.reversed.truncate(self.len);
        store.impure.truncate(self.impure);
        store.measured.truncate(self.measured);
    }
    /// Tail rewind keeps earlier audit marks, including cancellation by a
    /// removed marker, exactly as the logical history projection did before.
    pub(crate) fn truncate(&mut self, len: usize) {
        let mut store = self.owner.borrow_mut();
        store.frames.truncate(len);
        store.reversed.truncate(len);
        let impure_len = lower_bound(&store.impure, len);
        store.impure.truncate(impure_len);
        let measured_len = lower_bound(&store.measured, len);
        store.measured.truncate(measured_len);
        while store.segments.len > 0 && store.segments.get(store.segments.len - 1).end >= len {
            let index = store.segments.len - 1;
            if store.live.last() == Some(&index) {
                store.live.pop();
            }
            store.segments.truncate(index);
        }
        // No rollback checkpoint spans an explicit tail rewind. Old journal
        // entries would refer to discarded row identities after re-execution.
        store.marks.clear();
        store.segment_edits.clear();
        let cancelled_len = store.cancelled.partition_point(|(start, _)| *start < len);
        store.cancelled.truncate(cancelled_len);
        self.len = len;
        self.segments = store.segments.len;
        self.impure = store.impure.len();
        self.measured = store.measured.len();
        self.marks = 0;
        self.segment_edits = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_records_survive_chunk_tail_replacement() {
        // Non-Clone payloads make payload copying impossible. Cross both a
        // chunk boundary and a directory-growth boundary on each rewrite.
        struct Record(usize);
        let mut history = History::<Record, ()>::new();
        for row in 0..3 * CHUNK {
            history.push(Record(row), true, false);
        }
        let frozen = history.get(CHUNK + 1);
        for replacement in [11, 22, 33] {
            history.truncate(CHUNK - 1);
            for _ in CHUNK - 1..3 * CHUNK {
                history.push(Record(replacement), true, false);
            }
            assert_eq!(history.get(CHUNK + 1).0, replacement);
            assert_eq!(frozen.0, CHUNK + 1);
        }
    }

    #[test]
    fn rollback_restores_prior_marks_live_segments_and_private_append_tail() {
        let mut history = History::new();
        for row in 0..4 {
            history.push(row, true, false);
        }
        history.complete(1, 2, "inner");
        history.complete(0, 3, "outer");
        let checkpoint = history.clone();
        history.mark_reversed(1);
        history.retire(0);
        assert!(history.latest_segment().is_none());
        assert!(history.cuts_cancelled_segment(2));
        assert!(!history.cuts_cancelled_segment(0));
        assert!(!history.cuts_cancelled_segment(4));
        history.push(4, false, true);
        let inner_checkpoint = history.clone();
        history.mark_reversed(0);
        history.push(5, false, true);
        inner_checkpoint.restore();
        history = inner_checkpoint;
        assert!(history.reversed(1));
        assert!(!history.reversed(0));
        assert!(history.measured_in(4, 5));
        checkpoint.restore();
        history = checkpoint;
        assert_eq!(history.len(), 4);
        assert_eq!(history.latest_segment().unwrap().value, "outer");
        assert!(!history.reversed(1));
        assert!(!history.cuts_cancelled_segment(2));
        assert!(!history.impure_from(0));
        assert!(!history.measured_in(0, 5));
    }

    #[test]
    fn nested_cancellation_index_matches_all_retired_segment_ranges_after_rewind() {
        let mut history = History::new();
        for row in 0..8 {
            history.push(row, true, false);
        }
        history.complete(2, 3, "inner");
        history.retire(2);
        history.complete(1, 5, "outer");
        history.retire(1);
        history.complete(6, 7, "later");
        history.retire(6);
        for row in 0..=8 {
            let expected = [(2, 3), (1, 5), (6, 7)]
                .iter()
                .any(|(start, end)| *start < row && row <= *end);
            assert_eq!(history.cuts_cancelled_segment(row), expected, "cut {row}");
        }
        history.truncate(6);
        assert!(history.cuts_cancelled_segment(5));
        assert!(!history.cuts_cancelled_segment(7));
        history.truncate(1);
        assert!(!history.cuts_cancelled_segment(2));
    }

    #[test]
    fn cancelled_audit_rows_stay_impure_until_their_tail_is_removed() {
        let mut history = History::<usize, ()>::new();
        history.push(0, false, false);
        history.push(1, true, true);
        history.mark_reversed(0);
        history.mark_reversed(1);
        assert!(history.impure_from(0));
        assert!(history.measured_in(1, 2));
        history.truncate(1);
        assert!(history.reversed(0));
        assert!(history.impure_from(0));
        assert!(!history.measured_in(0, 2));
        history.truncate(0);
        history.push(2, true, false);
        assert!(!history.reversed(0));
        assert!(!history.impure_from(0));
    }
}
