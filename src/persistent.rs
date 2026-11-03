//! Private persistent AVL collections for constant-size execution checkpoints.
//! Updates copy only a balanced search path; retained roots own immutable tails.
use std::{borrow::Borrow, rc::Rc};

type Link<K, V> = Option<Rc<Node<K, V>>>;
#[derive(Clone, Debug)]
struct Node<K, V> {
    key: K,
    value: V,
    left: Link<K, V>,
    right: Link<K, V>,
    height: usize,
}
#[derive(Clone, Debug)]
pub(crate) struct Map<K, V> {
    root: Link<K, V>,
    len: usize,
}
impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self { root: None, len: 0 }
    }
}
fn height<K, V>(n: &Link<K, V>) -> usize {
    n.as_ref().map_or(0, |n| n.height)
}
fn update<K, V>(n: &mut Node<K, V>) {
    n.height = 1 + height(&n.left).max(height(&n.right));
}
fn left<K: Clone, V: Clone>(mut a: Rc<Node<K, V>>) -> Rc<Node<K, V>> {
    let mut b = Rc::make_mut(&mut a).right.take().unwrap();
    Rc::make_mut(&mut a).right = Rc::make_mut(&mut b).left.take();
    update(Rc::make_mut(&mut a));
    Rc::make_mut(&mut b).left = Some(a);
    update(Rc::make_mut(&mut b));
    b
}
fn right<K: Clone, V: Clone>(mut a: Rc<Node<K, V>>) -> Rc<Node<K, V>> {
    let mut b = Rc::make_mut(&mut a).left.take().unwrap();
    Rc::make_mut(&mut a).left = Rc::make_mut(&mut b).right.take();
    update(Rc::make_mut(&mut a));
    Rc::make_mut(&mut b).right = Some(a);
    update(Rc::make_mut(&mut b));
    b
}
fn balance<K: Clone, V: Clone>(mut n: Rc<Node<K, V>>) -> Rc<Node<K, V>> {
    update(Rc::make_mut(&mut n));
    if height(&n.left) > height(&n.right) + 1 {
        let l = n.left.as_ref().unwrap();
        if height(&l.right) > height(&l.left) {
            let l = Rc::make_mut(&mut n).left.take().unwrap();
            Rc::make_mut(&mut n).left = Some(left(l));
        }
        right(n)
    } else if height(&n.right) > height(&n.left) + 1 {
        let r = n.right.as_ref().unwrap();
        if height(&r.left) > height(&r.right) {
            let r = Rc::make_mut(&mut n).right.take().unwrap();
            Rc::make_mut(&mut n).right = Some(right(r));
        }
        left(n)
    } else {
        n
    }
}
impl<K: Ord + Clone, V: Clone> Map<K, V> {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn same_root(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(crate) fn last_key_value(&self) -> Option<(&K, &V)> {
        let mut node = self.root.as_deref()?;
        while let Some(right) = node.right.as_deref() {
            node = right;
        }
        Some((&node.key, &node.value))
    }
    pub(crate) fn get<Q: Ord + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        let mut p = self.root.as_deref();
        while let Some(n) = p {
            match key.cmp(n.key.borrow()) {
                std::cmp::Ordering::Less => p = n.left.as_deref(),
                std::cmp::Ordering::Greater => p = n.right.as_deref(),
                std::cmp::Ordering::Equal => return Some(&n.value),
            }
        }
        None
    }
    pub(crate) fn get_mut<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        let mut p = &mut self.root;
        while let Some(n) = p {
            let n = Rc::make_mut(n);
            match key.cmp(n.key.borrow()) {
                std::cmp::Ordering::Less => p = &mut n.left,
                std::cmp::Ordering::Greater => p = &mut n.right,
                std::cmp::Ordering::Equal => return Some(&mut n.value),
            }
        }
        None
    }
    pub(crate) fn contains_key<Q: Ord + ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.get(key).is_some()
    }
    pub(crate) fn insert(&mut self, key: K, value: V) {
        fn put<K: Ord + Clone, V: Clone>(
            p: Link<K, V>,
            key: K,
            value: V,
            added: &mut bool,
        ) -> Rc<Node<K, V>> {
            let Some(mut n) = p else {
                *added = true;
                return Rc::new(Node {
                    key,
                    value,
                    left: None,
                    right: None,
                    height: 1,
                });
            };
            let m = Rc::make_mut(&mut n);
            match key.cmp(&m.key) {
                std::cmp::Ordering::Less => m.left = Some(put(m.left.take(), key, value, added)),
                std::cmp::Ordering::Greater => {
                    m.right = Some(put(m.right.take(), key, value, added))
                }
                std::cmp::Ordering::Equal => m.value = value,
            }
            balance(n)
        }
        let mut added = false;
        self.root = Some(put(self.root.take(), key, value, &mut added));
        self.len += usize::from(added);
    }
    pub(crate) fn remove(&mut self, key: &K) {
        fn take<K: Ord + Clone, V: Clone>(
            p: Link<K, V>,
            key: &K,
            removed: &mut bool,
        ) -> Link<K, V> {
            let mut n = p?;
            let m = Rc::make_mut(&mut n);
            match key.cmp(&m.key) {
                std::cmp::Ordering::Less => m.left = take(m.left.take(), key, removed),
                std::cmp::Ordering::Greater => m.right = take(m.right.take(), key, removed),
                std::cmp::Ordering::Equal => {
                    *removed = true;
                    if m.left.is_none() {
                        return m.right.take();
                    }
                    if m.right.is_none() {
                        return m.left.take();
                    }
                    let mut successor = m.right.as_deref().unwrap();
                    while let Some(l) = successor.left.as_deref() {
                        successor = l;
                    }
                    m.key = successor.key.clone();
                    m.value = successor.value.clone();
                    m.right = take(m.right.take(), &m.key, &mut false);
                }
            }
            Some(balance(n))
        }
        let mut removed = false;
        self.root = take(self.root.take(), key, &mut removed);
        self.len -= usize::from(removed);
    }
    pub(crate) fn iter(&self) -> Iter<'_, K, V> {
        let mut it = Iter { stack: vec![] };
        it.descend(self.root.as_deref());
        it
    }
    pub(crate) fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(k, _)| k)
    }
    /// Merge ordered frontiers, skipping entire shared subtrees. A local update
    /// visits only its copied paths, including paths moved by AVL rotations.
    pub(crate) fn changed_keys(&self, other: &Self) -> Vec<K>
    where
        V: PartialEq,
    {
        enum Part<'a, K, V> {
            Tree(&'a Rc<Node<K, V>>),
            Entry(&'a K, &'a V),
        }
        fn expand<'a, K, V>(s: &mut Vec<Part<'a, K, V>>) {
            if let Some(Part::Tree(n)) = s.pop() {
                if let Some(r) = &n.right {
                    s.push(Part::Tree(r));
                }
                s.push(Part::Entry(&n.key, &n.value));
                if let Some(l) = &n.left {
                    s.push(Part::Tree(l));
                }
            }
        }
        let mut a = self.root.iter().map(Part::Tree).collect::<Vec<_>>();
        let mut b = other.root.iter().map(Part::Tree).collect::<Vec<_>>();
        let mut keys = vec![];
        while !a.is_empty() || !b.is_empty() {
            match (a.last(), b.last()) {
                (Some(Part::Tree(x)), Some(Part::Tree(y))) if Rc::ptr_eq(x, y) => {
                    a.pop();
                    b.pop();
                }
                (Some(Part::Tree(x)), Some(Part::Tree(y))) => match x.key.cmp(&y.key) {
                    std::cmp::Ordering::Less => expand(&mut b),
                    std::cmp::Ordering::Greater => expand(&mut a),
                    std::cmp::Ordering::Equal => {
                        expand(&mut a);
                        expand(&mut b);
                    }
                },
                (Some(Part::Tree(_)), _) => expand(&mut a),
                (_, Some(Part::Tree(_))) => expand(&mut b),
                (Some(Part::Entry(k, v)), Some(Part::Entry(l, w))) => match k.cmp(l) {
                    std::cmp::Ordering::Less => {
                        keys.push((*k).clone());
                        a.pop();
                    }
                    std::cmp::Ordering::Greater => {
                        keys.push((*l).clone());
                        b.pop();
                    }
                    std::cmp::Ordering::Equal => {
                        if v != w {
                            keys.push((*k).clone());
                        }
                        a.pop();
                        b.pop();
                    }
                },
                (Some(Part::Entry(k, _)), None) => {
                    keys.push((*k).clone());
                    a.pop();
                }
                (None, Some(Part::Entry(k, _))) => {
                    keys.push((*k).clone());
                    b.pop();
                }
                (None, None) => break,
            }
        }
        keys
    }
    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
    pub(crate) fn floor(&self, key: &K) -> Option<(&K, &V)> {
        let mut p = self.root.as_deref();
        let mut found = None;
        while let Some(n) = p {
            if n.key > *key {
                p = n.left.as_deref();
            } else {
                found = Some((&n.key, &n.value));
                p = n.right.as_deref();
            }
        }
        found
    }
}
impl<K: Ord + Clone, V: Clone + PartialEq> PartialEq for Map<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}
impl<K: Ord + Clone, V: Clone> FromIterator<(K, V)> for Map<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(it: T) -> Self {
        let mut m = Self::new();
        for (k, v) in it {
            m.insert(k, v);
        }
        m
    }
}
impl<K: Ord + Clone, V: Clone> std::ops::Index<&K> for Map<K, V> {
    type Output = V;
    fn index(&self, key: &K) -> &V {
        self.get(key).expect("missing persistent key")
    }
}
pub(crate) struct Iter<'a, K, V> {
    stack: Vec<&'a Node<K, V>>,
}
impl<'a, K, V> Iter<'a, K, V> {
    fn descend(&mut self, mut p: Option<&'a Node<K, V>>) {
        while let Some(n) = p {
            self.stack.push(n);
            p = n.left.as_deref();
        }
    }
}
impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        let n = self.stack.pop()?;
        self.descend(n.right.as_deref());
        Some((&n.key, &n.value))
    }
}

/// Append-only indexed history. Clones share all retained nodes, including values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Seq<V: Clone>(Map<usize, V>);
impl<V: Clone> Default for Seq<V> {
    fn default() -> Self {
        Self(Map::new())
    }
}
impl<V: Clone> Seq<V> {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn push(&mut self, value: V) {
        self.0.insert(self.len(), value);
    }
    pub(crate) fn same_root(&self, other: &Self) -> bool {
        self.0.same_root(&other.0)
    }
    pub(crate) fn last_mut(&mut self) -> Option<&mut V> {
        self.len().checked_sub(1).and_then(|i| self.get_mut(i))
    }
    pub(crate) fn pop(&mut self) -> Option<V> {
        let value = self.last()?.clone();
        self.truncate(self.len() - 1);
        Some(value)
    }
    pub(crate) fn last(&self) -> Option<&V> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }
    pub(crate) fn get(&self, i: usize) -> Option<&V> {
        self.0.get(&i)
    }
    pub(crate) fn get_mut(&mut self, i: usize) -> Option<&mut V> {
        self.0.get_mut(&i)
    }
    pub(crate) fn truncate(&mut self, n: usize) {
        while self.len() > n {
            self.0.remove(&(self.len() - 1));
        }
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::new();
    }
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &V> + ExactSizeIterator {
        (0..self.len()).map(|i| &self[i])
    }
    pub(crate) fn to_vec(&self) -> Vec<V> {
        self.iter().cloned().collect()
    }
    pub(crate) fn extend(&mut self, it: impl IntoIterator<Item = V>) {
        for v in it {
            self.push(v);
        }
    }
    pub(crate) fn retain(&mut self, mut f: impl FnMut(&V) -> bool) {
        *self = self.iter().filter(|v| f(v)).cloned().collect();
    }
}
impl<V: Clone> FromIterator<V> for Seq<V> {
    fn from_iter<T: IntoIterator<Item = V>>(it: T) -> Self {
        let mut s = Self::new();
        s.extend(it);
        s
    }
}
impl<V: Clone> std::ops::Index<usize> for Seq<V> {
    type Output = V;
    fn index(&self, i: usize) -> &V {
        self.get(i).expect("history index out of bounds")
    }
}
impl<V: Clone> std::ops::IndexMut<usize> for Seq<V> {
    fn index_mut(&mut self, i: usize) -> &mut V {
        self.get_mut(i).expect("history index out of bounds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeMap};
    #[test]
    fn retained_avl_roots_match_ordered_map_across_rotations_and_deletions() {
        let mut map = Map::new();
        let mut expected = BTreeMap::new();
        let mut roots = vec![];
        let mut seed = 7u64;
        for i in 0..4000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = (seed >> 32) % 400;
            if i % 3 == 0 {
                map.remove(&key);
                expected.remove(&key);
            } else {
                map.insert(key, i);
                expected.insert(key, i);
            }
            if i % 40 == 0 {
                roots.push((map.clone(), expected.clone()));
            }
            assert!(height(&map.root) <= 16);
        }
        for (root, expected) in roots {
            assert!(root.iter().eq(expected.iter()));
            let want: Vec<_> = expected
                .keys()
                .chain(map.keys())
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .filter(|k| expected.get(k) != map.get(k))
                .collect();
            assert_eq!(root.changed_keys(&map), want);
        }
    }
    #[test]
    fn local_differences_skip_shared_subtrees_even_across_root_rotations() {
        #[derive(Clone)]
        struct Compared(usize, Rc<Cell<usize>>);
        impl PartialEq for Compared {
            fn eq(&self, other: &Self) -> bool {
                self.1.set(self.1.get() + 1);
                self.0 == other.0
            }
        }
        let count = Rc::new(Cell::new(0));
        let mut map = Map::new();
        for i in 0..40000 {
            let before = map.clone();
            map.insert(i, Compared(i, count.clone()));
            if i >= 20000 {
                count.set(0);
                assert_eq!(before.changed_keys(&map), [i]);
                assert!(count.get() < 100, "{} comparisons at row {i}", count.get());
            }
        }
    }

    #[derive(Debug)]
    struct Counted(Rc<Cell<usize>>);
    impl Clone for Counted {
        fn clone(&self) -> Self {
            self.0.set(self.0.get() + 1);
            Self(self.0.clone())
        }
    }
    #[test]
    fn checkpoints_share_history_and_value_tails_at_twenty_and_forty_thousand_rows() {
        let count = Rc::new(Cell::new(0));
        let mut history = Seq::new();
        for n in 0..40000 {
            history.push(Counted(count.clone()));
            if n == 19999 || n == 39999 {
                count.set(0);
                let root = history.clone();
                assert_eq!(count.get(), 0, "checkpoint must not clone retained values");
                history.push(Counted(count.clone()));
                assert!(count.get() < 40, "one append copies only the balanced path");
                assert_eq!(root.len(), n + 1);
                history.truncate(n + 1);
            }
        }
    }
}
