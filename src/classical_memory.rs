//! Sparse classical bytes (ISA §7.1), independent of allocation validity.
//! Compressed radix branches examine at most one bit per address-width level.
//! Checkpoints share immutable directory/page versions and displaced spans.
use crate::bigint::BigUint;
use std::rc::Rc;

// Host storage granularity only; ISA addresses remain arbitrary-width integers.
pub(crate) const PAGE_BITS: usize = 12;
pub(crate) const PAGE_SIZE: usize = 1 << PAGE_BITS;
type Link = Option<Rc<Node>>;
#[derive(Clone, Debug)]
struct Page {
    bytes: [u8; PAGE_SIZE],
    nonzero: usize,
}
#[derive(Debug)]
enum Node {
    Leaf { key: BigUint, page: Rc<Page> },
    Branch { bit: usize, children: [Rc<Node>; 2] },
}
#[derive(Debug)]
struct Displaced {
    depth: usize,
    previous: Option<Rc<Displaced>>,
    address: BigUint,
    original: Vec<u8>,
}
// A run can own a very long journal without checkpoints. Release its unique
// suffix iteratively rather than recursing through one stack frame per write.
impl Drop for Displaced {
    fn drop(&mut self) {
        let mut previous = self.previous.take();
        while let Some(entry) = previous {
            match Rc::try_unwrap(entry) {
                Ok(mut entry) => previous = entry.previous.take(),
                Err(_) => break,
            }
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Memory {
    root: Link,
    pages: usize,
    journal: Option<Rc<Displaced>>,
}
fn split_address(address: &BigUint) -> (BigUint, usize) {
    (
        address.shr(PAGE_BITS),
        address
            .mask(PAGE_BITS)
            .to_usize()
            .expect("page offset fits host usize"),
    )
}
impl Node {
    fn leaf(&self, key: &BigUint) -> (&BigUint, &Page) {
        let mut node = self;
        loop {
            #[cfg(test)]
            crate::step_scaling::record_memory_work(|w| w.metadata_probes += 1);
            match node {
                Self::Leaf { key, page } => return (key, page),
                Self::Branch { bit, children } => node = &children[usize::from(key.bit(*bit))],
            }
        }
    }
    fn set(root: Link, key: &BigUint, page: Option<Rc<Page>>, split: Option<usize>) -> Link {
        #[cfg(test)]
        crate::step_scaling::record_memory_work(|w| w.metadata_probes += 1);
        let Some(node) = root else {
            return page.map(|page| {
                Rc::new(Self::Leaf {
                    key: key.clone(),
                    page,
                })
            });
        };
        let bit = match node.as_ref() {
            Self::Branch { bit, .. } => Some(*bit),
            Self::Leaf { .. } => None,
        };
        if split.is_some_and(|split| bit.is_none_or(|bit| split > bit)) {
            let new = Rc::new(Self::Leaf {
                key: key.clone(),
                page: page.expect("new radix leaf has a page"),
            });
            let split = split.expect("radix insertion has a split bit");
            let children = if key.bit(split) {
                [node, new]
            } else {
                [new, node]
            };
            return Some(Rc::new(Self::Branch {
                bit: split,
                children,
            }));
        }
        match node.as_ref() {
            Self::Leaf { key: old, .. } => {
                assert_eq!(old, key, "radix replacement key");
                page.map(|page| {
                    Rc::new(Self::Leaf {
                        key: key.clone(),
                        page,
                    })
                })
            }
            Self::Branch { bit, children } => {
                let side = usize::from(key.bit(*bit));
                let changed = Self::set(Some(children[side].clone()), key, page, split);
                match changed {
                    None => Some(children[1 - side].clone()),
                    Some(changed) => {
                        let mut children = children.clone();
                        children[side] = changed;
                        Some(Rc::new(Self::Branch {
                            bit: *bit,
                            children,
                        }))
                    }
                }
            }
        }
    }
    fn export(&self, bytes: &mut Vec<(BigUint, u8)>) {
        match self {
            Self::Branch { children, .. } => {
                children[0].export(bytes);
                children[1].export(bytes);
            }
            Self::Leaf { key, page } => {
                let base = key.shl(PAGE_BITS);
                bytes.extend(
                    page.bytes
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| **b != 0)
                        .map(|(offset, b)| (base.add(&BigUint::from_u64(offset as u64)), *b)),
                );
            }
        }
    }
}
impl Memory {
    fn page(&self, key: &BigUint) -> Option<&Page> {
        let (found, page) = self.root.as_ref()?.leaf(key);
        (found == key).then_some(page)
    }
    // Compatibility with diagnostic trace projection: absent bytes are zero.
    pub(crate) fn get(&self, address: &BigUint) -> Option<&u8> {
        let (key, offset) = split_address(address);
        self.page(&key)
            .map(|page| &page.bytes[offset])
            .filter(|b| **b != 0)
    }
    pub(crate) fn read(&self, address: &BigUint, bytes: &mut [u8]) {
        let (mut key, mut offset) = split_address(address);
        let mut rest = bytes;
        while !rest.is_empty() {
            let count = rest.len().min(PAGE_SIZE - offset);
            let (head, tail) = rest.split_at_mut(count);
            match self.page(&key) {
                Some(page) => head.copy_from_slice(&page.bytes[offset..offset + count]),
                None => head.fill(0),
            }
            rest = tail;
            if !rest.is_empty() {
                key = key.add(&BigUint::from_u64(1));
                offset = 0;
            }
        }
    }
    pub(crate) fn write(&mut self, address: &BigUint, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut original = Vec::with_capacity(bytes.len());
        self.replace(address, bytes, Some(&mut original));
        #[cfg(test)]
        crate::step_scaling::record_memory_work(|w| {
            w.journal_bytes += bytes.len() as u64;
            w.copied_bytes += bytes.len() as u64;
        });
        let depth = self
            .journal
            .as_ref()
            .map_or(0, |entry| entry.depth)
            .checked_add(1)
            .expect("host limit: memory journal versions");
        self.journal = Some(Rc::new(Displaced {
            depth,
            previous: self.journal.take(),
            address: address.clone(),
            original,
        }));
    }
    pub(crate) fn replace(
        &mut self,
        address: &BigUint,
        bytes: &[u8],
        mut original: Option<&mut Vec<u8>>,
    ) {
        let (mut key, mut offset) = split_address(address);
        let mut rest = bytes;
        while !rest.is_empty() {
            let count = rest.len().min(PAGE_SIZE - offset);
            self.replace_page(&key, offset, &rest[..count], original.as_deref_mut());
            rest = &rest[count..];
            if !rest.is_empty() {
                key = key.add(&BigUint::from_u64(1));
                offset = 0;
            }
        }
        #[cfg(test)]
        crate::step_scaling::record_memory_work(|w| w.resident_units = self.pages as u64);
    }
    fn replace_page(
        &mut self,
        key: &BigUint,
        offset: usize,
        bytes: &[u8],
        original: Option<&mut Vec<u8>>,
    ) {
        let found = self.root.as_ref().map(|node| node.leaf(key));
        let old = found
            .filter(|(found, _)| *found == key)
            .map(|(_, page)| page);
        if let Some(original) = original {
            match old {
                Some(page) => original.extend_from_slice(&page.bytes[offset..offset + bytes.len()]),
                None => original.resize(original.len() + bytes.len(), 0),
            }
        }
        let split =
            found.and_then(|(found, _)| (found != key).then(|| found.bitxor(key).bits() - 1));
        if old.is_none() && bytes.iter().all(|b| *b == 0) {
            return;
        }
        if old.is_some_and(|page| page.bytes[offset..offset + bytes.len()] == *bytes) {
            return;
        }
        let existed = old.is_some();
        let mut page = match old {
            Some(page) => {
                #[cfg(test)]
                crate::step_scaling::record_memory_work(|w| w.copied_bytes += PAGE_SIZE as u64);
                page.clone()
            }
            None => Page {
                bytes: [0; PAGE_SIZE],
                nonzero: 0,
            },
        };
        let span = &mut page.bytes[offset..offset + bytes.len()];
        page.nonzero -= span.iter().filter(|b| **b != 0).count();
        page.nonzero += bytes.iter().filter(|b| **b != 0).count();
        span.copy_from_slice(bytes);
        let present = page.nonzero != 0;
        self.root = Node::set(self.root.take(), key, present.then(|| Rc::new(page)), split);
        self.pages = self.pages + usize::from(present) - usize::from(existed);
        #[cfg(test)]
        crate::step_scaling::record_memory_work(|w| w.page_allocations += u64::from(present));
    }
    pub(crate) fn ordered_bytes(&self) -> Vec<(BigUint, u8)> {
        let mut bytes = Vec::new();
        if let Some(root) = &self.root {
            root.export(&mut bytes);
        }
        bytes
    }
    #[cfg(test)]
    pub(crate) fn retained_journal_bytes(&self) -> usize {
        let mut bytes = 0;
        let mut cursor = self.journal.as_deref();
        while let Some(entry) = cursor {
            bytes += entry.original.len();
            cursor = entry.previous.as_deref();
        }
        bytes
    }
    pub(crate) fn undo_delta(&mut self, before: &Self, after: &Self) {
        if same_journal(&self.journal, &after.journal) {
            // No intervening memory writer: sharing the saved version also
            // releases the undone suffix when its frame checkpoints disappear.
            self.root = before.root.clone();
            self.pages = before.pages;
            self.journal = before.journal.clone();
            #[cfg(test)]
            crate::step_scaling::record_memory_work(|w| w.resident_units = self.pages as u64);
            return;
        }
        let mut left = before.journal.clone();
        let mut right = after.journal.clone();
        let mut before_spans = Vec::new();
        let mut after_spans = Vec::new();
        while !same_journal(&left, &right) {
            let left_depth = left.as_ref().map_or(0, |entry| entry.depth);
            let right_depth = right.as_ref().map_or(0, |entry| entry.depth);
            if left_depth >= right_depth {
                let entry = left.expect("memory journal common ancestor");
                left = entry.previous.clone();
                before_spans.push(entry);
            } else {
                let entry = right.expect("memory journal common ancestor");
                right = entry.previous.clone();
                after_spans.push(entry);
            }
        }
        // Uncall can restore an ancestor, then append a restoration that keeps
        // another run's bytes. Such branches need the saved pre-row values for
        // both sides of the journal fork. Ordinary writes use displaced bytes.
        let extends_before = before_spans.is_empty();
        for entry in after_spans.into_iter().chain(before_spans) {
            let mut restore = entry.original.clone();
            if !extends_before {
                before.read(&entry.address, &mut restore);
            }
            let mut changed = false;
            for (i, byte) in restore.iter_mut().enumerate() {
                let address = entry.address.add(&BigUint::from_u64(i as u64));
                if before.get(&address) == after.get(&address) {
                    *byte = self.get(&address).copied().unwrap_or(0);
                } else {
                    changed = true;
                }
            }
            if changed {
                self.write(&entry.address, &restore);
            }
        }
    }
}
fn same_journal(left: &Option<Rc<Displaced>>, right: &Option<Rc<Displaced>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Rc::ptr_eq(left, right),
        _ => false,
    }
}
impl PartialEq for Memory {
    fn eq(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (Some(a), Some(b)) if Rc::ptr_eq(a, b) => true,
            (None, None) => true,
            _ => self.ordered_bytes() == other.ordered_bytes(),
        }
    }
}
