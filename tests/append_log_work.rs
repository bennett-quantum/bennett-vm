use super::Entry;

impl<T: Clone> Clone for Entry<T> {
    fn clone(&self) -> Self {
        let _scope = crate::execution_work::PathCopyScope::enter();
        crate::execution_work::record(crate::execution_work::Operation::PathCopy, 1);
        Self {
            value: self.value.clone(),
            previous: self.previous.clone(),
        }
    }
}
