//! Immutable append tails: constant-work append, shared checkpoints, ordered export.
use std::rc::Rc;

#[cfg_attr(not(test), derive(Clone))]
#[derive(Debug)]
struct Entry<T> {
    value: T,
    previous: Option<Rc<Entry<T>>>,
}
#[derive(Debug)]
pub(crate) struct AppendLog<T> {
    tail: Option<Rc<Entry<T>>>,
    len: usize,
}
impl<T> Clone for AppendLog<T> {
    fn clone(&self) -> Self {
        Self {
            tail: self.tail.clone(),
            len: self.len,
        }
    }
}
impl<T> Default for AppendLog<T> {
    fn default() -> Self {
        Self { tail: None, len: 0 }
    }
}
impl<T> Drop for AppendLog<T> {
    fn drop(&mut self) {
        let mut tail = self.tail.take();
        // Release long unique tails iteratively; shared snapshots keep their prefix.
        while let Some(entry) = tail {
            match Rc::try_unwrap(entry) {
                Ok(mut entry) => tail = entry.previous.take(),
                Err(_) => break,
            }
        }
    }
}
impl<T> AppendLog<T> {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(crate) fn last(&self) -> Option<&T> {
        self.tail.as_ref().map(|e| &e.value)
    }
    pub(crate) fn last_mut(&mut self) -> Option<&mut T>
    where
        T: Clone,
    {
        let tail = self.tail.as_mut()?;
        let entry = Rc::make_mut(tail);
        let value = &mut entry.value;
        Some(value)
    }
    pub(crate) fn pop_value(&mut self) -> Option<T>
    where
        T: Clone,
    {
        let tail = self.tail.take()?;
        let value = tail.value.clone();
        self.tail = tail.previous.clone();
        self.len -= 1;
        Some(value)
    }
    pub(crate) fn push(&mut self, value: T) {
        #[cfg(test)]
        {
            use crate::execution_work::{Operation, record};
            record(Operation::AppendNode, 1);
            record(
                Operation::AppendPayloadBytes,
                std::mem::size_of::<Entry<T>>() as u64,
            );
        }
        self.tail = Some(Rc::new(Entry {
            value,
            previous: self.tail.take(),
        }));
        self.len += 1;
    }
    pub(crate) fn pop(&mut self) {
        let tail = self.tail.take().expect("pop requires a retained entry");
        self.tail = tail.previous.clone();
        self.len -= 1;
    }
    /// Uncall may cancel an instance below a newer pure suffix. Share the
    /// older prefix and rebuild only that suffix; ordinary LIFO removal is O(1).
    pub(crate) fn remove_latest(&mut self, mut matches: impl FnMut(&T) -> bool) -> bool
    where
        T: Clone,
    {
        let mut tail = self.tail.as_deref();
        let mut suffix = Vec::new();
        while let Some(entry) = tail {
            if matches(&entry.value) {
                self.truncate(self.len - suffix.len() - 1);
                for value in suffix.into_iter().rev() {
                    self.push(value);
                }
                return true;
            }
            suffix.push(entry.value.clone());
            tail = entry.previous.as_deref();
        }
        false
    }
    pub(crate) fn truncate(&mut self, len: usize) {
        assert!(len <= self.len, "append log cannot extend by truncation");
        while self.len > len {
            self.pop();
        }
    }
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        let mut values = Vec::with_capacity(self.len);
        let mut tail = self.tail.as_deref();
        while let Some(entry) = tail {
            values.push(&entry.value);
            tail = entry.previous.as_deref();
        }
        values.into_iter().rev()
    }
    pub(crate) fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.iter().cloned().collect()
    }
}
impl<T> FromIterator<T> for AppendLog<T> {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        let mut log = Self::new();
        for value in values {
            log.push(value);
        }
        log
    }
}
impl<T: PartialEq> PartialEq for AppendLog<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branches_rewind_and_release_long_shared_tails() {
        let mut log: AppendLog<_> = (0..20000).collect();
        let prefix = log.clone();
        log.push(20000);
        log.truncate(15000);
        log.push(42);
        assert_eq!(log.last(), Some(&42));
        assert_eq!(log.len(), 15001);
        assert_eq!(
            prefix.iter().copied().collect::<Vec<_>>(),
            (0..20000).collect::<Vec<_>>()
        );
        drop(prefix);
        assert_eq!(
            log.iter().take(3).copied().collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }
    #[test]
    fn cancellation_preserves_newer_instances_and_shared_versions() {
        let mut log: AppendLog<_> = (0..20000).collect();
        let retained = log.clone();
        assert!(log.remove_latest(|v| *v == 19997));
        assert_eq!(
            log.iter().rev().take(3).copied().collect::<Vec<_>>(),
            vec![19999, 19998, 19996]
        );
        assert_eq!(retained.len(), 20000);
        assert_eq!(retained.iter().rev().nth(2), Some(&19997));
        let before = log.clone();
        assert!(!log.remove_latest(|v| *v == 20000));
        assert_eq!(log, before);
    }
}

#[cfg(test)]
#[path = "../tests/append_log_work.rs"]
mod work;
