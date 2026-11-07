//! Source formation from this consumer's parsed objects and the published ISA.
use super::*;
use crate::object::{FnKind, Instr, Object, Type};
mod partition;

fn text(s: impl Into<String>) -> Json {
    Json::Str(s.into())
}
fn num(n: usize) -> Json {
    Json::u64(n as u64)
}
fn invocation(kind: &str, args: Vec<Json>) -> Json {
    Json::obj(vec![("invoke", text(kind)), ("args", Json::Arr(args))])
}
fn inverse(body: Vec<Json>) -> Json {
    Json::obj(vec![
        ("inverse", Json::Bool(true)),
        ("body", Json::Arr(body)),
    ])
}
#[derive(Clone, Debug)]
struct Region {
    address: usize,
    cell: usize,
    role: String,
    local: bool,
}
struct WholePair {
    cleanup_end: usize,
    carrier: usize,
    computing: Vec<usize>,
    entry_point: usize,
    bundle: Option<Vec<String>>,
    outside: Vec<usize>,
    target: String,
    perm: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Source {
    symbol: String,
    points: BTreeMap<usize, String>,
    widths: BTreeMap<String, usize>,
    regions: BTreeMap<String, Region>,
    declarations: Vec<(String, usize, String, String)>,
    locals: Vec<Instr>,
    computations: BTreeMap<String, Vec<Instr>>,
    cleanup_sites: BTreeMap<String, Vec<usize>>,
    history: Vec<Instr>,
    images: BTreeMap<String, String>,
    readonly: BTreeSet<String>,
    history_replay: bool,
    forbidden_witnesses: BTreeSet<String>,
}
struct HistoryDomain<'a> {
    iterations: &'a [usize],
    branches: BTreeMap<String, usize>,
}
struct HistoryChange {
    row: Instr,
    predicates: Vec<String>,
}
struct HistoryStep {
    row: Instr,
    predicates: Vec<String>,
    rollback: Vec<HistoryChange>,
    snapshot_controls: BTreeSet<String>,
    live_versions: BTreeSet<String>,
}
struct CallInputs {
    rows: Vec<Instr>,
    // Preserve the original compact replay when no historical external
    // version is needed. Otherwise each row owns its version workspace.
    versioned: Option<Vec<HistoryStep>>,
}
impl Catalog {
    /// Form a structured source action from resolved classical parameters,
    /// retaining arbitrary symbolic interface inputs. This boundary precedes
    /// invocation-address assignment and target/schedule realisation.
    pub fn lower_action(
        &self,
        object: &Object,
        name: &str,
        params: &Json,
        adjoint: bool,
        inherited_controls: usize,
    ) -> Result<Vec<Leaf>, String> {
        self.lower_action_with_cleanup(object, name, params, adjoint, inherited_controls, &[])
            .map(|e| e.leaves)
    }
    /// Select whole explicit cleanup pairs before expanding them. This source
    /// trace boundary does not claim target realisation or schedule admission.
    /// Direct copy/load pairs, target-local written inverse sequences and
    /// matched action/adjoint calls retain whole-pair carriers. Source patterns
    /// not yet bound leave a marked default trace; selecting an unavailable
    /// recompute reports its remaining computing-sequence obligation.
    pub fn lower_action_with_cleanup(
        &self,
        object: &Object,
        name: &str,
        params: &Json,
        adjoint: bool,
        inherited_controls: usize,
        entries: &[CleanupEntry],
    ) -> Result<ActionExpansion, String> {
        let mut env = Bindings::new();
        env.link(object)?;
        // Root formation has the same source authentication boundary as a
        // nested call: caller-owned AST fields cannot retarget the text's hash.
        let registry = env.registry.clone().unwrap();
        let object = &registry.object;
        let f = object
            .function(name)
            .ok_or_else(|| err(format!("unresolved action @{name}")))?;
        if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
            return Err(err("source is not an action"));
        }
        let types = f.lists[3].iter().map(|p| p.ty.clone()).collect::<Vec<_>>();
        crate::invocation::decode_args(params, &types, object.ptr_width)?;
        env.cleanup.entries = Rc::new(entries.to_vec());
        for p in f.lists[0].iter().chain(&f.lists[1]) {
            let Type::Wire(width) = p.ty else {
                return Err(err("non-wire source interface"));
            };
            env.wire(&p.name, width);
        }
        for p in &f.lists[2] {
            let Type::Region(address, cell) = p.ty else {
                return Err(err("non-region memory interface"));
            };
            let region = Region {
                address,
                cell,
                role: p.role.clone().ok_or_else(|| err("region role missing"))?,
                local: false,
            };
            let width = region_width(&region)?;
            env.wire(&p.name, width);
            let cells = wires(&text(&p.name), &env)?;
            env.values.insert(
                p.name.clone(),
                Bound::Region {
                    address,
                    cell,
                    role: region.role,
                    origin: format!("interface:{}", p.name),
                    cells,
                },
            );
        }
        for (p, value) in f.lists[3].iter().zip(arr(params)?) {
            let ty = match p.ty {
                Type::Ptr => Json::obj(vec![("ptr", num(object.ptr_width))]),
                _ => text(p.ty.signature_text()),
            };
            env.values
                .insert(p.name.clone(), Bound::Value(value.clone(), ty));
        }
        env.wire("$controls", inherited_controls);
        env.add_controls(wires(&text("$controls"), &env)?);
        let body = action_body(object, name, adjoint, &mut env)?;
        let (leaves, mut calls) = self.expand_with_calls("adj", &[body], &env)?;
        calls.region = Some(name.into());
        let cleanups = env.cleanup.carriers.borrow().clone();
        Ok(ActionExpansion {
            leaves,
            calls,
            cleanups,
            cleanup_pairing_complete: !env.cleanup.incomplete.get(),
        })
    }
}
pub(super) fn interface(object: &Object, name: &str) -> Result<Vec<Json>, String> {
    let f = object
        .function(name)
        .ok_or_else(|| err("linked symbol is not a function"))?;
    let mut fs = vec![];
    let groups: &[usize] = match f.kind {
        FnKind::Oracle => &[0, 1, 2, 3, 4],
        FnKind::Action | FnKind::PermAction => &[0, 1, 2, 3],
        _ => return Err(err("symbol is not a coherent region")),
    };
    for group in groups {
        for p in &f.lists[*group] {
            let mut formal = vec![("name", text(&p.name))];
            match p.ty {
                Type::Wire(width) => formal.extend([("type", text("wire")), ("width", num(width))]),
                Type::Region(address, cell) => formal.extend([
                    ("type", text("region")),
                    ("address", num(address)),
                    ("cell", num(cell)),
                    (
                        "role",
                        text(p.role.as_ref().ok_or_else(|| err("region role missing"))?),
                    ),
                ]),
                _ => formal.extend([
                    ("type", text("value")),
                    (
                        "isa_type",
                        if p.ty == Type::Ptr {
                            Json::obj(vec![("ptr", num(object.ptr_width))])
                        } else {
                            text(p.ty.signature_text())
                        },
                    ),
                ]),
            }
            fs.push(Json::obj(formal));
        }
    }
    Ok(fs)
}
fn region_width(r: &Region) -> Result<usize, String> {
    u32::try_from(r.address)
        .ok()
        .and_then(|a| 1usize.checked_shl(a))
        .and_then(|n| n.checked_mul(r.cell))
        .ok_or_else(|| err("source region width exceeds host limit"))
}
pub(super) fn action_body(
    object: &Object,
    name: &str,
    adjoint: bool,
    env: &mut Bindings,
) -> Result<Json, String> {
    env.source_formed = true;
    let f = object
        .function(name)
        .ok_or_else(|| err("unresolved linked action"))?;
    if f.kind == FnKind::Oracle {
        return super::oracle_source::body(object, name, env);
    }
    if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
        return Err(err(
            "unavailable.reason: linked oracle construction is unavailable",
        ));
    }
    let bound = (object.isa == "0.9").then(|| crate::action::bind_allocations(f));
    let f = bound.as_ref().unwrap_or(f);
    let registry = env
        .registry
        .as_ref()
        .ok_or_else(|| err("source registry missing"))?;
    let symbol = registry
        .identities
        .interface
        .get(name)
        .ok_or_else(|| err("source interface identity missing"))?
        .clone();
    let points = crate::sched::points(f)
        .into_iter()
        .map(|(name, p)| (p.instruction.line, name))
        .collect();
    let mut source = Source {
        symbol,
        points,
        widths: BTreeMap::new(),
        regions: BTreeMap::new(),
        declarations: vec![],
        locals: vec![],
        computations: BTreeMap::new(),
        cleanup_sites: BTreeMap::new(),
        history: vec![],
        images: object.data.clone(),
        readonly: f.lists[1].iter().map(|p| p.name.clone()).collect(),
        history_replay: false,
        forbidden_witnesses: BTreeSet::new(),
    };

    for p in f.lists[0].iter().chain(&f.lists[1]) {
        let Type::Wire(width) = p.ty else {
            return Err(err("non-wire interface"));
        };
        source.widths.insert(p.name.clone(), width);
    }
    for p in &f.lists[2] {
        let Type::Region(address, cell) = p.ty else {
            return Err(err("non-region interface"));
        };
        source.regions.insert(
            p.name.clone(),
            Region {
                address,
                cell,
                role: p.role.clone().ok_or_else(|| err("region role missing"))?,
                local: false,
            },
        );
    }
    let instructions = f
        .blocks
        .iter()
        .flat_map(|b| b.instrs.clone())
        .collect::<Vec<_>>();
    source.history = instructions.clone();
    source.collect_regions(&instructions)?;
    source.declarations(&instructions)?;
    env.source_history = Some(Rc::new(source.clone()));
    let mut body = source.sequence(&instructions, adjoint, env)?;
    for (name, width, class, state) in source.declarations.iter().rev() {
        body = vec![Json::obj(vec![
            ("declare", text(name)),
            ("width", num(*width)),
            ("class", Json::obj(vec![("class", text(class))])),
            ("state", text(state)),
            ("body", Json::Arr(body)),
        ])];
    }
    for local in source.locals.iter().rev() {
        let name = &local.operands[0];
        let region = &source.regions[name];
        let cells = source.region_width(region)?;
        let image = if let Some(image) = local.operands.get(3) {
            object
                .data
                .get(image.trim_start_matches('@'))
                .ok_or_else(|| err("unbound source image"))?
                .clone()
        } else {
            String::new()
        };
        let template = Json::obj(vec![
            (
                "parameters",
                Json::Arr(vec![Json::obj(vec![
                    ("name", text(name)),
                    ("type", text("wire")),
                    ("width", num(cells)),
                ])]),
            ),
            ("body", Json::Arr(body)),
        ]);
        body = vec![Json::obj(vec![
            (
                "$source",
                Json::obj(vec![
                    ("symbol", text(&source.symbol)),
                    ("point", text(&source.points[&local.line])),
                    ("reverse_iterations", Json::Bool(false)),
                ]),
            ),
            (
                "body",
                Json::Arr(vec![invocation(
                    "qregion",
                    vec![
                        num(cells / region.cell),
                        num(region.cell),
                        text(image),
                        Json::Arr(vec![template]),
                        Json::Arr(vec![]),
                    ],
                )]),
            ),
        ])];
    }
    Ok(Json::Arr(body))
}
impl Source {
    fn allocation_metadata(&self, instruction: &Instr, body: Vec<Json>) -> Vec<Json> {
        if !matches!(
            instruction.op.as_str(),
            "qalloc" | "qborrow" | "qcopy" | "qload"
        ) {
            return body;
        }
        vec![Json::obj(vec![
            (
                "$source_allocation",
                Json::obj(vec![
                    ("name", text(&instruction.lhs[0])),
                    (
                        "class",
                        text(if matches!(instruction.op.as_str(), "qalloc" | "qborrow") {
                            instruction.operands[0].as_str()
                        } else {
                            "any"
                        }),
                    ),
                    (
                        "state",
                        text(if instruction.op == "qborrow" {
                            "borrowed"
                        } else {
                            "clean"
                        }),
                    ),
                ]),
            ),
            ("body", Json::Arr(body)),
        ])]
    }
    fn region_width(&self, r: &Region) -> Result<usize, String> {
        u32::try_from(r.address)
            .ok()
            .and_then(|a| 1usize.checked_shl(a))
            .and_then(|n| n.checked_mul(r.cell))
            .ok_or_else(|| err("source region width exceeds host limit"))
    }
    fn width(&self, wire: &str) -> Result<usize, String> {
        if let Some((base, sub)) = wire.split_once('[') {
            let width = *self
                .widths
                .get(base)
                .ok_or_else(|| err(format!("unbound source bundle {base}")))?;
            let bit = sub
                .strip_suffix(']')
                .ok_or_else(|| err("malformed source wire"))?
                .parse::<usize>()
                .map_err(err)?;
            if bit >= width {
                return Err(err("source wire outside width"));
            }
            Ok(1)
        } else {
            self.widths
                .get(wire)
                .copied()
                .ok_or_else(|| err(format!("unbound source bundle {wire}")))
        }
    }
    fn collect_regions(&mut self, instructions: &[Instr]) -> Result<(), String> {
        for i in instructions {
            if i.op == "qregion" {
                let name = i.operands[0].clone();
                let r = Region {
                    address: i.operands[1].parse::<usize>().map_err(err)?,
                    cell: i.operands[2].parse::<usize>().map_err(err)?,
                    role: "mut".into(),
                    local: true,
                };
                if self.regions.insert(name, r).is_some() {
                    return Err(err("duplicate source region declaration"));
                }
                self.locals.push(i.clone());
            }
            for arm in &i.nested {
                self.collect_regions(arm)?;
            }
        }
        Ok(())
    }
    fn declarations(&mut self, instructions: &[Instr]) -> Result<(), String> {
        for i in instructions {
            if matches!(i.op.as_str(), "qcopy" | "qload") {
                self.computations
                    .entry(i.lhs[0].clone())
                    .or_default()
                    .push(i.clone());
            }
            if i.op == "quncopy" {
                self.cleanup_sites
                    .entry(i.operands[1].clone())
                    .or_default()
                    .push(i.line);
            }
            if i.op == "qunload" {
                self.cleanup_sites
                    .entry(i.operands[2].clone())
                    .or_default()
                    .push(i.line);
            }
            let allocation = match i.op.as_str() {
                "qalloc" | "qborrow" => Some((
                    i.operands[1].parse::<usize>().map_err(err)?,
                    i.operands[0].clone(),
                    if i.op == "qborrow" {
                        "borrowed"
                    } else {
                        "clean"
                    },
                )),
                "qcopy" => Some((self.width(&i.operands[0])?, "any".into(), "clean")),
                "qload" => Some((
                    self.regions
                        .get(&i.operands[0])
                        .ok_or_else(|| err("unbound source region"))?
                        .cell,
                    "any".into(),
                    "clean",
                )),
                _ => None,
            };
            if let Some((width, class, state)) = allocation {
                let name = i
                    .lhs
                    .first()
                    .ok_or_else(|| err("allocation result missing"))?;
                if let Some(previous) = self.declarations.iter().find(|(n, ..)| n == name) {
                    if previous.1 != width {
                        return Err(err("loop-carried allocation width differs"));
                    }
                } else {
                    self.widths.insert(name.clone(), width);
                    self.declarations
                        .push((name.clone(), width, class, state.into()));
                }
            }
            for arm in &i.nested {
                self.declarations(arm)?;
            }
        }
        Ok(())
    }
    // Match a whole written inverse at source level before any cleanup leaf
    // is expanded. Keep the original row wrappers for reverse; recompute is
    // newly instantiated at the release point.
    fn inverse_rows(a: &Instr, b: &Instr) -> bool {
        if a.op == b.op && a.operands == b.operands && (a.op == "qif" || a.op == "qloop") {
            return a.nested.len() == b.nested.len()
                && a.nested.iter().zip(&b.nested).all(|(a, b)| {
                    a.len() == b.len()
                        && a.iter()
                            .zip(b.iter().rev())
                            .all(|(a, b)| Self::inverse_rows(a, b))
                });
        }
        if matches!(a.op.as_str(), "action_call" | "adj" | "ctl")
            && matches!(b.op.as_str(), "action_call" | "adj" | "ctl")
        {
            return match (
                crate::action::normalize_call(a),
                crate::action::normalize_call(b),
            ) {
                (Ok((ai, ac, ao, aa)), Ok((bi, bc, bo, ba))) => {
                    ai != bi && ac == bc && ao == bo && aa == ba
                }
                _ => false,
            };
        }
        if a.operands != b.operands || !a.lhs.is_empty() || !b.lhs.is_empty() {
            return false;
        }
        let inverse = match a.op.as_str() {
            "s" => "sdg",
            "sdg" => "s",
            "t" => "tdg",
            "tdg" => "t",
            "x" | "y" | "z" | "h" | "cx" | "cy" | "cz" | "ccx" | "mcx" | "swap" | "qxor"
            | "qexch" => a.op.as_str(),
            _ => return false,
        };
        b.op == inverse
    }
    fn writes_only(i: &Instr, target: &str, env: &Bindings) -> bool {
        if i.op == "qif" || i.op == "qloop" {
            return i
                .nested
                .iter()
                .flatten()
                .all(|i| Self::writes_only(i, target, env));
        }
        let is_target = |name: &str| name == target || name.starts_with(&format!("{target}["));
        if matches!(i.op.as_str(), "swap" | "qexch") {
            return i.operands.iter().all(|n| is_target(n));
        }
        if crate::action::is_gate_name(&i.op) || i.op == "qxor" {
            return i.operands.last().is_some_and(|n| is_target(n));
        }
        if let Ok((_, _, op, args)) = crate::action::normalize_call(i) {
            return op == "action_call"
                && crate::validate::list_operand(&args[1])
                    .is_ok_and(|ns| !ns.is_empty() && ns.iter().all(|n| is_target(n)))
                && env.registry.as_ref().is_some_and(|r| {
                    r.object
                        .function(args[0].trim_start_matches('@'))
                        .is_some_and(|f| f.lists[2].iter().all(|p| p.role.as_deref() == Some("in")))
                });
        }
        false
    }
    fn permutation_row(i: &Instr, env: &Bindings) -> bool {
        if i.op == "qif" || i.op == "qloop" {
            return i
                .nested
                .iter()
                .flatten()
                .all(|i| Self::permutation_row(i, env));
        }
        if matches!(
            i.op.as_str(),
            "x" | "cx" | "ccx" | "mcx" | "swap" | "qxor" | "qexch" | "qstore" | "qexchm"
        ) {
            return true;
        }
        if let Ok((_, _, op, args)) = crate::action::normalize_call(i) {
            return op == "action_call"
                && env.registry.as_ref().is_some_and(|r| {
                    r.object
                        .function(args[0].trim_start_matches('@'))
                        .is_some_and(|f| f.kind == FnKind::PermAction)
                });
        }
        false
    }
    fn mentions(i: &Instr, target: &str) -> bool {
        i.lhs.iter().any(|n| n == target)
            || i.operands.iter().any(|a| {
                crate::validate::list_operand(a)
                    .unwrap_or_else(|_| vec![a.clone()])
                    .iter()
                    .any(|n| n == target || n.starts_with(&format!("{target}[")))
            })
            || i.nested.iter().flatten().any(|i| Self::mentions(i, target))
    }
    fn references(i: &Instr) -> BTreeSet<String> {
        i.operands
            .iter()
            .flat_map(|a| crate::validate::list_operand(a).unwrap_or_else(|_| vec![a.clone()]))
            .map(|n| n.split('[').next().unwrap().to_string())
            .chain(i.nested.iter().flatten().flat_map(Self::references))
            .collect()
    }
    fn outputs(i: &Instr, env: &Bindings) -> Vec<String> {
        match i.op.as_str() {
            "qif" | "qloop" => i
                .nested
                .iter()
                .flatten()
                .flat_map(|i| Self::outputs(i, env))
                .collect(),
            "swap" | "qexch" => i.operands.clone(),
            "qstore" => vec![i.operands[0].clone()],
            "qexchm" => vec![i.operands[0].clone(), i.operands[2].clone()],
            "action_call" | "oracle_call" | "adj" | "ctl" => {
                let Ok((_, _, op, args)) = crate::action::normalize_call(i) else {
                    return vec![];
                };
                if op == "oracle_call" {
                    return args[3..5].to_vec();
                }
                if op != "action_call" {
                    return vec![];
                }
                let mut out = crate::validate::list_operand(&args[1]).unwrap();
                if let Some(f) = env
                    .registry
                    .as_ref()
                    .and_then(|r| r.object.function(args[0].trim_start_matches('@')))
                {
                    out.extend(
                        crate::validate::list_operand(&args[3])
                            .unwrap()
                            .into_iter()
                            .zip(&f.lists[2])
                            .filter(|(_, p)| p.role.as_deref() == Some("mut"))
                            .map(|(n, _)| n),
                    );
                }
                out
            }
            op if crate::action::is_gate_name(op) || op == "qxor" => {
                i.operands.last().cloned().into_iter().collect()
            }
            _ => vec![],
        }
    }
    fn input_history(
        &self,
        rows: &[Instr],
        adjoint: bool,
        env: &mut Bindings,
    ) -> Result<Vec<Json>, String> {
        // Local region creation is hoisted in ordinary source formation. Its
        // fresh history instead prepares/restores the declared image here.
        let mut source = self.clone();
        source.history_replay = true;
        let mut body = vec![];
        let mut chunk = vec![];
        for i in rows {
            if i.op != "qregion" {
                chunk.push(i.clone());
                continue;
            }
            if adjoint {
                chunk.reverse();
            }
            body.extend(source.raw_sequence(&chunk, adjoint, false, env)?);
            chunk.clear();
            let name = &i.operands[0];
            let image = i
                .operands
                .get(3)
                .map(|n| {
                    self.images
                        .get(n.trim_start_matches('@'))
                        .cloned()
                        .ok_or_else(|| err("unbound history image"))
                })
                .transpose()?
                .unwrap_or_default();
            let prepare = vec![
                Json::obj(vec![("acquire", text(name))]),
                Json::obj(vec![("image", text(image)), ("wires", text(name))]),
            ];
            body.extend(prepare);
        }
        if adjoint {
            chunk.reverse();
        }
        body.extend(source.raw_sequence(&chunk, adjoint, false, env)?);
        Ok(body)
    }
    /// Reconstruct earlier interface versions on fresh wires. Each workspace
    /// starts from a coherent copy of the live external values, rolls back the
    /// proven permutation slice, performs one owned-history row, then restores
    /// and releases every snapshot. Live interface/control wires never change.
    fn versioned_input_history(
        &self,
        steps: &[HistoryStep],
        adjoint: bool,
        env: &mut Bindings,
    ) -> Result<Vec<Json>, String> {
        fn rename(text: &str, aliases: &BTreeMap<String, String>) -> String {
            let mut result = String::new();
            let mut start = 0;
            for (offset, c) in text.char_indices() {
                if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$') {
                    continue;
                }
                let token = &text[start..offset];
                result.push_str(if start > 0 && text.as_bytes()[start - 1] == b'@' {
                    token
                } else {
                    aliases.get(token).map_or(token, String::as_str)
                });
                result.push(c);
                start = offset + c.len_utf8();
            }
            let token = &text[start..];
            result.push_str(if start > 0 && text.as_bytes()[start - 1] == b'@' {
                token
            } else {
                aliases.get(token).map_or(token, String::as_str)
            });
            result
        }
        fn renamed(row: &Instr, aliases: &BTreeMap<String, String>) -> Instr {
            let mut row = row.clone();
            row.operands = row.operands.iter().map(|n| rename(n, aliases)).collect();
            row.nested = row
                .nested
                .iter()
                .map(|arm| arm.iter().map(|i| renamed(i, aliases)).collect())
                .collect();
            row
        }
        fn scoped(
            source: &Source,
            row: &Instr,
            predicates: &[String],
            adjoint: bool,
            env: &mut Bindings,
            aliases: &BTreeMap<String, String>,
        ) -> Result<Vec<Json>, String> {
            let mut body = source.input_history(&[renamed(row, aliases)], adjoint, env)?;
            for predicate in predicates.iter().rev() {
                body = vec![invocation(
                    "qpred.scope",
                    vec![text(rename(predicate, aliases)), Json::Arr(body)],
                )];
            }
            Ok(body)
        }
        let mut result = vec![];
        for (ordinal, step) in steps.iter().enumerate() {
            let mut source = self.clone();
            let names = step
                .rollback
                .iter()
                .flat_map(|change| {
                    Self::references(&change.row).into_iter().chain(
                        change
                            .predicates
                            .iter()
                            .map(|n| n.split('[').next().unwrap().to_string()),
                    )
                })
                .chain(step.snapshot_controls.iter().cloned())
                .filter(|n| self.widths.contains_key(n) || self.regions.contains_key(n))
                .collect::<BTreeSet<_>>();
            let mut aliases = BTreeMap::new();
            let mut snapshots = vec![];
            for name in names {
                let alias = format!("$history.version.{ordinal}.{name}");
                let width = if let Some(region) = self.regions.get(&name) {
                    let mut region = region.clone();
                    region.local = true;
                    source.regions.insert(alias.clone(), region.clone());
                    self.region_width(&region)?
                } else {
                    self.widths[&name]
                };
                source.widths.insert(alias.clone(), width);
                let live = if step.live_versions.contains(&name) {
                    format!("$history.live.{name}")
                } else {
                    name.clone()
                };
                snapshots.push((live, alias.clone(), width));
                aliases.insert(name, alias);
            }
            let mut rollback = vec![];
            for change in &step.rollback {
                rollback.extend(scoped(
                    &source,
                    &change.row,
                    &change.predicates,
                    !adjoint,
                    env,
                    &aliases,
                )?);
            }
            let mut body = snapshots
                .iter()
                .map(|(name, alias, width)| {
                    invocation("qcopy.bound", vec![num(*width), text(name), text(alias)])
                })
                .collect::<Vec<_>>();
            body.extend(rollback.clone());
            let mut row_aliases = aliases.clone();
            for name in &step.live_versions {
                row_aliases.remove(name);
            }
            body.extend(scoped(
                &source,
                &step.row,
                &step.predicates,
                adjoint,
                env,
                &row_aliases,
            )?);
            if !rollback.is_empty() {
                body.push(inverse(rollback));
            }
            body.extend(snapshots.iter().rev().map(|(name, alias, width)| {
                invocation("quncopy.bound", vec![num(*width), text(name), text(alias)])
            }));
            for (_, alias, width) in snapshots.iter().rev() {
                body = vec![Json::obj(vec![
                    ("declare", text(alias)),
                    ("width", num(*width)),
                    ("class", Json::obj(vec![("class", text("any"))])),
                    ("state", text("clean")),
                    ("body", Json::Arr(body)),
                ])];
            }
            result.extend(body);
        }
        Ok(result)
    }
    // Recover exact pre-call target values from their source computation,
    // rather than treating allocation metadata as a zero-value assertion.
    // Keep execution order here, including source-adjoint acquisition.
    fn call_inputs(
        &self,
        instructions: &[Instr],
        compute: &Instr,
        cleanup: &Instr,
        targets: &[String],
        adjoint: bool,
        env: &Bindings,
    ) -> Option<CallInputs> {
        // Generated traversal projects enclosing fixed/while-loop and branch histories
        // into this domain first. Predicate slices still retain the enclosing
        // prefix, including acquisitions before the lexical scope.
        let history = if self.history.iter().any(|i| i.line == compute.line) {
            self.history.as_slice()
        } else {
            instructions
        };
        let mut prefix = history.iter().collect::<Vec<_>>();
        if adjoint {
            prefix.reverse();
        }
        prefix.truncate(prefix.iter().rposition(|i| i.line == cleanup.line)?);
        prefix.retain(|i| i.op != "qregion");
        prefix.splice(0..0, self.locals.iter());
        let compute_at = prefix.iter().rposition(|i| i.line == compute.line)?;
        let acquisition = |i: &Instr, target: &str| {
            if !adjoint {
                matches!(i.op.as_str(), "qalloc" | "qcopy" | "qload")
                    && i.lhs.first().is_some_and(|n| n == target)
            } else {
                match i.op.as_str() {
                    "qfree" => i.operands.first().is_some_and(|n| n == target),
                    "quncopy" => i.operands.get(1).is_some_and(|n| n == target),
                    "qunload" => i.operands.get(2).is_some_and(|n| n == target),
                    _ => false,
                }
            }
        };
        // Walk backwards from the exact versions the call consumes. Rebuild
        // source-owned read dependencies too: using their current wire names
        // can silently substitute a later version of an input to an earlier copy.
        let base = |n: &str| n.split('[').next().unwrap().to_string();
        let owned = self
            .declarations
            .iter()
            .filter(|(_, _, _, state)| state == "clean")
            .map(|(n, ..)| n.clone())
            .chain(self.locals.iter().map(|i| i.operands[0].clone()))
            .collect::<BTreeSet<_>>();
        let mut needed = targets.iter().map(|n| base(n)).collect::<BTreeSet<_>>();
        needed.extend(
            Self::references(compute)
                .into_iter()
                .filter(|n| owned.contains(n)),
        );
        let mut selected = BTreeSet::new();
        let mut steps = BTreeMap::new();
        let mut versioned = false;
        let mut acquired = BTreeSet::new();
        let mut external_changes: Vec<(&Instr, Vec<String>, usize)> = vec![];
        let mut predicates = vec![];
        for (index, i) in prefix.iter().enumerate().rev() {
            if matches!(i.op.as_str(), "qpred" | "qunpred") {
                if (i.op == "qunpred") != adjoint {
                    predicates.push(i.operands[0].clone());
                } else {
                    predicates.pop();
                }
                if index < compute_at {
                    needed.extend(
                        Self::references(i)
                            .into_iter()
                            .filter(|n| owned.contains(n)),
                    );
                    selected.insert(index);
                }
                continue;
            }
            let acquisitions = owned
                .iter()
                .filter(|n| {
                    acquisition(i, n) || (i.op == "qregion" && i.operands.first() == Some(n))
                })
                .cloned()
                .collect::<Vec<_>>();
            let outputs = if acquisitions.is_empty() {
                Self::outputs(i, env)
                    .into_iter()
                    .map(|n| base(&n))
                    .collect::<BTreeSet<_>>()
            } else {
                acquisitions.iter().cloned().collect()
            };
            if index >= compute_at || outputs.is_disjoint(&needed) {
                if !outputs.is_empty() && acquisitions.is_empty() {
                    if external_changes.last().is_some_and(|(next, scopes, at)| {
                        let mut reads = Self::references(i);
                        reads.extend(predicates.iter().map(|n| base(n)));
                        *scopes == predicates
                            && Self::inverse_rows(i, next)
                            && prefix[index + 1..*at].iter().all(|row| {
                                Self::outputs(row, env)
                                    .iter()
                                    .all(|n| !reads.contains(&base(n)))
                            })
                    }) {
                        external_changes.pop();
                    } else {
                        external_changes.push((i, predicates.clone(), index));
                    }
                }
                continue;
            }
            // Slice the inverse external history from the values this row
            // actually reads. Walking the inverse sequence backwards also
            // captures the versions of its controls and read dependencies.
            let mut version_inputs = Self::references(i);
            version_inputs.extend(predicates.iter().map(|n| base(n)));
            version_inputs.retain(|n| !owned.contains(n));
            let mut rollback = vec![];
            let mut live_versions = BTreeSet::new();
            for (change, scopes, at) in external_changes.iter().rev() {
                if Self::outputs(change, env)
                    .iter()
                    .all(|n| !version_inputs.contains(&base(n)))
                {
                    continue;
                }
                let mut refs = Self::references(change);
                refs.extend(scopes.iter().map(|n| base(n)));
                if !Self::permutation_row(change, env) {
                    return None;
                }
                fn lifetime_change(i: &Instr, name: &str) -> bool {
                    (matches!(i.op.as_str(), "qalloc" | "qborrow" | "qcopy" | "qload")
                        && i.lhs.first().is_some_and(|n| n == name))
                        || (matches!(i.op.as_str(), "qfree" | "qreturn" | "qregion")
                            && i.operands.first().is_some_and(|n| n == name))
                        || (i.op == "quncopy" && i.operands.get(1).is_some_and(|n| n == name))
                        || (i.op == "qunload" && i.operands.get(2).is_some_and(|n| n == name))
                        || i.nested.iter().flatten().any(|i| lifetime_change(i, name))
                }
                for name in refs.iter().filter(|n| owned.contains(*n)) {
                    // A live owned lifetime supplies a basis snapshot, outside
                    // the call's mutable targets. Later version changes enter
                    // this same inverse slice; a released/reacquired lifetime
                    // cannot supply the captured value. Keep this live binding
                    // distinct from the fresh reconstructed owned history.
                    if targets.iter().any(|n| base(n) == *name)
                        || self.forbidden_witnesses.contains(name)
                        || !env.values.contains_key(name)
                        || prefix[at + 1..]
                            .iter()
                            .any(|row| lifetime_change(row, name))
                    {
                        return None;
                    }
                    live_versions.insert(name.clone());
                }
                version_inputs.extend(refs);
                rollback.push(HistoryChange {
                    row: (*change).clone(),
                    predicates: scopes.clone(),
                });
            }
            rollback.reverse();
            fn selectors(i: &Instr, out: &mut BTreeSet<String>) {
                if i.op == "qif" {
                    out.insert(i.operands[0].split('[').next().unwrap().to_string());
                }
                if i.op == "qloop" && i.operands.len() != 1 {
                    out.insert(i.operands[2].clone());
                }
                for child in i.nested.iter().flatten() {
                    selectors(child, out);
                }
            }
            let mut snapshot_controls = BTreeSet::new();
            selectors(i, &mut snapshot_controls);
            snapshot_controls.retain(|n| !owned.contains(n));
            versioned |= !rollback.is_empty() || !snapshot_controls.is_empty();
            steps.insert(
                index,
                HistoryStep {
                    row: (*i).clone(),
                    predicates: predicates.clone(),
                    rollback,
                    snapshot_controls,
                    live_versions,
                },
            );
            // A selected row may change only reconstructed owned values.
            // Interface and borrowed values must retain their original bindings.
            if outputs.iter().any(|n| !owned.contains(n))
                || (acquisitions.is_empty() && !Self::permutation_row(i, env))
            {
                return None;
            }
            selected.insert(index);
            let is_acquisition = !acquisitions.is_empty();
            for n in acquisitions {
                needed.remove(&n);
                acquired.insert(n);
            }
            needed.extend(Self::references(i).into_iter().filter(|n| {
                owned.contains(n)
                    && !acquisition(i, n)
                    && !(i.op == "qregion" && i.operands.first() == Some(n))
            }));
            if !is_acquisition && !outputs.is_empty() {
                external_changes.push((i, predicates.clone(), index));
            }
        }
        if !needed.is_empty() || targets.iter().any(|n| !acquired.contains(&base(n))) {
            return None;
        }
        let mut rows = prefix
            .into_iter()
            .enumerate()
            .filter(|(n, _)| selected.contains(n))
            .map(|(_, i)| i.clone())
            .collect::<Vec<_>>();
        // The prefix can end inside a predicate which contains the call.
        // Close its captured input-history scope before replay; in a source
        // adjoint, execution-order qunpred opens the corresponding scope.
        let (open, close) = if adjoint {
            ("qunpred", "qpred")
        } else {
            ("qpred", "qunpred")
        };
        let mut scopes = vec![];
        for i in &rows {
            if i.op == open {
                scopes.push(i.clone());
            } else if i.op == close {
                scopes.pop()?;
            }
        }
        for mut scope in scopes.into_iter().rev() {
            scope.op = close.into();
            rows.push(scope);
        }
        Some(CallInputs {
            rows,
            versioned: versioned.then(|| steps.into_values().collect()),
        })
    }
    /// Different outputs of a terminal chain may enter at different source
    /// occurrences. Compute each captured input in an isolated workspace, copy
    /// it into its fresh target and uncompute that workspace. This preserves
    /// live secondary outputs and never uses a post-call target as a witness.
    fn resolve_joint_inputs(&self, pair: &Json, env: &Bindings) -> Result<Json, String> {
        let request = field(pair, "input_request")?;
        let targets = arr(field(request, "targets")?)?
            .iter()
            .map(|n| string(n).map(String::from))
            .collect::<Result<Vec<_>, _>>()?;
        let entries = arr(field(request, "entry_points")?)?;
        let mut source = self.clone();
        source.forbidden_witnesses.extend(targets.iter().cloned());
        let mut initialising = vec![];
        let mut allocations = vec![];
        let mut live = BTreeSet::new();
        let mut found = BTreeSet::new();
        let mut result = pair.clone();
        let Json::Obj(p) = &mut result else {
            return Err(err("invalid joint cleanup"));
        };
        for target in &targets {
            let (_, width, class, state) = self
                .declarations
                .iter()
                .find(|(n, ..)| n == target)
                .ok_or_else(|| err("joint cleanup target lacks a source declaration"))?;
            if state != "clean" {
                p.insert("recompute_supported".into(), Json::Bool(false));
                return Ok(result);
            }
            allocations.push(Json::obj(vec![
                ("name", text(target)),
                ("width", num(*width)),
                ("class", text(class)),
            ]));
            initialising.push(invocation("qalloc", vec![text(target)]));
        }
        for entry in entries {
            keys(entry, &["target", "compute"])?;
            let target = string(field(entry, "target")?)?;
            if !targets.iter().any(|n| n == target) || !found.insert(target.to_string()) {
                return Err(err("invalid joint input target partition"));
            }
            let mut nested = pair.clone();
            let Json::Obj(n) = &mut nested else {
                unreachable!()
            };
            n.insert(
                "input_request".into(),
                Json::obj(vec![
                    ("compute", field(entry, "compute")?.clone()),
                    ("cleanup", field(request, "cleanup")?.clone()),
                    ("adjoint", field(request, "adjoint")?.clone()),
                    ("targets", Json::Arr(vec![text(target)])),
                ]),
            );
            let input = source.resolve_call_inputs(&nested, env)?;
            if input.get("recompute_supported") == Some(&Json::Bool(false)) {
                p.insert("recompute_supported".into(), Json::Bool(false));
                return Ok(result);
            }
            if let Some(names) = input.get("live_versions") {
                for name in arr(names)? {
                    live.insert(string(name)?.to_string());
                }
            }
            let mut body = arr(field(&input, "initialising")?)?.clone();
            body.push(invocation(
                "bind.qxor",
                vec![
                    num(self.width(target)?),
                    text(target),
                    text("$history.target"),
                ],
            ));
            body.extend(arr(field(&input, "uninitialising")?)?.iter().cloned());
            let mut temporaries = BTreeSet::new();
            for allocation in arr(field(&input, "allocations")?)?.iter().rev() {
                if !temporaries.insert(string(field(allocation, "name")?)?.to_string()) {
                    continue;
                }
                body = vec![Json::obj(vec![
                    ("declare", field(allocation, "name")?.clone()),
                    ("width", field(allocation, "width")?.clone()),
                    (
                        "class",
                        Json::obj(vec![("class", field(allocation, "class")?.clone())]),
                    ),
                    ("state", text("clean")),
                    ("body", Json::Arr(body)),
                ])];
            }
            initialising.push(Json::obj(vec![
                (
                    "$history_target",
                    Json::obj(vec![
                        ("target", text(target)),
                        (
                            "temporaries",
                            Json::Arr(temporaries.iter().map(text).collect()),
                        ),
                    ]),
                ),
                ("body", Json::Arr(body)),
            ]));
        }
        if found.len() != targets.len() {
            return Err(err("missing joint input target"));
        }
        p.insert("recompute_supported".into(), Json::Bool(true));
        p.insert(
            "targets".into(),
            Json::Arr(targets.iter().map(text).collect()),
        );
        p.insert("allocations".into(), Json::Arr(allocations));
        p.insert(
            "live_versions".into(),
            Json::Arr(live.iter().map(text).collect()),
        );
        p.insert("initialising".into(), Json::Arr(initialising.clone()));
        p.insert(
            "uninitialising".into(),
            Json::Arr(vec![inverse(initialising)]),
        );
        Ok(result)
    }
    /// Resolve only when the carrier is generated: enclosing loop indices are
    /// not known while forming the source recipe (and a zero-trip body is inert).
    pub(super) fn resolve_call_inputs(&self, pair: &Json, env: &Bindings) -> Result<Json, String> {
        let request = field(pair, "input_request")?;
        if request.get("entry_points").is_some() {
            return self.resolve_joint_inputs(pair, env);
        }
        let compute_line = field(request, "compute")?
            .as_usize()
            .ok_or_else(|| err("invalid computing line"))?;
        let cleanup_line = field(request, "cleanup")?
            .as_usize()
            .ok_or_else(|| err("invalid cleanup line"))?;
        let adjoint = field(request, "adjoint")? == &Json::Bool(true);
        let targets = arr(field(request, "targets")?)?
            .iter()
            .map(|n| string(n).map(String::from))
            .collect::<Result<Vec<_>, _>>()?;
        fn find(rows: &[Instr], line: usize) -> Option<&Instr> {
            rows.iter().find_map(|i| {
                if i.line == line {
                    Some(i)
                } else {
                    i.nested.iter().find_map(|a| find(a, line))
                }
            })
        }
        let compute =
            find(&self.history, compute_line).ok_or_else(|| err("missing source compute"))?;
        let cleanup =
            find(&self.history, cleanup_line).ok_or_else(|| err("missing source cleanup"))?;
        fn branches(rows: &[Instr], line: usize, known: &mut BTreeMap<String, usize>) -> bool {
            for i in rows {
                if i.line == line {
                    return true;
                }
                for (arm, rows) in i.nested.iter().enumerate() {
                    if branches(rows, line, known) {
                        if i.op == "qif" {
                            known.insert(i.operands[0].clone(), arm);
                        }
                        return true;
                    }
                }
            }
            false
        }
        let mut known = BTreeMap::new();
        branches(&self.history, cleanup_line, &mut known);
        // Only read-only conditions prove the same arm in earlier iterations.
        // Mutable predicates replay their actual conditional on reconstructed
        // historical selector versions instead of specializing the current arm.
        known.retain(|name, _| self.readonly.contains(name));
        let domain = HistoryDomain {
            iterations: &env.source_iterations,
            branches: known,
        };
        let mut history = vec![];
        let formed = self
            .project_history(
                &self.history,
                Some(cleanup_line),
                adjoint,
                &domain,
                0,
                &mut history,
            )
            .is_some();
        if adjoint {
            history.reverse();
        }
        let mut source = self.clone();
        source.history = history;
        let inputs = formed
            .then(|| source.call_inputs(&[], compute, cleanup, &targets, adjoint, env))
            .flatten();
        let mut result = pair.clone();
        let Json::Obj(p) = &mut result else {
            return Err(err("invalid call cleanup"));
        };
        let mut local = env.clone();
        let env = &mut local;
        p.insert("recompute_supported".into(), Json::Bool(inputs.is_some()));
        if let Some(inputs) = inputs {
            let allocations = inputs
                .rows
                .iter()
                .filter_map(|i| {
                    let name = if i.op == "qregion" {
                        i.operands.first()
                    } else if !adjoint && matches!(i.op.as_str(), "qalloc" | "qcopy" | "qload") {
                        i.lhs.first()
                    } else if adjoint {
                        match i.op.as_str() {
                            "qfree" => i.operands.first(),
                            "quncopy" => i.operands.get(1),
                            "qunload" => i.operands.get(2),
                            _ => None,
                        }
                    } else {
                        None
                    }?;
                    let (width, class) = if let Some(r) = self.regions.get(name) {
                        (self.region_width(r).ok()?, "any".to_string())
                    } else {
                        let (_, w, c, state) =
                            self.declarations.iter().find(|(n, ..)| n == name)?;
                        if state != "clean" {
                            return None;
                        }
                        (*w, c.clone())
                    };
                    Some(Json::obj(vec![
                        ("name", text(name)),
                        ("width", num(width)),
                        ("class", text(class)),
                    ]))
                })
                .collect();
            p.insert("allocations".into(), Json::Arr(allocations));
            p.insert(
                "targets".into(),
                Json::Arr(targets.iter().map(text).collect()),
            );
            let initialising = if let Some(steps) = inputs.versioned {
                p.insert(
                    "live_versions".into(),
                    Json::Arr(
                        steps
                            .iter()
                            .flat_map(|s| s.live_versions.iter().cloned())
                            .collect::<BTreeSet<_>>()
                            .iter()
                            .map(text)
                            .collect(),
                    ),
                );
                self.versioned_input_history(&steps, adjoint, env)?
            } else {
                self.input_history(&inputs.rows, adjoint, env)?
            };
            p.insert("initialising".into(), Json::Arr(initialising.clone()));
            p.insert(
                "uninitialising".into(),
                Json::Arr(vec![inverse(initialising)]),
            );
        }
        Ok(result)
    }

    /// Project the executed prefix at one generated source coordinate. Completed
    /// loops replay their full predicated history; the containing loop contributes
    /// exactly its earlier iterations and the selected current prefix. A containing
    /// qif contributes only the active arm. Its else predicate is already inverted
    /// by ref.qif: retain that external version change for the dependency check.
    fn project_history(
        &self,
        rows: &[Instr],
        stop: Option<usize>,
        adjoint: bool,
        domain: &HistoryDomain<'_>,
        depth: usize,
        out: &mut Vec<Instr>,
    ) -> Option<()> {
        fn contains(i: &Instr, line: usize) -> bool {
            i.line == line || i.nested.iter().flatten().any(|i| contains(i, line))
        }
        let ordered = if adjoint {
            rows.iter().rev().collect::<Vec<_>>()
        } else {
            rows.iter().collect()
        };
        for i in ordered {
            if stop == Some(i.line) {
                out.push(i.clone());
                return Some(());
            }
            let containing = stop.is_some_and(|line| contains(i, line));
            if i.op == "qloop" {
                let count = i.operands[0].parse::<usize>().ok()?;
                let current = if containing {
                    let iteration = *domain.iterations.get(depth)?;
                    if iteration >= count {
                        return None;
                    }
                    Some(iteration)
                } else {
                    None
                };
                let iterations: Vec<_> = if adjoint {
                    (current.map_or(0, |i| i + 1)..count).rev().collect()
                } else {
                    (0..current.unwrap_or(count)).collect()
                };
                for iteration in iterations {
                    // Inside a forward while iteration its live flag proves
                    // every earlier iteration active. Completed loops (and
                    // later forward iterations in a source adjoint) retain
                    // the full conjunction of their immutable condition bits.
                    let conditional = i.operands.len() != 1 && (!containing || adjoint);
                    let (open, close) = if adjoint {
                        ("qunpred", "qpred")
                    } else {
                        ("qpred", "qunpred")
                    };
                    let predicate = |bit, op: &str| {
                        let mut p = i.clone();
                        p.op = op.into();
                        p.operands = vec![format!("{}[{bit}]", i.operands[2])];
                        p.nested.clear();
                        p
                    };
                    if conditional {
                        out.extend((0..=iteration).map(|bit| predicate(bit, open)));
                    }
                    self.project_history(&i.nested[0], None, adjoint, domain, depth + 1, out)?;
                    if conditional {
                        out.extend((0..=iteration).rev().map(|bit| predicate(bit, close)));
                    }
                }
                if containing {
                    return self.project_history(
                        &i.nested[0],
                        stop,
                        adjoint,
                        domain,
                        depth + 1,
                        out,
                    );
                }
            } else if containing && i.op == "qif" {
                let arm = i
                    .nested
                    .iter()
                    .position(|a| a.iter().any(|i| contains(i, stop.unwrap())))?;
                if arm == 1 {
                    let mut inversion = i.clone();
                    inversion.op = "x".into();
                    inversion.nested.clear();
                    out.push(inversion);
                }
                return self.project_history(&i.nested[arm], stop, adjoint, domain, depth, out);
            } else if i.op == "qif" && domain.branches.contains_key(&i.operands[0]) {
                let arm = domain.branches[&i.operands[0]];
                self.project_history(&i.nested[arm], None, adjoint, domain, depth, out)?;
            } else if containing {
                // Unknown containing structures cannot prove an exact prefix.
                return None;
            } else {
                out.push(i.clone());
            }
        }
        stop.is_none().then_some(())
    }

    fn whole_pairs(
        &self,
        instructions: &[Instr],
        adjoint: bool,
        env: &Bindings,
        occurrences: &partition::OccurrenceMatches,
    ) -> Result<BTreeMap<usize, WholePair>, String> {
        let mut pairs = BTreeMap::new();
        let order: Vec<_> = if adjoint {
            (0..instructions.len()).rev().collect()
        } else {
            (0..instructions.len()).collect()
        };
        let positions = order
            .iter()
            .enumerate()
            .map(|(p, n)| (*n, p))
            .collect::<BTreeMap<_, _>>();
        let mut claimed = BTreeSet::new();
        for release in order
            .iter()
            .copied()
            .filter(|n| occurrences.entries.contains_key(n))
        {
            let acquire = occurrences.entries[&release];
            let target = if adjoint {
                &instructions[release].lhs[0]
            } else {
                &instructions[release].operands[0]
            };
            let start = positions[&acquire];
            let end = positions[&release];
            let mut inverses = vec![];
            let mut owned_targets = BTreeSet::from([target.clone()]);
            loop {
                inverses.clear();
                let mut expanded = false;
                for position in (start + 1..end).rev() {
                    let index = order[position];
                    let row = &instructions[index];
                    if !owned_targets
                        .iter()
                        .any(|target| Self::mentions(row, target))
                    {
                        continue;
                    }
                    let Some(&partner) = occurrences.inverse.get(&index) else {
                        break;
                    };
                    if positions[&partner] >= position || claimed.contains(&index) {
                        break;
                    }
                    if !owned_targets
                        .iter()
                        .any(|target| Self::writes_only(row, target, env))
                    {
                        // Expand the connected output set, then rescan the
                        // suffix: an inverse skipped before this expansion can
                        // restore a newly included target nearer the release.
                        let names = Self::outputs(row, env)
                            .into_iter()
                            .map(|n| n.split('[').next().unwrap().to_string())
                            .collect::<BTreeSet<_>>();
                        if names.is_empty()
                            || names.is_disjoint(&owned_targets)
                            || !names
                                .iter()
                                .all(|name| self.declarations.iter().any(|(n, ..)| n == name))
                        {
                            break;
                        }
                        let before = owned_targets.len();
                        owned_targets.extend(names);
                        if owned_targets.len() != before {
                            expanded = true;
                            break;
                        }
                    }
                    inverses.push(index);
                }
                if !expanded {
                    break;
                }
            }
            let bundle = (owned_targets.len() > 1)
                .then(|| owned_targets.iter().cloned().collect::<Vec<_>>());
            inverses.reverse();
            let mut computing = if bundle.is_some() {
                vec![]
            } else {
                vec![acquire]
            };
            computing.extend(inverses.iter().rev().map(|n| occurrences.inverse[n]));
            let mut cleanup = inverses;
            cleanup.push(release);
            computing.sort_unstable();
            cleanup.sort_unstable();
            let cleanup_start = *cleanup.first().unwrap();
            let cleanup_end = *cleanup.last().unwrap();
            let mut touched = self.locations(target)?;
            if let Some(names) = &bundle {
                for name in names {
                    touched.extend(self.locations(name)?);
                }
            }
            for n in &computing {
                touched.extend(self.read_locations(&instructions[*n])?);
            }
            let mut safe = true;
            for (offset, row) in instructions[cleanup_start..=cleanup_end].iter().enumerate() {
                let n = cleanup_start + offset;
                if cleanup.contains(&n) {
                    continue;
                }
                // A disjoint structured row keeps its own source traversal,
                // predicate scopes and nested cleanup carriers. It is outside
                // this lifetime's inverse chain, just like a disjoint gate.
                if !(crate::action::is_gate_name(&row.op)
                    || matches!(row.op.as_str(), "qxor" | "qexch" | "qif" | "qloop"))
                    || !self.read_locations(row)?.is_disjoint(&touched)
                {
                    safe = false;
                    break;
                }
            }
            if !safe {
                continue;
            }
            let outside = (cleanup_start..=cleanup_end)
                .filter(|n| !cleanup.contains(n))
                .collect();
            let perm = bundle.as_ref().is_none_or(|names| {
                names.iter().all(|name| {
                    self.declarations
                        .iter()
                        .any(|(n, _, _, state)| n == name && state == "clean")
                })
            }) && self
                .declarations
                .iter()
                .find(|(n, ..)| n == target)
                .is_some_and(|(_, _, _, state)| state == "clean")
                && computing
                    .iter()
                    .filter(|n| **n != acquire)
                    .all(|n| Self::permutation_row(&instructions[*n], env));
            claimed.extend(cleanup.iter().copied().filter(|n| *n != release));
            pairs.insert(
                cleanup_start,
                WholePair {
                    cleanup_end,
                    carrier: release,
                    computing,
                    entry_point: acquire,
                    bundle,
                    outside,
                    target: target.clone(),
                    perm,
                },
            );
        }
        for (end, release) in instructions.iter().enumerate() {
            // Intrinsic endings retain their direct computing operation only.
            let target = match release.op.as_str() {
                "qfree" | "qreturn" => &release.operands[0],
                _ => continue,
            };
            let Some(start) = instructions[..end].iter().rposition(|i| {
                matches!(i.op.as_str(), "qalloc" | "qborrow" | "qcopy" | "qload")
                    && i.lhs.first() == Some(target)
            }) else {
                continue;
            };
            let allocation = &instructions[start];
            if allocation.op != "qcopy" {
                continue;
            }
            if (release.op == "qreturn") != (allocation.op == "qborrow")
                || allocation.op == "qload"
                || instructions[start..=end]
                    .iter()
                    .any(|i| matches!(i.op.as_str(), "qpred" | "qunpred"))
            {
                continue;
            }
            let rows = (start..=end)
                .filter(|n| Self::mentions(&instructions[*n], target))
                .collect::<Vec<_>>();
            let mut left = 1;
            let mut right = rows.len() - 1;
            if allocation.op == "qcopy" {
                if right <= left
                    || instructions[rows[right - 1]].op != "qxor"
                    || instructions[rows[right - 1]].operands
                        != [allocation.operands[0].clone(), target.clone()]
                {
                    continue;
                }
                right -= 1;
            }
            let inverse_end = right;
            while left + 1 < right
                && Self::writes_only(&instructions[rows[left]], target, env)
                && occurrences.inverse.get(&rows[left]) == Some(&rows[right - 1])
            {
                left += 1;
                right -= 1;
            }
            if left == 1 && allocation.op != "qcopy" && left < right {
                continue;
            }
            let k = inverse_end - right;
            let computing = if adjoint {
                rows[right..].to_vec()
            } else {
                rows[..=k].to_vec()
            };
            let cleanup = if adjoint {
                rows[..=k].to_vec()
            } else {
                rows[right..].to_vec()
            };
            let cleanup_start = *cleanup.first().unwrap();
            let cleanup_end = *cleanup.last().unwrap();
            let mut touched = self.locations(target)?;
            for n in &computing {
                touched.extend(self.read_locations(&instructions[*n])?);
            }
            // These interleaved rows commute with every owned computing and
            // inverse row. They retain source ownership and are emitted before
            // the fresh carrier replacement. More general cross-lifetime and
            // joint-output partitioning is handled separately.
            let mut safe = true;
            for n in (start..=end).filter(|n| !rows.contains(n)) {
                let row = &instructions[n];
                if !(crate::action::is_gate_name(&row.op)
                    || matches!(row.op.as_str(), "qxor" | "qexch"))
                    || !self.read_locations(row)?.is_disjoint(&touched)
                {
                    safe = false;
                    break;
                }
            }
            if !safe {
                continue;
            }
            let outside = (cleanup_start..=cleanup_end)
                .filter(|n| !cleanup.contains(n))
                .collect();
            let perm = allocation.op != "qborrow"
                && rows[1..=k]
                    .iter()
                    .all(|n| Self::permutation_row(&instructions[*n], env));
            pairs.insert(
                cleanup_start,
                WholePair {
                    cleanup_end,
                    carrier: if adjoint { start } else { end },
                    entry_point: if adjoint {
                        *computing.last().unwrap()
                    } else {
                        *computing.first().unwrap()
                    },
                    computing,
                    bundle: None,
                    outside,
                    target: target.clone(),
                    perm,
                },
            );
        }
        Ok(pairs)
    }
    fn raw_sequence(
        &self,
        instructions: &[Instr],
        adjoint: bool,
        source: bool,
        env: &mut Bindings,
    ) -> Result<Vec<Json>, String> {
        let mut body = vec![];
        let mut index = 0;
        while index < instructions.len() {
            let i = &instructions[index];
            let row = if i.op == "qpred" {
                let mut depth = 1;
                let mut end = index + 1;
                while end < instructions.len() {
                    match instructions[end].op.as_str() {
                        "qpred" => depth += 1,
                        "qunpred" => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    end += 1;
                }
                if end == instructions.len() || instructions[end].operands != i.operands {
                    return Err(err("unmatched input-history predicate scope"));
                }
                let nested =
                    self.raw_sequence(&instructions[index + 1..end], adjoint, source, env)?;
                index = end;
                vec![invocation(
                    "qpred.scope",
                    vec![text(&i.operands[0]), Json::Arr(nested)],
                )]
            } else if self.history_replay && i.op == "qif" {
                // Reconstruct the written history, not cleanup policy inside
                // that history: replaying a prior pair is not a new carrier.
                let body = vec![invocation(
                    "qif",
                    vec![
                        text(&i.operands[0]),
                        Json::Arr(self.raw_sequence(&i.nested[0], false, false, env)?),
                        Json::Arr(self.raw_sequence(&i.nested[1], false, false, env)?),
                    ],
                )];
                if adjoint { vec![inverse(body)] } else { body }
            } else if self.history_replay && i.op == "qloop" {
                let count = num(i.operands[0].parse::<usize>().map_err(err)?);
                if i.operands.len() == 1 {
                    vec![invocation(
                        "qloop",
                        vec![
                            count,
                            Json::Arr(self.raw_sequence(&i.nested[0], adjoint, false, env)?),
                        ],
                    )]
                } else {
                    let body = vec![invocation(
                        "qloop.while",
                        vec![
                            count,
                            text(&i.operands[2]),
                            Json::Arr(self.raw_sequence(&i.nested[0], false, false, env)?),
                        ],
                    )];
                    if adjoint { vec![inverse(body)] } else { body }
                }
            } else {
                self.instruction(i, adjoint, env)?
            };
            if source {
                body.push(self.source_annotation(i, adjoint, row));
            } else {
                body.extend(row);
            }
            index += 1;
        }
        if adjoint {
            body.reverse();
        }
        Ok(body)
    }
    fn source_annotation(&self, i: &Instr, adjoint: bool, body: Vec<Json>) -> Json {
        Json::obj(vec![
            (
                "$source",
                Json::obj(vec![
                    ("symbol", text(&self.symbol)),
                    ("point", text(&self.points[&i.line])),
                    (
                        "reverse_iterations",
                        Json::Bool(adjoint && i.op == "qloop" && i.operands.len() == 1),
                    ),
                ]),
            ),
            ("body", Json::Arr(body)),
        ])
    }
    fn sequence(
        &self,
        instructions: &[Instr],
        adjoint: bool,
        env: &mut Bindings,
    ) -> Result<Vec<Json>, String> {
        let mut out = vec![];
        let occurrences = self.inverse_occurrences(instructions, env, adjoint)?;
        let whole_pairs = self.whole_pairs(instructions, adjoint, env, &occurrences)?;
        let mut empty_pairs = BTreeMap::new();
        for pair in instructions.windows(2) {
            if matches!(pair[0].op.as_str(), "qalloc" | "qborrow")
                && matches!(pair[1].op.as_str(), "qfree" | "qreturn")
                && pair[0].lhs.first() == pair[1].operands.first()
            {
                empty_pairs.insert(pair[0].line, pair[1].clone());
                empty_pairs.insert(pair[1].line, pair[0].clone());
            }
        }
        let mut call_pairs = BTreeMap::new();
        for (index, i) in instructions.iter().enumerate() {
            let Some(&prior) = occurrences.inverse.get(&index) else {
                continue;
            };
            if prior >= index || !matches!(i.op.as_str(), "action_call" | "adj" | "ctl") {
                continue;
            }
            let (inverse, _control, op, args) = crate::action::normalize_call(i)?;
            if !inverse || op != "action_call" {
                continue;
            }
            let compute = &instructions[prior];
            let registry = env
                .registry
                .as_ref()
                .ok_or_else(|| err("source registry missing"))?;
            let f = registry
                .object
                .function(args[0].trim_start_matches('@'))
                .ok_or_else(|| err("missing paired callee"))?;
            let mut targets = crate::validate::list_operand(&args[1])?;
            targets.extend(
                crate::validate::list_operand(&args[3])?
                    .into_iter()
                    .zip(&f.lists[2])
                    .filter(|(_, p)| p.role.as_deref() == Some("mut"))
                    .map(|(n, _)| n),
            );
            let perm = f.kind == FnKind::PermAction;
            call_pairs.insert(compute.line, (i.clone(), targets.clone(), perm));
            call_pairs.insert(i.line, (compute.clone(), targets, perm));
        }
        let mut index = 0;
        while index < instructions.len() {
            let i = &instructions[index];
            if let Some(pair) = whole_pairs.get(&index) {
                let computing_rows = pair
                    .computing
                    .iter()
                    .map(|n| instructions[*n].clone())
                    .collect::<Vec<_>>();
                let computing = self.raw_sequence(&computing_rows, adjoint, false, env)?;
                let uncomputing = self.raw_sequence(&computing_rows, !adjoint, false, env)?;
                let body =
                    self.raw_sequence(&instructions[index..=pair.cleanup_end], adjoint, true, env)?;
                let mut annotation = self.pair_annotation(
                    if pair.bundle.is_some() {
                        ""
                    } else {
                        &pair.target
                    },
                    &self.points[&instructions[pair.entry_point].line],
                    computing,
                    uncomputing,
                    body,
                    pair.perm,
                );
                if let Some(targets) = &pair.bundle {
                    let compute = if adjoint {
                        *pair.computing.last().unwrap()
                    } else {
                        *pair.computing.first().unwrap()
                    };
                    let Json::Obj(c) = &mut annotation else {
                        unreachable!()
                    };
                    let Some(Json::Obj(p)) = c.get_mut("$cleanup") else {
                        unreachable!()
                    };
                    let mut ordered = pair.computing.clone();
                    if adjoint {
                        ordered.reverse();
                    }
                    let mut entry_points = vec![];
                    for target in targets {
                        let first = ordered
                            .iter()
                            .find(|n| {
                                Self::outputs(&instructions[**n], env)
                                    .iter()
                                    .any(|name| name.split('[').next().unwrap() == target)
                            })
                            .ok_or_else(|| err("joint target has no computing occurrence"))?;
                        entry_points.push(Json::obj(vec![
                            ("target", text(target)),
                            ("compute", num(instructions[*first].line)),
                        ]));
                    }
                    p.insert(
                        "input_request".into(),
                        Json::obj(vec![
                            ("compute", num(instructions[compute].line)),
                            ("cleanup", num(instructions[pair.carrier].line)),
                            ("adjoint", Json::Bool(adjoint)),
                            ("targets", Json::Arr(targets.iter().map(text).collect())),
                            ("entry_points", Json::Arr(entry_points)),
                        ]),
                    );
                    p.insert("zero_targets".into(), Json::Arr(vec![text(&pair.target)]));
                    p.insert(
                        "release_targets".into(),
                        Json::Arr(vec![text(&pair.target)]),
                    );
                }
                if !pair.outside.is_empty() {
                    let rows = pair
                        .outside
                        .iter()
                        .map(|n| instructions[*n].clone())
                        .collect::<Vec<_>>();
                    let outside = self.raw_sequence(&rows, adjoint, true, env)?;
                    let Json::Obj(c) = &mut annotation else {
                        unreachable!()
                    };
                    let Some(Json::Obj(p)) = c.get_mut("$cleanup") else {
                        unreachable!()
                    };
                    p.insert("before_recompute".into(), Json::Arr(outside));
                }
                out.push(self.source_annotation(
                    &instructions[pair.carrier],
                    adjoint,
                    vec![annotation],
                ));
                index = pair.cleanup_end + 1;
            } else if i.op == "qpred" {
                let mut depth = 1;
                let mut end = index + 1;
                while end < instructions.len() {
                    match instructions[end].op.as_str() {
                        "qpred" => depth += 1,
                        "qunpred" => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    end += 1;
                }
                if end == instructions.len() || instructions[end].operands != i.operands {
                    return Err(err("unmatched source predicate scope"));
                }
                let body = self.sequence(&instructions[index + 1..end], adjoint, env)?;
                out.push(invocation(
                    "qpred.scope",
                    vec![text(&i.operands[0]), Json::Arr(body)],
                ));
                index = end + 1;
            } else {
                let body = self.instruction(i, adjoint, env)?;
                let body = if let Some(partner) = empty_pairs.get(&i.line) {
                    if matches!(
                        (i.op.as_str(), adjoint),
                        ("qfree" | "qreturn", false) | ("qalloc" | "qborrow", true)
                    ) {
                        let compute = if adjoint { i } else { partner };
                        let target = &compute.lhs[0];
                        let computing = self.instruction(compute, false, env)?;
                        let uncomputing = self.instruction(compute, true, env)?;
                        vec![self.pair_annotation(
                            target,
                            &self.points[&partner.line],
                            computing,
                            uncomputing,
                            body,
                            compute.op == "qalloc",
                        )]
                    } else {
                        body
                    }
                } else if let Some((partner, targets, perm)) = call_pairs.get(&i.line) {
                    let (call_inverse, control, _, _) = crate::action::normalize_call(i)?;
                    if call_inverse ^ adjoint {
                        let computing = self.instruction(partner, adjoint, env)?;
                        let uncomputing = self.instruction(partner, !adjoint, env)?;
                        {
                            let mut pair = self.pair_annotation(
                                "",
                                &self.points[&partner.line],
                                computing,
                                uncomputing,
                                body,
                                *perm,
                            );
                            if let Json::Obj(c) = &mut pair
                                && let Some(Json::Obj(p)) = c.get_mut("$cleanup")
                            {
                                p.insert(
                                    "input_request".into(),
                                    Json::obj(vec![
                                        ("compute", num(partner.line)),
                                        ("cleanup", num(i.line)),
                                        ("adjoint", Json::Bool(adjoint)),
                                        ("targets", Json::Arr(targets.iter().map(text).collect())),
                                    ]),
                                );
                            }
                            if let Some(control) = control {
                                vec![invocation(
                                    "ctl",
                                    vec![text(control), Json::Arr(vec![pair])],
                                )]
                            } else {
                                vec![pair]
                            }
                        }
                    } else {
                        body
                    }
                } else {
                    self.cleanup_pair(i, adjoint, body, env)?
                };
                out.push(self.source_annotation(i, adjoint, body));
                index += 1;
            }
        }
        if adjoint {
            out.reverse();
        }
        Ok(out)
    }
    fn pair_annotation(
        &self,
        target: &str,
        point: &str,
        computing: Vec<Json>,
        uncomputing: Vec<Json>,
        body: Vec<Json>,
        perm: bool,
    ) -> Json {
        Json::obj(vec![
            (
                "$cleanup",
                Json::obj(vec![
                    ("target", text(target)),
                    ("point", text(point)),
                    ("computing", Json::Arr(computing)),
                    ("uncomputing", Json::Arr(uncomputing)),
                    ("perm", Json::Bool(perm)),
                ]),
            ),
            ("body", Json::Arr(body)),
        ])
    }
    fn defer_cleanup(&self, body: Vec<Json>, _env: &Bindings) -> Result<Vec<Json>, String> {
        // Preserve generation staging: an unresolved pair inside a zero-trip
        // source loop is not an operation and cannot reject any policy.
        Ok(vec![Json::obj(vec![
            ("$cleanup_pending", Json::Bool(true)),
            ("body", Json::Arr(body)),
        ])])
    }
    fn cleanup_pair(
        &self,
        i: &Instr,
        adjoint: bool,
        body: Vec<Json>,
        env: &mut Bindings,
    ) -> Result<Vec<Json>, String> {
        let target = match (i.op.as_str(), adjoint) {
            ("quncopy", false) => Some(&i.operands[1]),
            ("qunload", false) => Some(&i.operands[2]),
            ("qcopy" | "qload", true) => Some(&i.lhs[0]),
            _ => None,
        };
        let Some(target) = target else {
            let release = matches!(
                (i.op.as_str(), adjoint),
                ("qfree" | "qreturn", false) | ("qalloc" | "qborrow", true)
            );
            let call = matches!(i.op.as_str(), "action_call" | "adj" | "ctl")
                && (crate::action::normalize_call(i)?.0 ^ adjoint);
            return if release || call {
                self.defer_cleanup(body, env)
            } else {
                Ok(body)
            };
        };
        // A loop-carried slot can have different producing source rows on
        // different visits. Keep its default trace until visit-local pairing
        // has established which occurrence owns this cleanup.
        let Some([compute]) = self.computations.get(target).map(Vec::as_slice) else {
            return self.defer_cleanup(body, env);
        };
        if !adjoint {
            let expected = if compute.op == "qcopy" {
                vec![compute.operands[0].clone(), target.clone()]
            } else {
                vec![
                    compute.operands[0].clone(),
                    compute.operands[1].clone(),
                    target.clone(),
                ]
            };
            if i.operands != expected {
                return self.defer_cleanup(body, env);
            }
        }
        let line = if adjoint {
            let Some([line]) = self.cleanup_sites.get(target).map(Vec::as_slice) else {
                return self.defer_cleanup(body, env);
            };
            *line
        } else {
            compute.line
        };
        let computing = self.instruction(compute, false, env)?;
        let uncomputing = self.instruction(compute, true, env)?;
        Ok(vec![self.pair_annotation(
            target,
            &self.points[&line],
            computing,
            uncomputing,
            body,
            true,
        )])
    }
    fn instruction(
        &self,
        i: &Instr,
        adjoint: bool,
        env: &mut Bindings,
    ) -> Result<Vec<Json>, String> {
        let op = i.op.as_str();
        let operands = &i.operands;
        if crate::action::is_gate_name(op) {
            let op = if adjoint {
                match op {
                    "s" => "sdg",
                    "sdg" => "s",
                    "t" => "tdg",
                    "tdg" => "t",
                    _ => op,
                }
            } else {
                op
            };
            let args = if ["rx", "ry", "rz"].contains(&op) {
                let bits = match env.values.get(&operands[0]) {
                    Some(Bound::Value(value, ty)) if ty.as_str() == Some("f64") => {
                        angle(value, env)?
                    }
                    Some(_) => return Err(err("gate angle argument must have type f64")),
                    None => operands[0].parse::<f64>().map_err(err)?.to_bits(),
                };
                let angle = text(format!(
                    "0x{:016x}",
                    bits ^ if adjoint { 1 << 63 } else { 0 }
                ));
                vec![angle, text(&operands[1]), Json::Arr(vec![])]
            } else if op == "mcx" {
                let controls = crate::validate::list_operand(&operands[0])?;
                vec![
                    num(controls.len()),
                    Json::Arr(controls.iter().map(text).collect()),
                    text(&operands[1]),
                    Json::Arr(vec![]),
                ]
            } else {
                operands
                    .iter()
                    .map(text)
                    .chain(std::iter::once(Json::Arr(vec![])))
                    .collect()
            };
            let kind = if ["x", "cx", "ccx", "swap"].contains(&op) {
                format!("bind.gate.{op}")
            } else {
                format!("gate.{op}")
            };
            return Ok(vec![invocation(&kind, args)]);
        }
        if matches!(op, "oracle_call" | "adj" | "ctl") {
            let (_, control, inner, operands) = crate::action::normalize_call(i)?;
            if inner == "oracle_call" {
                let mut call = i.clone();
                call.op = inner.into();
                call.operands = operands;
                let registry = env
                    .registry
                    .clone()
                    .ok_or_else(|| err("source closure registry missing"))?;
                let body = vec![super::oracle_source::call(&registry.object, &call, env)?];
                return Ok(if let Some(control) = control {
                    vec![invocation("ctl", vec![text(control), Json::Arr(body)])]
                } else {
                    body
                });
            }
        }
        let (kind, args) = match op {
            "action_call" | "adj" | "ctl" => {
                let mut offset = 0;
                let control = if op == "ctl" {
                    offset = 1;
                    Some(&operands[0])
                } else {
                    None
                };
                let inner = if op == "action_call" {
                    "action_call"
                } else {
                    operands
                        .get(offset)
                        .map(String::as_str)
                        .ok_or_else(|| err("call opcode missing"))?
                };
                let call_adjoint = op == "adj" || inner == "adj";
                if inner == "adj" {
                    offset += 1;
                }
                if op != "action_call" {
                    if operands.get(offset).map(String::as_str) != Some("action_call") {
                        return Err(err(
                            "unavailable.reason: source oracle call construction is unavailable",
                        ));
                    }
                    offset += 1;
                }
                let call = &operands[offset..];
                let name = call[0].trim_start_matches('@');
                let registry = env
                    .registry
                    .clone()
                    .ok_or_else(|| err("source closure registry missing"))?;
                let callee = registry
                    .object
                    .function(name)
                    .ok_or_else(|| err("unresolved source action callee"))?;
                if !matches!(callee.kind, FnKind::Action | FnKind::PermAction) {
                    return Err(err("action call requires action callee"));
                }
                let hash = registry
                    .identities
                    .closure
                    .get(name)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| err("callee closure unresolved"))?
                    .clone();
                let alias = format!("$symbol.{}.{}", i.line, adjoint);
                env.values.insert(
                    alias.clone(),
                    Bound::Symbol {
                        hash,
                        registry: registry.clone(),
                        adjoint: adjoint ^ call_adjoint,
                    },
                );
                let mut actuals = vec![];
                for group in [1, 2] {
                    actuals.extend(
                        crate::validate::list_operand(&call[group])?
                            .iter()
                            .map(text),
                    );
                }
                for name in crate::validate::list_operand(&call[3])? {
                    let r = self
                        .regions
                        .get(&name)
                        .ok_or_else(|| err("unbound source region actual"))?;
                    actuals.push(if !r.local {
                        text(&name)
                    } else {
                        Json::obj(vec![
                            ("address", num(r.address)),
                            ("cell", num(r.cell)),
                            ("role", text(&r.role)),
                            (
                                "origin",
                                text(format!("local:{}", name.trim_start_matches('$'))),
                            ),
                            ("cells", text(name)),
                        ])
                    });
                }
                let args = crate::validate::list_operand(&call[4])?;
                if args.len() != callee.lists[3].len() {
                    return Err(err("source call argument arity"));
                }
                for (a, p) in args.iter().zip(&callee.lists[3]) {
                    if matches!(env.values.get(a), Some(Bound::Value(..))) {
                        actuals.push(text(a));
                        continue;
                    }
                    let value = match p.ty {
                        Type::I(width) => {
                            let n = if let Some(magnitude) = a.strip_prefix('-') {
                                let n = crate::bigint::BigUint::parse_decimal(magnitude)?;
                                if n.bits() > width {
                                    return Err(err("source integer argument out of range"));
                                }
                                crate::bigint::BigUint::from_u64(1)
                                    .shl(width)
                                    .sub(&n)
                                    .mask(width)
                            } else {
                                crate::bigint::BigUint::parse_int(a)?
                            };
                            text(n.to_decimal())
                        }
                        Type::Ptr => text(crate::bigint::BigUint::parse_int(a)?.to_decimal()),
                        Type::F64 => text(format!(
                            "0x{:016x}",
                            a.parse::<f64>().map_err(err)?.to_bits()
                        )),
                        _ => return Err(err("unbound source token argument")),
                    };
                    actuals.push(value);
                }
                let body = vec![invocation(
                    "action_call",
                    vec![text(alias), Json::Arr(actuals)],
                )];
                return Ok(if let Some(control) = control {
                    vec![invocation("ctl", vec![text(control), Json::Arr(body)])]
                } else {
                    body
                });
            }
            "qregion" => return Ok(vec![]),
            "qunpred" => return Err(err("unmatched source qunpred")),
            "qalloc" | "qborrow" => (
                match (op, adjoint) {
                    ("qalloc", false) => "qalloc",
                    ("qalloc", true) => "qfree",
                    ("qborrow", false) => "qborrow",
                    _ => "qreturn",
                },
                vec![text(&i.lhs[0])],
            ),
            "qfree" | "qreturn" => (
                if !adjoint {
                    op
                } else if op == "qfree" {
                    "qalloc"
                } else {
                    "qborrow"
                },
                vec![text(&operands[0])],
            ),
            "qcopy" => (
                if adjoint {
                    "quncopy.bound"
                } else {
                    "qcopy.bound"
                },
                vec![
                    num(self.width(&operands[0])?),
                    text(&operands[0]),
                    text(&i.lhs[0]),
                ],
            ),
            "quncopy" => (
                if adjoint {
                    "qcopy.bound"
                } else {
                    "quncopy.bound"
                },
                vec![
                    num(self.width(&operands[0])?),
                    text(&operands[0]),
                    text(&operands[1]),
                ],
            ),
            "qxor" | "qexch" => (
                if op == "qxor" {
                    "bind.qxor"
                } else {
                    "bind.qexch"
                },
                vec![
                    num(self.width(&operands[0])?),
                    text(&operands[0]),
                    text(&operands[1]),
                ],
            ),
            "qload" | "qunload" | "qstore" | "qexchm" => {
                let name = &operands[0];
                let r = self
                    .regions
                    .get(name)
                    .ok_or_else(|| err("unbound source region"))?;
                let width = self.region_width(r)?;
                let region = Json::obj(vec![
                    ("address", num(r.address)),
                    ("cell", num(r.cell)),
                    ("role", text(&r.role)),
                    (
                        "origin",
                        text(format!(
                            "{}:{}",
                            if r.local { "local" } else { "interface" },
                            name.trim_start_matches('$')
                        )),
                    ),
                    ("cells", text(name)),
                ]);
                let load = op == "qload" || op == "qunload";
                let value = if op == "qload" {
                    &i.lhs[0]
                } else {
                    &operands[2]
                };
                let kind = if load {
                    format!(
                        "{}.{}",
                        if (op == "qload") != adjoint {
                            "qload"
                        } else {
                            "qunload"
                        },
                        r.role
                    )
                } else {
                    op.into()
                };
                return Ok(self.allocation_metadata(
                    i,
                    vec![invocation(
                        &kind,
                        vec![
                            num(r.address),
                            num(width / r.cell),
                            num(r.cell),
                            if r.address == 0 {
                                Json::Arr(vec![])
                            } else {
                                text(&operands[1])
                            },
                            if load && r.local { region } else { text(name) },
                            text(value),
                        ],
                    )],
                ));
            }
            "qif" => {
                let body = vec![invocation(
                    "qif",
                    vec![
                        text(&operands[0]),
                        Json::Arr(self.sequence(&i.nested[0], false, env)?),
                        Json::Arr(self.sequence(&i.nested[1], false, env)?),
                    ],
                )];
                return Ok(if adjoint { vec![inverse(body)] } else { body });
            }
            "qloop" => {
                let count = operands[0].parse::<usize>().map_err(err)?;
                if operands.len() != 1 {
                    let body = vec![invocation(
                        "qloop.while",
                        vec![
                            num(count),
                            text(&operands[2]),
                            Json::Arr(self.sequence(&i.nested[0], false, env)?),
                        ],
                    )];
                    return Ok(if adjoint { vec![inverse(body)] } else { body });
                }
                (
                    "qloop",
                    vec![
                        num(count),
                        Json::Arr(self.sequence(&i.nested[0], adjoint, env)?),
                    ],
                )
            }
            _ => {
                return Err(err(format!(
                    "unavailable.reason: source {op} symbol/row formation is unavailable"
                )));
            }
        };
        Ok(self.allocation_metadata(i, vec![invocation(kind, args)]))
    }
}
