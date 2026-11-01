//! K64 provenance over the authenticated constructor traversal.
use super::*;
use crate::sched::{Coordinate, Point};
#[derive(Clone, Debug)]
pub(super) struct OracleAddress {
    symbol: String,
    rows: Vec<Point>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct RowSpace {
    next: Rc<Cell<usize>>,
    calls: Rc<Cell<usize>>,
}
impl Bindings {
    /// Attach source-derived K64 points to a raw catalog construction. This
    /// assigns provenance only: descriptor/source consistency and invocation
    /// token admission remain obligations of the source-body builder.
    pub fn address_oracle(
        &mut self,
        object: &crate::object::Object,
        name: &str,
    ) -> Result<(), String> {
        self.link(object)?;
        let registry = self.registry.as_ref().unwrap();
        let instances = crate::instances::for_oracle(&registry.object, name)?;
        let mut rows = vec![];
        for instance in instances {
            let f = registry
                .object
                .function(&instance.symbol)
                .ok_or_else(|| err("missing source instance"))?;
            for block in &f.blocks {
                for index in 0..block.instrs.len() {
                    rows.push(Point {
                        symbol: registry.identities.interface[&instance.symbol].clone(),
                        point: format!("{}:{index}", block.name),
                        instance: instance.key.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        self.oracle_address = Some(Rc::new(OracleAddress {
            symbol: registry.identities.interface[name].clone(),
            rows,
        }));
        Ok(())
    }
    fn support(&mut self, point: &str) {
        self.source_point = Some(Point {
            symbol: self.oracle_address.as_ref().unwrap().symbol.clone(),
            point: point.into(),
            ..Default::default()
        });
        self.source_next = Rc::default();
        self.call_index = Rc::default();
        self.source_iterations.clear();
    }
    pub(super) fn inherit_address(&mut self, other: &Self) {
        self.source_point = other.source_point.clone();
        self.source_next = other.source_next.clone();
        self.call_index = other.call_index.clone();
        self.source_iterations = other.source_iterations.clone();
        self.reverse_iterations = other.reverse_iterations;
        self.coordinate = other.coordinate.clone();
        self.oracle_address = other.oracle_address.clone();
        self.row_spaces = other.row_spaces.clone();
    }
}
impl Catalog {
    pub(super) fn walk_oracle(
        &self,
        body: &Json,
        env: &Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<(), String> {
        let body = arr(body)?;
        // Bound by the authenticated oracle.construct body, never by a
        // caller's interpretation of allocation names or emitted leaves.
        let last = body.len() - 2;
        for (point, segment, phase) in [
            ("$construct", &body[..1], ""),
            ("$step", &body[1..2], "fwd"),
            ("$commit", &body[2..last], ""),
            ("$step", &body[last..last + 1], "rev"),
            ("$release", &body[last + 1..], ""),
        ] {
            let mut local = env.clone();
            local.support(point);
            local.coordinate = Coordinate {
                step: None,
                phase: phase.into(),
            };
            self.walk(&Json::Arr(segment.to_vec()), &mut local, out, next, active)?;
        }
        Ok(())
    }
    pub(super) fn walk_step(
        &self,
        body: &Json,
        env: &Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<(), String> {
        let oracle = env.oracle_address.as_ref().unwrap();
        if scalar(&Json::Str("N".into()), env)?.usize()? != oracle.rows.len() {
            return Err(err("row address domain differs from source instance rows"));
        }
        let mut local = env.clone();
        local.support("$step");
        local.row_spaces = (0..oracle.rows.len())
            .map(|_| RowSpace::default())
            .collect();
        self.walk(body, &mut local, out, next, active)
    }
    pub(super) fn address_row(&self, env: &mut Bindings, index: usize) -> Result<(), String> {
        let oracle = env.oracle_address.as_ref().unwrap();
        env.source_point = Some(
            oracle
                .rows
                .get(index)
                .ok_or_else(|| err("source row address out of range"))?
                .clone(),
        );
        let space = env
            .row_spaces
            .get(index)
            .ok_or_else(|| err("row ordinal space missing"))?;
        env.source_next = space.next.clone();
        env.call_index = space.calls.clone();
        env.source_iterations.clear();
        Ok(())
    }
}
