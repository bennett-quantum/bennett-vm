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
#[derive(Clone, Debug)]
pub struct Point {
    pub symbol: String,
    pub point: String,
    pub iteration: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct Operation {
    pub path: Vec<Point>,
    pub step: Option<usize>,
    pub phase: String,
    pub generated: usize,
}
#[derive(Clone, Debug)]
pub struct Selector {
    pub path: Vec<(String, String, Option<Vec<usize>>)>,
    pub step: Option<usize>,
    pub phase: Option<String>,
    pub generated: Option<usize>,
}
impl Selector {
    pub fn specificity(&self) -> (usize, usize, bool, bool, bool) {
        (
            self.path.len(),
            self.path.iter().filter(|p| p.2.is_some()).count(),
            self.step.is_some(),
            self.phase.is_some(),
            self.generated.is_some(),
        )
    }
    fn matches(&self, op: &Operation) -> bool {
        self.path.len() <= op.path.len()
            && self.path.iter().zip(&op.path).all(|(p, q)| {
                p.0 == q.symbol && p.1 == q.point && p.2.as_ref().is_none_or(|i| i == &q.iteration)
            })
            && self.step.is_none_or(|s| op.step == Some(s))
            && self.phase.as_ref().is_none_or(|p| p == &op.phase)
            && self.generated.is_none_or(|g| g == op.generated)
    }
    pub fn parse(e: &Json) -> Result<Self, String> {
        let mut path = vec![];
        for p in arr(field(e, "path")?)? {
            keys(p, &["symbol", "point", "iteration"])?;
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
            path.push((
                string(field(p, "symbol")?)?.into(),
                string(field(p, "point")?)?.into(),
                iter,
            ));
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
        "unimplemented: expansion library {name}@{version} (milestone 4); no published operation/ordinal/scratch/removability document is available (bennett-vm-47t)"
    )))
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
fn points(obj: &Object, name: &str) -> Vec<(String, usize)> {
    fn walk(v: &[crate::object::Instr], depth: usize, out: &mut Vec<usize>) {
        for i in v {
            out.push(depth);
            for arm in &i.nested {
                walk(arm, depth + usize::from(i.op == "qloop"), out);
            }
        }
    }
    let mut out = vec![];
    for b in &obj.function(name).unwrap().blocks {
        let mut depths = vec![];
        walk(&b.instrs, 0, &mut depths);
        out.extend(
            depths
                .into_iter()
                .enumerate()
                .map(|(i, d)| (format!("{}:{i}", b.name), d)),
        );
    }
    out
}
pub fn validate_entries(doc: &Json, object: &Object, ids: &Identities) -> Result<(), String> {
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
            for (i, (symbol, point, it)) in selector.path.iter().enumerate() {
                let name = ids
                    .interface
                    .iter()
                    .find(|(_, h)| *h == symbol)
                    .map(|(n, _)| n)
                    .or_else(|| {
                        ids.closure
                            .iter()
                            .find(|(_, h)| h.as_ref() == Some(symbol))
                            .map(|(n, _)| n)
                    })
                    .ok_or(error("path symbol is not in object"))?;
                let depth = points(object, name)
                    .into_iter()
                    .find(|p| &p.0 == point)
                    .map(|p| p.1)
                    .ok_or_else(|| error(format!("path point {point} is not in @{name}")))?;
                if it.as_ref().is_some_and(|it| it.len() != depth) {
                    return Err(error("iteration length differs from loop nesting depth"));
                }
                if i + 1 < selector.path.len() {
                    let next = &selector.path[i + 1].0;
                    let (block, ordinal) = point.rsplit_once(':').unwrap();
                    let b = object.function(name).unwrap().block(block).unwrap();
                    let mut flat = vec![];
                    fn flatten<'a>(
                        v: &'a [crate::object::Instr],
                        out: &mut Vec<&'a crate::object::Instr>,
                    ) {
                        for i in v {
                            out.push(i);
                            for arm in &i.nested {
                                flatten(arm, out);
                            }
                        }
                    }
                    flatten(&b.instrs, &mut flat);
                    let ins = flat[ordinal.parse::<usize>().unwrap()];
                    if !ins
                        .operands
                        .iter()
                        .filter_map(|s| s.strip_prefix('@'))
                        .any(|callee| {
                            ids.interface.get(callee) == Some(next)
                                || ids.closure.get(callee).and_then(Option::as_ref) == Some(next)
                        })
                    {
                        return Err(error("path is not an invocation chain"));
                    }
                }
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
