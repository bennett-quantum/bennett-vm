//! §10 run-owned memory intervals and discrete resource locks.
use crate::{bigint::BigUint, cc::ResourceKey};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    ops::Bound::Excluded,
};

// Count ordered lookup comparisons as well as owner entries: the test probe
// observes index work, not just the final candidate returned by a range query.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Key<T>(T);
impl<T: Ord> Ord for Key<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        #[cfg(test)]
        crate::execution_work::record(crate::execution_work::Operation::LockScan, 1);
        self.0.cmp(&other.0)
    }
}
impl<T: Ord> PartialOrd for Key<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Default)]
struct RunLocks {
    ranges: BTreeMap<Key<BigUint>, Key<BigUint>>,
    points: BTreeSet<Key<BigUint>>,
    discrete: BTreeSet<Key<ResourceKey>>,
}
impl RunLocks {
    fn conflicts(&self, key: &ResourceKey) -> bool {
        match key {
            ResourceKey::Memory(start, end) => {
                let start = Key(start.clone());
                let end = Key(end.clone());
                if let Some((_, limit)) = self.ranges.range(..end.clone()).next_back()
                    && start < *limit
                {
                    return true;
                }
                // Original strict overlap treats an empty held range as a
                // point strictly inside a nonempty query, never at an endpoint.
                start < end
                    && self
                        .points
                        .range((Excluded(start), Excluded(end)))
                        .next()
                        .is_some()
            }
            _ => self.discrete.contains(&Key(key.clone())),
        }
    }
    fn acquire(&mut self, key: ResourceKey) {
        match key {
            ResourceKey::Memory(start, end) => self.acquire_range(Key(start), Key(end)),
            key => {
                self.discrete.insert(Key(key));
            }
        }
    }
    fn acquire_range(&mut self, mut start: Key<BigUint>, mut end: Key<BigUint>) {
        assert!(start <= end, "§10: reversed internal lock range");
        if start == end {
            self.points.insert(start);
            return;
        }
        if let Some((base, limit)) = self.ranges.range(..=start.clone()).next_back() {
            if end <= *limit {
                return;
            }
            if start < *limit {
                let base = base.clone();
                self.ranges.remove(&base);
                start = base;
            }
        }
        loop {
            let next = self
                .ranges
                .range(start.clone()..end.clone())
                .next()
                .map(|(base, limit)| (base.clone(), limit.clone()));
            let Some((base, limit)) = next else {
                break;
            };
            self.ranges.remove(&base);
            end = end.max(limit);
        }
        // Merge only strictly overlapping ranges. Merging adjacency would
        // incorrectly make an empty query at their shared boundary conflict.
        self.ranges.insert(start, end);
    }
}
#[derive(Default)]
pub(crate) struct MemoryLocks {
    runs: BTreeMap<Key<u64>, RunLocks>,
}
impl MemoryLocks {
    pub(crate) fn conflicts(&self, run: u64, key: &ResourceKey) -> bool {
        self.runs.iter().any(|(owner, locks)| {
            #[cfg(test)]
            crate::execution_work::record(crate::execution_work::Operation::LockScan, 1);
            owner.0 != run && locks.conflicts(key)
        })
    }
    pub(crate) fn acquire(&mut self, run: u64, key: ResourceKey) {
        self.runs.entry(Key(run)).or_default().acquire(key);
    }
    pub(crate) fn release(&mut self, run: u64) {
        self.runs.remove(&Key(run));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn memory(start: u64, end: u64) -> ResourceKey {
        ResourceKey::Memory(BigUint::from_u64(start), BigUint::from_u64(end))
    }
    #[test]
    fn empty_queries_keep_adjacent_boundaries_and_shared_points() {
        // Hand-derived strict §10 overlap witnesses, checked on
        // tobias-ThinkPad-X1-Carbon-Gen-11, 2026-11-07.
        let mut locks = MemoryLocks::default();
        locks.acquire(1, memory(0, 4));
        locks.acquire(1, memory(4, 8));
        assert!(!locks.conflicts(2, &memory(4, 4)));
        assert!(locks.conflicts(2, &memory(3, 3)));
        locks.acquire(2, memory(4, 4));
        locks.acquire(3, memory(4, 4));
        assert!(locks.conflicts(1, &memory(2, 6)));
        locks.release(2);
        assert!(locks.conflicts(1, &memory(2, 6)));
        locks.release(3);
        assert!(!locks.conflicts(1, &memory(2, 6)));
        locks.acquire(1, memory(2, 6));
        assert!(locks.conflicts(2, &memory(4, 4)));
        locks.release(1);
        assert!(!locks.conflicts(2, &memory(0, 8)));
    }
    #[test]
    fn indexed_locks_match_linear_exclusivity_for_ranges_points_and_resources() {
        let mut indexed = MemoryLocks::default();
        let mut linear: Vec<(ResourceKey, u64)> = vec![];
        let plan = [
            (1, memory(0, 4)),
            (1, memory(4, 8)),
            (2, memory(4, 4)),
            (3, memory(9, 12)),
            (1, memory(2, 6)),
            (2, memory(8, 9)),
            (1, ResourceKey::Allocator),
            (2, ResourceKey::Entry(BigUint::from_u64(4))),
            (3, ResourceKey::Device("console".into())),
        ];
        for (owner, key) in plan {
            let blocked = linear
                .iter()
                .any(|(other, run)| *run != owner && key.overlaps(other));
            assert_eq!(indexed.conflicts(owner, &key), blocked);
            if !blocked {
                indexed.acquire(owner, key.clone());
                linear.push((key, owner));
            }
            for run in 1..=3 {
                for start in 0..=12 {
                    for end in start..=12 {
                        let query = memory(start, end);
                        let expected = linear
                            .iter()
                            .any(|(other, owner)| *owner != run && query.overlaps(other));
                        assert_eq!(
                            indexed.conflicts(run, &query),
                            expected,
                            "run {run}, [{start},{end})"
                        );
                    }
                }
                for query in [
                    ResourceKey::Allocator,
                    ResourceKey::Entry(BigUint::from_u64(4)),
                    ResourceKey::Device("console".into()),
                ] {
                    assert_eq!(
                        indexed.conflicts(run, &query),
                        linear
                            .iter()
                            .any(|(other, owner)| *owner != run && query.overlaps(other))
                    );
                }
            }
        }
        for run in 1..=3 {
            indexed.release(run);
            linear.retain(|(_, owner)| *owner != run);
            for query in [memory(0, 12), memory(4, 4), ResourceKey::Allocator] {
                assert_eq!(
                    indexed.conflicts(4, &query),
                    linear.iter().any(|(other, _)| query.overlaps(other))
                );
            }
        }
    }
}
