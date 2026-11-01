//! Whole-pair decisions are separate from emitted leaves and their ordinals.
use super::*;
use crate::sched::{Operation, OperationKind, Point, Selector};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CleanupRealisation {
    #[default]
    Reverse,
    Recompute,
}
#[derive(Clone, Debug)]
pub struct CleanupEntry {
    pub selector: Selector,
    pub realisation: CleanupRealisation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupCarrier {
    pub address: Operation,
    pub computing_point: Point,
    pub realisation: CleanupRealisation,
    pub controls: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub struct ActionExpansion {
    pub leaves: Vec<Leaf>,
    pub cleanups: Vec<CleanupCarrier>,
    /// False for source pairs whose whole realisation is still staged. Such a
    /// default trace must not be treated as an admitted cleanup schedule.
    pub cleanup_pairing_complete: bool,
}
#[derive(Clone, Debug, Default)]
pub(super) struct CleanupContext {
    pub entries: Rc<Vec<CleanupEntry>>,
    pub carriers: Rc<RefCell<Vec<CleanupCarrier>>>,
    pub prefix: Vec<Point>,
    pub incomplete: Rc<Cell<bool>>,
}
impl Bindings {
    fn cleanup_operation(&self) -> Result<Operation, String> {
        let mut point = self
            .source_point
            .clone()
            .ok_or_else(|| err("cleanup lacks a source point"))?;
        point.iteration = self.source_iterations.clone();
        let mut path = self.cleanup.prefix.clone();
        path.push(point);
        Ok(Operation {
            path,
            step: self.coordinate.step,
            phase: self.coordinate.phase.clone(),
            generated: 0,
            kind: OperationKind::Cleanup,
        })
    }
}
impl Catalog {
    pub(super) fn walk_pending_cleanup(
        &self,
        c: &Json,
        env: &mut Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<(), String> {
        env.cleanup.incomplete.set(true);
        let address = env.cleanup_operation()?;
        let selectors = env
            .cleanup
            .entries
            .iter()
            .map(|e| e.selector.clone())
            .collect::<Vec<_>>();
        if crate::sched::select(&selectors, &address)?.is_some() {
            return Err(err(
                "unavailable.reason: whole multirow cleanup pairing/addresses remain staged (ISA b-isa-spec-5cd)",
            ));
        }
        self.walk(field(c, "body")?, env, out, next, active)
    }
    pub(super) fn walk_cleanup(
        &self,
        c: &Json,
        env: &mut Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<(), String> {
        let pair = field(c, "$cleanup")?;
        let address = env.cleanup_operation()?;
        let point = address.path.last().unwrap().clone();
        let selectors = env
            .cleanup
            .entries
            .iter()
            .map(|e| e.selector.clone())
            .collect::<Vec<_>>();
        let choice = crate::sched::select(&selectors, &address)?
            .map_or(CleanupRealisation::Reverse, |n| {
                env.cleanup.entries[n].realisation
            });
        let mut computing_point = point;
        computing_point.point = string(field(pair, "point")?)?.into();
        env.cleanup.carriers.borrow_mut().push(CleanupCarrier {
            address,
            computing_point,
            realisation: choice,
            controls: env.controls.clone(),
        });
        if choice == CleanupRealisation::Reverse {
            return self.walk(field(c, "body")?, env, out, next, active);
        }
        if field(pair, "perm")? != &Json::Bool(true) {
            return Err(err("recompute requires a permutation computing sequence"));
        }
        let name = string(field(pair, "target")?)?;
        if name.is_empty() {
            let computing = field(pair, "computing")?.clone();
            let mut body = arr(&computing)?.to_vec();
            body.push(Json::obj(vec![
                ("inverse", Json::Bool(true)),
                ("body", computing),
            ]));
            return self.walk(&Json::Arr(body), env, out, next, active);
        }
        let Some(Bound::Allocation(target)) = env.values.get(name) else {
            return Err(err("cleanup target is not a bound allocation"));
        };
        let target = target.borrow();
        if target.state != "clean" {
            return Err(err("recompute requires a clean owned target"));
        }
        let width = target.width;
        let class = target.class.clone();
        drop(target);
        let mut local = env.clone();
        let original = format!("$cleanup.original.{name}");
        if local.values.contains_key(&original) {
            return Err(err("cleanup alias collision"));
        }
        let value = local.values.remove(name).unwrap();
        local.values.insert(original.clone(), value);
        let computing = field(pair, "computing")?.clone();
        let mut fresh = arr(&computing)?.to_vec();
        fresh.push(Json::obj(vec![
            ("invoke", Json::Str("bind.qxor".into())),
            (
                "args",
                Json::Arr(vec![
                    Json::u64(width as u64),
                    Json::Str(name.into()),
                    Json::Str(original.clone()),
                ]),
            ),
        ]));
        fresh.push(Json::obj(vec![
            ("inverse", Json::Bool(true)),
            ("body", computing),
        ]));
        let body = Json::Arr(vec![
            Json::obj(vec![
                ("declare", Json::Str(name.into())),
                ("width", Json::u64(width as u64)),
                ("class", Json::obj(vec![("class", Json::Str(class))])),
                ("state", Json::Str("clean".into())),
                ("body", Json::Arr(fresh)),
            ]),
            Json::obj(vec![("relinquish", Json::Str(original))]),
        ]);
        self.walk(&body, &mut local, out, next, active)
    }
}
