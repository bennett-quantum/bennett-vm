use super::Node;

impl<K: Clone, V: Clone> Clone for Node<K, V> {
    fn clone(&self) -> Self {
        let _scope = crate::execution_work::PathCopyScope::enter();
        crate::execution_work::record(crate::execution_work::Operation::PathCopy, 1);
        Self {
            key: self.key.clone(),
            value: self.value.clone(),
            left: self.left.clone(),
            right: self.right.clone(),
            height: self.height,
        }
    }
}
