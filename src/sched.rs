//! ISA §11 consumer validation. No private substitute for the unpublished ref library.
use crate::{
    identity::Identities,
    json::{self, Json},
    object::Object,
    target::{self, Rational as R},
};
use target::{arr, field, keys, string};
fn error(s: impl std::fmt::Display) -> String {
    format!("§11: E_SCHEDULE_INFEASIBLE: {s}")
}
/// Oracle coordinates at one invocation level. Non-step support has no step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coordinate {
    pub step: Option<usize>,
    pub phase: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub symbol: String,
    pub point: String,
    pub iteration: Vec<usize>,
    pub instance: Vec<usize>,
    pub call_index: Option<usize>,
    /// Coordinates of an entered oracle, used only when this is a proper prefix.
    /// Action calls have None and preserve the containing oracle level.
    pub oracle: Option<Coordinate>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OperationKind {
    #[default]
    Gate,
    Alloc,
    Release,
    Classical,
    Start,
    Cleanup,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operation {
    pub path: Vec<Point>,
    /// Coordinates at the root level, never substituted for a nested budget.
    pub step: Option<usize>,
    pub phase: String,
    pub generated: usize,
    pub kind: OperationKind,
}
#[derive(Clone, Debug)]
pub struct PathSelector {
    pub symbol: String,
    pub point: String,
    pub iteration: Option<Vec<usize>>,
    pub instance: Option<Vec<usize>>,
    pub call_index: Option<usize>,
}
impl From<(String, String, Option<Vec<usize>>)> for PathSelector {
    fn from((symbol, point, iteration): (String, String, Option<Vec<usize>>)) -> Self {
        Self {
            symbol,
            point,
            iteration,
            instance: None,
            call_index: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Selector {
    pub path: Vec<PathSelector>,
    pub step: Option<usize>,
    pub phase: Option<String>,
    pub generated: Option<usize>,
}
impl Selector {
    pub fn specificity(&self) -> (usize, usize, bool, bool, bool) {
        (
            self.path.len(),
            self.path.iter().filter(|p| p.iteration.is_some()).count(),
            self.step.is_some(),
            self.phase.is_some(),
            self.generated.is_some(),
        )
    }
    fn matches(&self, op: &Operation) -> bool {
        let entered = op
            .path
            .iter()
            .take(self.path.len().min(op.path.len().saturating_sub(1)))
            .filter_map(|p| p.oracle.as_ref())
            .next_back();
        let step = entered.map_or(op.step, |c| c.step);
        let phase = entered.map_or(op.phase.as_str(), |c| c.phase.as_str());
        self.path.len() <= op.path.len()
            && self.path.iter().zip(&op.path).all(|(p, q)| {
                p.symbol == q.symbol
                    && p.point == q.point
                    && p.iteration.as_ref().is_none_or(|i| i == &q.iteration)
                    && p.instance.as_ref().is_none_or(|i| i == &q.instance)
                    && p.call_index.is_none_or(|i| q.call_index == Some(i))
            })
            && self.step.is_none_or(|s| step == Some(s))
            && self.phase.as_ref().is_none_or(|p| p == phase)
            && self.generated.is_none_or(|g| g == op.generated)
    }
    pub fn parse(e: &Json) -> Result<Self, String> {
        let mut path = vec![];
        for p in arr(field(e, "path")?)? {
            keys(
                p,
                &["symbol", "point", "iteration", "instance", "call_index"],
            )?;
            let iteration = field(p, "iteration")?;
            let iter = if iteration.as_str() == Some("*") {
                None
            } else {
                Some(
                    arr(iteration)?
                        .iter()
                        .map(|v| v.as_usize().ok_or(error("invalid iteration")))
                        .collect::<Result<_, _>>()?,
                )
            };
            let instance = p.get("instance").map(wild_array).transpose()?.flatten();
            let call_index = p.get("call_index").map(wild_int).transpose()?.flatten();
            path.push(PathSelector {
                symbol: string(field(p, "symbol")?)?.into(),
                point: string(field(p, "point")?)?.into(),
                iteration: iter,
                instance,
                call_index,
            });
        }
        let step = wild_int(field(e, "step")?)?;
        if step == Some(0) {
            return Err(error("oracle steps start at 1"));
        }
        let phase = match string(field(e, "phase")?)? {
            "*" => None,
            p @ ("fwd" | "rev") => Some(p.into()),
            _ => return Err(error("invalid phase")),
        };
        Ok(Self {
            path,
            step,
            phase,
            generated: wild_int(field(e, "generated")?)?,
        })
    }
}
fn wild_array(v: &Json) -> Result<Option<Vec<usize>>, String> {
    if v.as_str() == Some("*") {
        return Ok(None);
    }
    arr(v)?
        .iter()
        .map(|v| {
            v.as_usize()
                .ok_or(error("expected nonnegative integer array"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}
/// Select one decision kind, preserving original array positions. Start applies
/// to emitted leaves only; a cleanup carrier never enters the timing stream.
pub fn select_kind(
    entries: &[(OperationKind, Selector)],
    op: &Operation,
    kind: OperationKind,
) -> Result<Option<usize>, String> {
    if (op.kind == OperationKind::Cleanup && kind != OperationKind::Cleanup)
        || (op.kind != OperationKind::Cleanup && kind == OperationKind::Cleanup)
        || (kind != OperationKind::Start && kind != op.kind)
    {
        return Ok(None);
    }
    let applicable = entries
        .iter()
        .enumerate()
        .filter(|(_, (k, _))| *k == kind)
        .collect::<Vec<_>>();
    let selectors = applicable
        .iter()
        .map(|(_, (_, s))| s.clone())
        .collect::<Vec<_>>();
    select(&selectors, op).map(|selected| selected.map(|i| applicable[i].0))
}
fn wild_int(v: &Json) -> Result<Option<usize>, String> {
    if v.as_str() == Some("*") {
        Ok(None)
    } else {
        Ok(Some(v.as_usize().ok_or(error("expected integer or *"))?))
    }
}
pub fn select(entries: &[Selector], op: &Operation) -> Result<Option<usize>, String> {
    let maximal = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.matches(op))
        .max_by_key(|(_, e)| e.specificity());
    let Some((index, best)) = maximal else {
        return Ok(None);
    };
    if entries
        .iter()
        .filter(|e| e.matches(op) && e.specificity() == best.specificity())
        .count()
        > 1
    {
        return Err(error("multiple maximal matching entries"));
    }
    Ok(Some(index))
}
pub fn epsilon_share(epsilon: &R, positive_error_operations: usize) -> Result<R, String> {
    if epsilon.negative() || epsilon > &R::from(1) {
        return Err(error("epsilon outside [0,1]"));
    }
    if positive_error_operations == 0 {
        return Ok(R::zero());
    }
    epsilon.div(&R::from(positive_error_operations as u64))
}
pub fn validate_start(start: &R, dependencies: &[R]) -> Result<(), String> {
    if start.negative() || dependencies.iter().any(|end| start < end) {
        return Err(error("start precedes dependency end"));
    }
    Ok(())
}
/// §17.7 defaults missing expansion bindings to ref at ISA version 0.8.
pub fn require_library(expansion: Option<&Json>) -> Result<(), String> {
    let name = expansion
        .and_then(|e| e.get("name"))
        .and_then(Json::as_str)
        .unwrap_or("ref");
    let version = expansion
        .and_then(|e| e.get("version"))
        .and_then(Json::as_str)
        .unwrap_or("0.8");
    Err(error(format!(
        "unimplemented: expansion library {name}@{version} (milestone 4); no published operation/ordinal/scratch/removability document is available (b-isa-spec-8r4; bennett-vm-47t)"
    )))
}
/// K57 binds an omitted expansion to the object's own ISA version.
/// This authenticates availability only; it does not admit a realised schedule.
pub fn require_library_version(expansion: Option<&Json>, isa: &str) -> Result<(), String> {
    if isa != "0.9" {
        return require_library(expansion);
    }
    if let Some(e) = expansion {
        keys(e, &["name", "version", "hash"])?;
        for (k, expected) in [
            ("name", "ref"),
            ("version", "0.9"),
            ("hash", crate::ref_library::HASH),
        ] {
            if field(e, k)?.as_str() != Some(expected) {
                return Err(error(format!("expansion {k} mismatch")));
            }
        }
    }
    crate::ref_library::Catalog::published().map(|_| ())
}
pub fn object_schedule(obj: &Object) -> Result<Option<Json>, String> {
    if !obj.has_schedule {
        return Ok(None);
    }
    let raw = obj
        .text
        .split_once("section schedule\n")
        .ok_or(error("schedule section missing"))?
        .1
        .lines()
        .next()
        .ok_or(error("schedule line missing"))?;
    json::parse(raw).map(Some)
}
struct SourcePoint<'a> {
    instruction: &'a crate::object::Instr,
    loops: Vec<usize>,
}
fn points(f: &crate::object::Function) -> Vec<(String, SourcePoint<'_>)> {
    fn walk<'a>(v: &'a [crate::object::Instr], loops: &[usize], out: &mut Vec<SourcePoint<'a>>) {
        for i in v {
            out.push(SourcePoint {
                instruction: i,
                loops: loops.to_vec(),
            });
            let mut nested = loops.to_vec();
            if i.op == "qloop" {
                // Source admission has already checked the canonical literal.
                nested.push(i.operands[0].parse().expect("admitted qloop bound"));
            }
            for arm in &i.nested {
                walk(arm, &nested, out);
            }
        }
    }
    let mut out = vec![];
    for b in &f.blocks {
        let mut flat = vec![];
        walk(&b.instrs, &[], &mut flat);
        out.extend(
            flat.into_iter()
                .enumerate()
                .map(|(i, p)| (format!("{}:{i}", b.name), p)),
        );
    }
    out
}
/// Only semantic region calls open an invocation edge. In particular neither
/// lift.call, recipe composition nor a symbol-like operand on another row does.
fn semantic_targets(i: &crate::object::Instr) -> Vec<&str> {
    if [
        "action_call",
        "oracle_call",
        "adj",
        "ctl",
        "lift.oracle_call",
        "lift.oracle_uncall",
        "lift.perm_call",
        "qinplace",
    ]
    .contains(&i.op.as_str())
    {
        i.operands
            .iter()
            .filter_map(|s| s.strip_prefix('@'))
            .collect()
    } else {
        vec![]
    }
}
fn validate_path(
    selector: &Selector,
    doc: &Json,
    object: &Object,
    ids: &Identities,
    bound: bool,
) -> Result<(), String> {
    use crate::object::FnKind;
    let mut oracle = doc
        .get("bindings")
        .and_then(|b| b.get("region"))
        .and_then(Json::as_str)
        .filter(|n| object.function(n).is_some_and(|f| f.kind == FnKind::Oracle));
    for (i, p) in selector.path.iter().enumerate() {
        let name = ids
            .interface
            .iter()
            .find(|(_, h)| **h == p.symbol)
            .map(|(n, _)| n)
            .ok_or(error("path symbol is not in object"))?;
        let f = object
            .function(name)
            .ok_or(error("path symbol is not a function"))?;
        if f.kind == FnKind::Oracle {
            oracle = Some(name);
        }
        if let Some(key) = &p.instance {
            if f.kind == FnKind::Lproc && !bound {
                let root = oracle.ok_or(error("lproc instance has no enclosing oracle"))?;
                if !crate::instances::for_oracle(object, root)?
                    .iter()
                    .any(|v| v.symbol == *name && v.key == *key)
                {
                    return Err(error("instance key is not formed in enclosing oracle"));
                }
            } else if f.kind != FnKind::Lproc && !key.is_empty() {
                return Err(error("instance must be [] outside lproc"));
            }
        }
        if p.point.starts_with('$') {
            if f.kind != FnKind::Oracle
                || !["$construct", "$step", "$commit", "$release"].contains(&p.point.as_str())
            {
                return Err(error("invalid reserved support point"));
            }
            if p.iteration.as_ref().is_some_and(|v| !v.is_empty())
                || p.call_index.is_some()
                || i + 1 < selector.path.len()
            {
                return Err(error(
                    "support point has no source iterations or callee edge",
                ));
            }
            continue;
        }
        let source_points = points(f);
        let point = source_points
            .iter()
            .find(|(n, _)| *n == p.point)
            .map(|(_, p)| p)
            .ok_or_else(|| error(format!("path point {} is not in @{name}", p.point)))?;
        if let Some(it) = &p.iteration {
            if it.len() != point.loops.len() {
                return Err(error("iteration length differs from loop nesting depth"));
            }
            if it.iter().zip(&point.loops).any(|(i, n)| i >= n) {
                return Err(error("iteration outside source qloop domain"));
            }
        }
        if bound {
            continue;
        }
        let targets = semantic_targets(point.instruction);
        if p.call_index.is_some_and(|n| n >= targets.len()) {
            return Err(error("call_index outside semantic-call domain"));
        }
        if let Some(next) = selector.path.get(i + 1)
            && !targets
                .iter()
                .enumerate()
                .filter(|(n, _)| p.call_index.is_none_or(|c| c == *n))
                .any(|(_, callee)| ids.interface.get(*callee) == Some(&next.symbol))
        {
            return Err(error("path is not an invocation chain"));
        }
    }
    Ok(())
}
/// Preliminary source checks. Concrete 0.9 consumers additionally validate the
/// bound library's occurrence domains with `validate_bound_entries`.
pub fn validate_entries(doc: &Json, object: &Object, ids: &Identities) -> Result<(), String> {
    validate_entries_mode(doc, object, ids, None)
}
fn validate_entries_mode(
    doc: &Json,
    object: &Object,
    ids: &Identities,
    calls: Option<&crate::ref_library::BoundCalls>,
) -> Result<(), String> {
    let check = || -> Result<(), String> {
        keys(doc, &["bindings", "entries"])?;
        for e in arr(field(doc, "entries")?)? {
            keys(
                e,
                &[
                    "path",
                    "step",
                    "phase",
                    "generated",
                    "kind",
                    "cleanup",
                    "gate",
                    "alloc",
                    "start",
                ],
            )?;
            let selector = Selector::parse(e)?;
            let kind = string(field(e, "kind")?)?;
            if !["cleanup", "gate", "alloc", "start"].contains(&kind) {
                return Err(error("unknown entry kind"));
            }
            for key in ["cleanup", "gate", "alloc", "start"] {
                if e.get(key).is_some() && key != kind {
                    return Err(error("entry payload differs from kind"));
                }
            }
            let payload = field(e, kind)?;
            match kind {
                "cleanup" => {
                    keys(payload, &["realisation"])?;
                    if !["reverse", "recompute"].contains(&string(field(payload, "realisation")?)?)
                    {
                        return Err(error("invalid cleanup realisation"));
                    }
                }
                "gate" => {
                    keys(payload, &["realisation", "epsilon_share"])?;
                    string(field(payload, "realisation")?)?;
                    let eps = target::decimal(field(payload, "epsilon_share")?)?;
                    if eps.negative() || eps > R::from(1) {
                        return Err(error("epsilon_share outside [0,1]"));
                    }
                }
                "alloc" => {
                    keys(payload, &["class"])?;
                    if string(field(payload, "class")?)? == "any" {
                        return Err(error("schedule allocation class cannot be any"));
                    }
                }
                "start" => {
                    keys(payload, &["time"])?;
                    if target::decimal(field(payload, "time")?)?.negative() {
                        return Err(error("negative start"));
                    }
                }
                _ => unreachable!(),
            }
            validate_path(&selector, doc, object, ids, calls.is_some())?;
            if let Some(calls) = calls {
                validate_bound_path(&selector, doc, object, ids, calls)?;
            }
        }
        Ok(())
    };
    check().map_err(|s| {
        if s.contains("E_SCHEDULE_INFEASIBLE") {
            s
        } else {
            error(s)
        }
    })
}

/// Validate concrete source paths against a bound library expansion.
pub fn validate_bound_entries(
    doc: &Json,
    object: &Object,
    ids: &Identities,
    calls: &crate::ref_library::BoundCalls,
) -> Result<(), String> {
    let region = string(field(field(doc, "bindings")?, "region")?)?;
    if !calls.is_for(object, region) {
        return Err(error(
            "semantic-call domains are not bound to this object and region",
        ));
    }
    validate_entries_mode(doc, object, ids, Some(calls))
}

fn path_matches(path: &[PathSelector], points: &[Point], indices: bool) -> bool {
    path.len() <= points.len()
        && path.iter().zip(points).all(|(p, q)| {
            p.symbol == q.symbol
                && p.point == q.point
                && p.iteration.as_ref().is_none_or(|v| v == &q.iteration)
                && p.instance.as_ref().is_none_or(|v| v == &q.instance)
                && (!indices || p.call_index.is_none_or(|v| q.call_index == Some(v)))
        })
}

fn validate_bound_path(
    selector: &Selector,
    doc: &Json,
    object: &Object,
    ids: &Identities,
    calls: &crate::ref_library::BoundCalls,
) -> Result<(), String> {
    if selector.path.is_empty() {
        return Ok(());
    }
    let root = string(field(field(doc,"bindings")?,"region")?)?;
    let first = &selector.path[0];
    let mut rooted = ids.interface.get(root) == Some(&first.symbol);
    if !rooted && object.function(root).is_some_and(|f|f.kind == crate::object::FnKind::Oracle) {
        rooted = crate::instances::for_oracle(object,root)?.iter().any(|instance|
            ids.interface.get(&instance.symbol) == Some(&first.symbol)
            && first.instance.as_ref().is_none_or(|key| key == &instance.key));
    }
    if !rooted { return Err(error("path is not rooted in the bound invocation")); }
    let mut owner_exists = false;
    for domain in calls.domains() {
        if path_matches(&selector.path, &domain.path, false) {
            owner_exists = true;
        }
        if path_matches(&selector.path, &domain.path, true) {
            return Ok(());
        }
        for index in 0..domain.targets.len() {
            let mut edge = domain.path.clone();
            if let Some(point) = edge.last_mut() {
                point.call_index = Some(index);
            }
            if path_matches(&selector.path, &edge, true) {
                return Ok(());
            }
        }
    }
    if owner_exists {
        return Err(error("call_index outside bound semantic-call domain"));
    }
    // A source point in a zero-trip loop emits no domain. Its structural path
    // remains subject to source/instance checking; absence is not a tie witness.
    validate_path(selector, doc, object, ids, false)
}
pub struct Bindings<'a> {
    pub region: &'a str,
    pub variant: &'a str,
    pub inherited_controls: u32,
    pub budget: [u64; 4],
    pub params: &'a Json,
    pub epsilon: &'a R,
}
pub fn consume(
    doc: &Json,
    object: &Object,
    ids: &Identities,
    target: &crate::target::Target,
    b: &Bindings<'_>,
) -> Result<(), String> {
    validate_entries(doc, object, ids)?;
    let check = || -> Result<(), String> {
        let binding = field(doc, "bindings")?;
        keys(
            binding,
            &[
                "closure_hash",
                "target_hash",
                "region",
                "variant",
                "inherited_controls",
                "budget",
                "params_hash",
                "epsilon",
                "expansion",
                "scheduler",
            ],
        )?;
        let closure = ids
            .closure
            .get(b.region)
            .and_then(Option::as_ref)
            .ok_or(error("region closure unavailable"))?;
        for (k, expected) in [
            ("closure_hash", closure.as_str()),
            ("target_hash", target.hash.as_str()),
            ("region", b.region),
            ("variant", b.variant),
        ] {
            if string(field(binding, k)?)? != expected {
                return Err(error(format!("binding {k} mismatch")));
            }
        }
        if field(binding, "inherited_controls")?.as_u64() != Some(b.inherited_controls as u64) {
            return Err(error("binding inherited_controls mismatch"));
        }
        let budget = arr(field(binding, "budget")?)?;
        if budget.len() != 4
            || budget
                .iter()
                .zip(b.budget)
                .any(|(v, n)| v.as_u64() != Some(n))
        {
            return Err(error("binding budget mismatch"));
        }
        if target::decimal(field(binding, "epsilon")?)? != *b.epsilon {
            return Err(error("binding epsilon mismatch"));
        }
        if string(field(binding, "params_hash")?)?
            != crate::sha256::sha256_hex(b.params.to_jcs().as_bytes())
        {
            return Err(error("binding params_hash mismatch"));
        }
        let scheduler = field(binding, "scheduler")?;
        keys(scheduler, &["name", "version"])?;
        string(field(scheduler, "name")?)?;
        string(field(scheduler, "version")?)?;
        if let Some(expansion) = binding.get("expansion") {
            keys(expansion, &["name", "version", "hash"])?;
            for k in ["name", "version", "hash"] {
                string(field(expansion, k)?)?;
            }
        }
        require_library(binding.get("expansion"))
    };
    check().map_err(|s| {
        if s.contains("E_SCHEDULE_INFEASIBLE") {
            s
        } else {
            error(s)
        }
    })
}
