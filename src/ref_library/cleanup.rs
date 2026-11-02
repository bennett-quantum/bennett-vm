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
    pub calls: BoundCalls,
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
                "unavailable.reason: exact whole-source cleanup pairing remains staged in bennett-vm-47t-c",
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
        let resolved;
        let pair = if pair.get("input_request").is_some() {
            resolved = env
                .source_history
                .as_ref()
                .ok_or_else(|| err("missing source history"))?
                .resolve_call_inputs(pair, env)?;
            &resolved
        } else {
            pair
        };
        if field(pair, "perm")? != &Json::Bool(true) {
            return Err(err("recompute requires a permutation computing sequence"));
        }
        if pair.get("recompute_supported") == Some(&Json::Bool(false)) {
            return Err(err(
                "unavailable.reason: nonempty call cleanup needs its exact clean-input computing sequence",
            ));
        }
        let name = string(field(pair, "target")?)?;
        if name.is_empty() {
            return self.walk_call_recompute(pair, env, out, next, active);
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
        local.replaying_cleanup = true;
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
        fresh.extend(arr(field(pair, "uncomputing")?)?.iter().cloned());
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

    fn walk_call_recompute(
        &self,
        pair: &Json,
        env: &Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<(), String> {
        let mut local = env.clone();
        local.replaying_cleanup = true;
        let mut targets = vec![];
        for name in arr(field(pair, "targets")?)? {
            let name = string(name)?;
            let (base, suffix) = name
                .split_once('[')
                .map_or((name, String::new()), |(base, sub)| {
                    (base, format!("[{sub}"))
                });
            let width = wires(&Json::Str(name.into()), env)?.len();
            let alias = format!("$cleanup.original.{base}");
            if let Some(value) = local.values.remove(base) {
                if local.values.insert(alias.clone(), value).is_some() {
                    return Err(err("call cleanup alias collision"));
                }
            } else if !local.values.contains_key(&alias) {
                return Err(err("call recompute target is unbound"));
            }
            targets.push((name.to_string(), format!("{alias}{suffix}"), width));
        }
        let xor = |targets: &[(String, String, usize)]| {
            targets
                .iter()
                .map(|(name, original, width)| {
                    Json::obj(vec![
                        ("invoke", Json::Str("bind.qxor".into())),
                        (
                            "args",
                            Json::Arr(vec![
                                Json::u64(*width as u64),
                                Json::Str(name.clone()),
                                Json::Str(original.clone()),
                            ]),
                        ),
                    ])
                })
                .collect::<Vec<_>>()
        };
        let mut body = arr(field(pair, "initialising")?)?.to_vec();
        body.extend(arr(field(pair, "computing")?)?.iter().cloned());
        body.extend(xor(&targets));
        body.extend(arr(field(pair, "uncomputing")?)?.iter().cloned());
        // The call's adjoint restores its exact pre-call value, which need
        // not be zero. Restore it before uncomputing the fresh input history.
        body.extend(xor(&targets));
        body.extend(arr(field(pair, "uninitialising")?)?.iter().cloned());
        for allocation in arr(field(pair, "allocations")?)?.iter().rev() {
            local.values.remove(string(field(allocation, "name")?)?);
            body = vec![Json::obj(vec![
                ("declare", field(allocation, "name")?.clone()),
                ("width", field(allocation, "width")?.clone()),
                (
                    "class",
                    Json::obj(vec![("class", field(allocation, "class")?.clone())]),
                ),
                ("state", Json::Str("clean".into())),
                ("body", Json::Arr(body)),
            ])];
        }
        self.walk(&Json::Arr(body), &mut local, out, next, active)
    }
}
