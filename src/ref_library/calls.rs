//! Semantic-call domains are separate from emitted leaves and their ordinals.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallTarget {
    pub interface: String,
    pub closure: String,
    pub adjoint: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallDomain {
    /// The containing invocation path, ending at the owning source row.
    pub path: Vec<crate::sched::Point>,
    pub targets: Vec<CallTarget>,
}
#[derive(Clone, Debug, Default)]
pub struct BoundCalls {
    pub(super) registry: Option<Rc<Registry>>,
    pub(super) region: Option<String>,
    pub(super) domains: Vec<CallDomain>,
}
impl BoundCalls {
    pub fn domains(&self) -> &[CallDomain] {
        &self.domains
    }
    pub(crate) fn is_for(&self, object: &crate::object::Object, region: &str) -> bool {
        self.registry
            .as_ref()
            .is_some_and(|r| r.object.text == object.text)
            && self.region.as_deref() == Some(region)
    }
}
#[derive(Clone, Debug)]
pub(super) struct RecordedDomain {
    // Keep each counter alive while its address indexes this collector. This
    // private lookup key is never part of a serialized invocation identity.
    _counter: Rc<Cell<usize>>,
    pub domain: CallDomain,
}
#[derive(Clone, Debug, Default)]
pub(super) struct CallCollector {
    pub records: Vec<RecordedDomain>,
    indices: BTreeMap<usize, usize>,
}
impl CallCollector {
    pub fn index(&self, counter: &Rc<Cell<usize>>) -> Option<usize> {
        self.indices.get(&(Rc::as_ptr(counter) as usize)).copied()
    }
}

impl Bindings {
    pub(super) fn call_domain(&self) {
        let mut domains = self.call_domains.borrow_mut();
        if domains.index(&self.call_index).is_some() {
            return;
        }
        let mut path = self.cleanup.prefix.clone();
        if let Some(point) = &self.source_point {
            let mut point = point.clone();
            point.iteration = self.source_iterations.clone();
            path.push(point);
        }
        let index = domains.records.len();
        domains
            .indices
            .insert(Rc::as_ptr(&self.call_index) as usize, index);
        domains.records.push(RecordedDomain {
            _counter: self.call_index.clone(),
            domain: CallDomain {
                path,
                targets: vec![],
            },
        });
    }
    pub(super) fn call_target(&self, target: CallTarget) {
        self.call_domain();
        let mut domains = self.call_domains.borrow_mut();
        let index = domains.index(&self.call_index).unwrap();
        let domain = &mut domains.records[index].domain;
        assert_eq!(
            domain.targets.len(),
            self.call_index.get(),
            "semantic-call domain counter drift"
        );
        domain.targets.push(target);
    }
}

impl Catalog {
    /// Keep zero-leaf calls and empty source-row domains independently of
    /// emitted leaves. The returned domains belong to this linked expansion.
    pub fn expand_with_calls(
        &self,
        kind: &str,
        args: &[Json],
        bindings: &Bindings,
    ) -> Result<(Vec<Leaf>, BoundCalls), String> {
        let mut env = bindings.clone();
        env.call_domains = Rc::default();
        env.call_index = Rc::default();
        env.call_domain();
        let mut out = vec![];
        self.invoke(kind, args, &env, &mut out, &mut 0, &mut vec![])?;
        resolve_lifetimes(&mut out)?;
        let calls = BoundCalls {
            registry: env.registry.clone(),
            region: None,
            domains: env
                .call_domains
                .borrow()
                .records
                .iter()
                .map(|d| d.domain.clone())
                .collect(),
        };
        Ok((out, calls))
    }
}
