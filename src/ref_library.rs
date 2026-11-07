//! Independent consumer of the published §11.1 catalog. Source linking is a
//! separate prerequisite; successful catalog authentication is not a schedule verdict.
pub(crate) mod action_support;
mod address;
mod calls;
mod named_interface;
pub use calls::{BoundCalls, CallDomain, CallTarget};
#[cfg(test)]
mod call_domains_tests;
mod cleanup;
#[cfg(test)]
mod named_interface_tests;
#[cfg(test)]
#[path = "ref_library/oracle_bindings_tests.rs"]
mod oracle_bindings_tests;
mod oracle_cleanup;
mod oracle_paths;
mod oracle_source;
pub use cleanup::{ActionExpansion, CleanupCarrier, CleanupEntry, CleanupRealisation};
pub use oracle_cleanup::OracleCleanupPair;
mod source;
pub(crate) use source::token_cleanup_points;
use crate::{
    json::{self, Json},
    target::{Rational as R, arr, field, keys, string},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    path::Path,
    rc::Rc,
};
pub const HASH: &str = "5de4e4995350313e50217880ba1b95de39cd043283b726337d4894efb005d197";
fn err(s: impl std::fmt::Display) -> String {
    format!("§11.1: E_SCHEDULE_INFEASIBLE: {s}")
}
#[derive(Clone, Debug)]
enum Bound {
    Integer(R),
    Wire(Vec<String>),
    Angle(u64),
    Body(Json, Box<Bindings>),
    Templates(Json, Box<Bindings>),
    Allocation(Rc<RefCell<Allocation>>),
    Image(String),
    Event(Json),
    Class(String),
    Symbol {
        hash: String,
        registry: Rc<Registry>,
        adjoint: bool,
    },
    Arguments {
        hash: String,
        values: Vec<Bound>,
        oracle_invocation: Option<Json>,
        oracle_identity: Option<u64>,
    },
    Region {
        address: usize,
        cell: usize,
        role: String,
        origin: String,
        cells: Vec<String>,
    },
    Value(Json, Json, Option<u64>),
    Records {
        fields: Json,
        order: Json,
        rows: Vec<BTreeMap<String, Bound>>,
    },
}
#[derive(Clone, Debug)]
struct Registry {
    object: crate::object::Object,
    identities: crate::identity::Identities,
}
impl Registry {
    fn name(&self, hash: &str) -> Result<&str, String> {
        self.identities
            .closure
            .iter()
            .find(|(_, h)| h.as_deref() == Some(hash))
            .map(|(n, _)| n.as_str())
            .ok_or_else(|| err(format!("unbound closure {hash}")))
    }
}
#[derive(Clone, Debug)]
struct Allocation {
    identity: String,
    width: usize,
    wires: Vec<String>,
    class: String,
    state: String,
}
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    values: BTreeMap<String, Bound>,
    controls: Vec<String>,
    locals: BTreeSet<String>,
    declarations: Rc<Cell<usize>>,
    scratch: BTreeSet<String>,
    registry: Option<Rc<Registry>>,
    call_index: Rc<Cell<usize>>,
    call_domains: Rc<RefCell<calls::CallCollector>>,
    active_symbols: Rc<RefCell<Vec<String>>>,
    source_point: Option<crate::sched::Point>,
    source_next: Rc<Cell<usize>>,
    source_iterations: Vec<usize>,
    reverse_iterations: bool,
    source_formed: bool,
    source_history: Option<Rc<source::Source>>,
    replaying_cleanup: bool,
    oracle_address: Option<Rc<address::OracleAddress>>,
    oracle_invocation: Option<Json>,
    oracle_identity: Option<u64>,
    row_spaces: Vec<address::RowSpace>,
    coordinate: crate::sched::Coordinate,
    cleanup: cleanup::CleanupContext,
}
impl Bindings {
    /// Reparse the canonical source before constructing a linked registry;
    /// caller-supplied Rust AST fields cannot retarget a closure hash.
    pub fn link(&mut self, object: &crate::object::Object) -> Result<(), String> {
        let (object, identities) = crate::load(&object.text, true)?;
        self.registry = Some(Rc::new(Registry { object, identities }));
        Ok(())
    }
    fn address(&self, kind: crate::sched::OperationKind) -> Option<crate::sched::Operation> {
        self.source_point.as_ref().map(|p| {
            let ordinal = self.source_next.get();
            self.source_next.set(ordinal + 1);
            let mut p = p.clone();
            p.iteration = self.source_iterations.clone();
            crate::sched::Operation {
                path: vec![p],
                generated: ordinal,
                kind,
                step: self.coordinate.step,
                phase: self.coordinate.phase.clone(),
            }
        })
    }
    fn add_controls(&mut self, wires: impl IntoIterator<Item = String>) {
        for wire in wires {
            if !self.controls.contains(&wire) {
                self.controls.push(wire);
            }
        }
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn wire(&mut self, name: &str, width: usize) {
        self.values.insert(
            name.into(),
            Bound::Wire((0..width).map(|i| format!("{name}[{i}]")).collect()),
        );
    }
    pub fn body(&mut self, name: &str, body: Json) -> Result<(), String> {
        syntax(&body)?;
        self.values
            .insert(name.into(), Bound::Body(body, Box::new(self.clone())));
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Leaf {
    /// Flat raw-recipe index retained for the library expansion API. Source
    /// schedule ordinals are per source point in address.generated.
    pub ordinal: usize,
    pub address: Option<crate::sched::Operation>,
    pub operation: String,
    pub wires: Vec<String>,
    pub controls: Vec<String>,
    pub angle: Option<u64>,
    pub class: Option<String>,
    pub state: Option<String>,
    pub event: Option<Json>,
    pub allocation: Option<String>,
    pub scratch: bool,
    // The original source acquisition fixes its lifetime's metadata in either
    // traversal. A loop-carried slot can be acquired with a new class.
    pub(crate) source_allocation: bool,
    pub invocation: Vec<SemanticCall>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticCall {
    pub closure_hash: String,
    pub call_index: usize,
    pub adjoint: bool,
}
impl Leaf {
    fn adjoint(&mut self) {
        self.operation = match self.operation.as_str() {
            "s" => "sdg",
            "sdg" => "s",
            "t" => "tdg",
            "tdg" => "t",
            "alloc" => "release",
            "release" => "alloc",
            s => s,
        }
        .into();
        if let Some(address) = &mut self.address {
            address.kind = match address.kind {
                crate::sched::OperationKind::Alloc => crate::sched::OperationKind::Release,
                crate::sched::OperationKind::Release => crate::sched::OperationKind::Alloc,
                kind => kind,
            };
        }
        if let Some(a) = &mut self.angle {
            *a ^= 1 << 63;
        }
    }
}
pub struct Catalog {
    doc: Json,
    interfaces: BTreeMap<String, Vec<String>>,
}
impl Catalog {
    pub fn published() -> Result<Self, String> {
        let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bennett-isa-spec/spec");
        let path = spec.join("ref-0.9.json");
        Self::parse(&std::fs::read_to_string(path).map_err(err)?)
    }
    pub fn parse(raw: &str) -> Result<Self, String> {
        let doc = json::parse(raw)?;
        keys(&doc, &["isa", "name", "version", "recipes"])?;
        for (k, v) in [("isa", "0.9"), ("name", "ref"), ("version", "0.9")] {
            if field(&doc, k)?.as_str() != Some(v) {
                return Err(err(format!("invalid {k}")));
            }
        }
        if raw != doc.to_jcs() {
            return Err(err("catalog must be JCS without trailing LF"));
        }
        let envelope = format!("isa=0.9\nexpansion=ref\nversion=0.9\n{raw}\n");
        if crate::sha256::sha256_hex(envelope.as_bytes()) != HASH {
            return Err(err("catalog envelope hash mismatch"));
        }
        let rs = arr(field(&doc, "recipes")?)?;
        if rs.len() != 219 {
            return Err(err("incomplete catalog"));
        }
        let mut last = "";
        for r in rs {
            let kind = string(field(r, "kind")?)?;
            if kind <= last {
                return Err(err("recipe order/duplicate kind"));
            }
            last = kind;
            validate_formals(arr(field(r, "parameters")?)?)?;
            if !arr(field(r, "removable")?)?.is_empty() {
                return Err(err("ref@0.9 removable must be empty"));
            }
            syntax(field(r, "body")?)?;
        }
        let interfaces = named_interface::collect(&doc)?;
        Ok(Self { doc, interfaces })
    }
    pub fn document(&self) -> &Json {
        &self.doc
    }
    pub fn hash(&self) -> &str {
        HASH
    }
    pub fn len(&self) -> usize {
        arr(self.doc.get("recipes").unwrap()).unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn recipe(&self, kind: &str) -> Result<&Json, String> {
        arr(field(&self.doc, "recipes")?)?
            .iter()
            .find(|r| r.get("kind").and_then(Json::as_str) == Some(kind))
            .ok_or_else(|| err(format!("unknown recipe {kind}")))
    }
    pub fn expand(
        &self,
        kind: &str,
        args: &[Json],
        bindings: &Bindings,
    ) -> Result<Vec<Leaf>, String> {
        self.expand_with_calls(kind, args, bindings).map(|e| e.0)
    }
    fn invoke(
        &self,
        kind: &str,
        args: &[Json],
        outer: &Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<(), String> {
        if active.iter().any(|k| k == kind) {
            return Err(err("cyclic invoke"));
        }
        let r = self.recipe(kind)?;
        let formals = arr(field(r, "parameters")?)?;
        if formals.len() != args.len() {
            return Err(err(format!("{kind}: arity mismatch")));
        }
        let capture = named_interface::scope(outer, self.interfaces.get(kind).map(Vec::as_slice))?;
        let mut env = bind_formals(formals, args, outer, &capture)?;
        env.scratch = arr(field(r, "scratch")?)?
            .iter()
            .map(|n| string(n).map(String::from))
            .collect::<Result<_, _>>()?;
        active.push(kind.into());
        let result = if env.oracle_address.is_some() && kind == "oracle.construct" {
            self.walk_oracle(field(r, "body")?, &env, out, next, active)
        } else if env.oracle_address.is_some() && kind == "step" {
            self.walk_step(field(r, "body")?, &env, out, next, active)
        } else {
            self.walk(field(r, "body")?, &mut env, out, next, active)
        };
        active.pop();
        result
    }
    fn walk(
        &self,
        body: &Json,
        env: &mut Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<(), String> {
        for c in arr(body)? {
            if !(self.walk_metadata(c, env, out, next, active)?
                || self.walk_leaves(c, env, out, next)?
                || self.walk_combinators(c, env, out, next, active)?
                || self.walk_calls(c, env, out, next, active)?)
            {
                return Err(err(
                    "unavailable.reason: unsupported catalog expansion descriptor",
                ));
            }
        }
        Ok(())
    }

    fn walk_metadata(
        &self,
        c: &Json,
        env: &mut Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<bool, String> {
        if let Some(p) = c.get("$source") {
            if env.replaying_cleanup {
                self.walk(field(c, "body")?, env, out, next, active)?;
                return Ok(true);
            }
            let mut local = env.clone();
            local.source_point = Some(crate::sched::Point {
                symbol: string(field(p, "symbol")?)?.into(),
                point: string(field(p, "point")?)?.into(),
                ..Default::default()
            });
            local.reverse_iterations = field(p, "reverse_iterations")? == &Json::Bool(true);
            local.call_index = Rc::new(Cell::new(0));
            local.source_next = Rc::new(Cell::new(0));
            local.call_domain();
            self.walk(field(c, "body")?, &mut local, out, next, active)?;
        } else if let Some(metadata) = c.get("$source_allocation") {
            let name = string(field(metadata, "name")?)?;
            let Some(Bound::Allocation(allocation)) = env.values.get(name) else {
                return Err(err("unbound source allocation slot"));
            };
            let mut allocation = allocation.borrow().clone();
            allocation.class = string(field(metadata, "class")?)?.into();
            allocation.state = string(field(metadata, "state")?)?.into();
            let identity = allocation.identity.clone();
            let mut local = env.clone();
            local.values.insert(
                name.into(),
                Bound::Allocation(Rc::new(RefCell::new(allocation))),
            );
            let start = out.len();
            self.walk(field(c, "body")?, &mut local, out, next, active)?;
            for leaf in &mut out[start..] {
                if leaf.allocation.as_ref() == Some(&identity) {
                    leaf.source_allocation = true;
                }
            }
        } else if let Some(storage) = c.get("$oracle_storage") {
            let mut local = env.clone();
            for entry in arr(storage)? {
                let name = string(field(entry, "name")?)?;
                let width = field(entry, "width")?
                    .as_usize()
                    .ok_or_else(|| err("invalid source storage width"))?;
                let id = env.declarations.get();
                env.declarations.set(id + 1);
                local.values.insert(
                    name.into(),
                    Bound::Allocation(Rc::new(RefCell::new(Allocation {
                        identity: format!("$decl{id}.{name}"),
                        width,
                        wires: (0..width)
                            .map(|i| format!("$decl{id}.{name}[{i}]"))
                            .collect(),
                        class: "any".into(),
                        state: "clean".into(),
                    }))),
                );
            }
            self.walk(field(c, "body")?, &mut local, out, next, active)?;
        } else if let Some(token) = c.get("$oracle_invocation") {
            let Some(Bound::Value(value, ty, identity)) = env.values.get(string(token)?) else {
                return Err(err("unbound oracle_invocation source token"));
            };
            if ty.as_str() != Some("token") {
                return Err(err("ill-typed oracle_invocation source token"));
            }
            let mut local = env.clone();
            local.oracle_invocation = Some(Json::obj(vec![("token", value.clone())]));
            local.oracle_identity = *identity;
            self.walk(field(c, "body")?, &mut local, out, next, active)?;
        } else if let Some(target) = c.get("$history_target") {
            let name = string(field(target, "target")?)?;
            let Some(value @ Bound::Allocation(_)) = env.values.get(name) else {
                return Err(err("history copy target is not an allocation"));
            };
            let mut local = env.clone();
            // Reserved, lexical binding: nested history workspaces may
            // shadow it without changing the enclosing target capability.
            local.values.insert("$history.target".into(), value.clone());
            for name in arr(field(target, "temporaries")?)? {
                let name = string(name)?;
                local.values.remove(name);
                local.locals.remove(name);
            }
            self.walk(field(c, "body")?, &mut local, out, next, active)?;
        } else if c.get("$cleanup_pending").is_some() {
            self.walk_pending_cleanup(c, env, out, next, active)?;
        } else if c.get("$cleanup").is_some() {
            self.walk_cleanup(c, env, out, next, active)?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    fn walk_leaves(
        &self,
        c: &Json,
        env: &mut Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
    ) -> Result<bool, String> {
        if let Some(g) = c.get("gate") {
            let op = string(g)?;
            if !crate::action::is_gate_name(op) {
                return Err(err("unknown gate"));
            }
            let mut ws = vec![];
            for w in arr(field(c, "wires")?)? {
                ws.extend(wires(w, env)?);
            }
            let controls = crate::target::own_controls(op, ws.len())?;
            if ws.iter().collect::<BTreeSet<_>>().len() != ws.len() {
                return Err(err("coincident gate operands"));
            }
            if ws.iter().skip(controls).any(|w| env.controls.contains(w)) {
                return Err(err("coincident target/control"));
            }
            let a = if ["rx", "ry", "rz"].contains(&op) {
                let p = arr(field(c, "params")?)?;
                if p.len() != 1 {
                    return Err(err("rotation parameter arity"));
                }
                Some(angle(&p[0], env)?)
            } else {
                if c.get("params").is_some() {
                    return Err(err("unexpected gate params"));
                }
                None
            };
            out.push(Leaf {
                ordinal: *next,
                address: env.address(crate::sched::OperationKind::Gate),
                operation: op.into(),
                wires: ws,
                controls: env.controls.clone(),
                angle: a,
                class: None,
                state: None,
                invocation: vec![],
                event: None,
                allocation: None,
                scratch: false,
                source_allocation: false,
            });
            *next += 1;
        } else if let Some(n) = c.get("alloc") {
            let name = string(n)?;
            let width = scalar(field(c, "width")?, env)?.usize()?;
            let state = string(field(c, "state")?)?;
            if state != "clean" && state != "borrowed" {
                return Err(err("invalid allocation state"));
            }
            let cls = string(field(c, "class")?)?;
            if cls.is_empty() {
                return Err(err("empty allocation class"));
            }
            // A repeated alloc in a raw inverse refers to the declaration at
            // that restored position. Validate actual lifetimes only after
            // inverse traversal, never by executing its forward prefix.
            let (identity, ws) = if env.locals.contains(name) {
                match env.values.get(name) {
                    Some(Bound::Allocation(a)) => {
                        let a = a.borrow();
                        if a.width != width || a.state != state || a.class != cls {
                            return Err(err("allocation metadata changed"));
                        }
                        (a.identity.clone(), a.wires.clone())
                    }
                    _ => return Err(err("allocation shadows binding")),
                }
            } else {
                let id = env.declarations.get();
                env.declarations.set(id + 1);
                let identity = format!("$decl{id}.{name}");
                let ws = (0..width)
                    .map(|i| format!("{identity}[{i}]"))
                    .collect::<Vec<_>>();
                (identity, ws)
            };
            env.locals.insert(name.into());
            env.values.insert(
                name.into(),
                Bound::Allocation(Rc::new(RefCell::new(Allocation {
                    identity: identity.clone(),
                    width,
                    wires: ws.clone(),
                    class: cls.into(),
                    state: state.into(),
                }))),
            );
            out.push(Leaf {
                ordinal: *next,
                address: env.address(crate::sched::OperationKind::Alloc),
                operation: "alloc".into(),
                wires: ws,
                controls: vec![],
                angle: None,
                class: Some(cls.into()),
                state: Some(state.into()),
                invocation: vec![],
                event: None,
                allocation: Some(identity),
                scratch: env.scratch.contains(name),
                source_allocation: false,
            });
            *next += 1;
        } else if let Some(n) = c.get("release") {
            let Some(Bound::Allocation(a)) = env.values.get(string(n)?) else {
                return Err(err("unbound release"));
            };
            let a = a.borrow();
            out.push(Leaf {
                ordinal: *next,
                address: env.address(crate::sched::OperationKind::Release),
                operation: "release".into(),
                wires: a.wires.clone(),
                controls: vec![],
                angle: None,
                class: Some(a.class.clone()),
                state: Some(a.state.clone()),
                invocation: vec![],
                event: None,
                allocation: Some(a.identity.clone()),
                scratch: false,
                source_allocation: false,
            });
            *next += 1;
        } else if let Some(n) = c.get("image") {
            let image = image(n, env)?;
            let ws = wires(field(c, "wires")?, env)?;
            if !image.is_empty() && ws.len() != image.len() * 4 {
                return Err(err("image width mismatch"));
            }
            for (byte, pair) in image.as_bytes().as_chunks::<2>().0.iter().enumerate() {
                let value =
                    u8::from_str_radix(std::str::from_utf8(pair).map_err(err)?, 16).map_err(err)?;
                for bit in 0..8 {
                    if value & (1 << bit) != 0 {
                        out.push(Leaf {
                            ordinal: *next,
                            address: env.address(crate::sched::OperationKind::Gate),
                            operation: "x".into(),
                            wires: vec![ws[byte * 8 + bit].clone()],
                            controls: vec![],
                            angle: None,
                            class: None,
                            state: None,
                            invocation: vec![],
                            event: None,
                            allocation: None,
                            scratch: false,
                            source_allocation: false,
                        });
                        *next += 1;
                    }
                }
            }
        } else if let Some(op) = c.get("classical") {
            let opcode = string(op)?;
            let point = string(field(c, "point")?)?;
            let (operation, event) = if let Some(Bound::Event(event)) = env.values.get(point) {
                let instruction = string(field(event, "instruction")?)?;
                let tokens = crate::object::split_tokens(instruction)?;
                let offset = tokens.iter().position(|t| t == "=").map_or(0, |i| i + 1);
                let actual = tokens
                    .get(offset)
                    .ok_or_else(|| err("empty classical event"))?;
                if actual != opcode
                    && !actual.strip_prefix(opcode).is_some_and(|s| {
                        s.strip_prefix(".i")
                            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                    })
                {
                    return Err(err("classical event opcode mismatch"));
                }
                (actual.clone(), event.clone())
            } else {
                (
                    opcode.into(),
                    Json::obj(vec![
                        ("instruction", Json::Str(opcode.into())),
                        ("point", Json::Str(point.into())),
                    ]),
                )
            };
            out.push(Leaf {
                ordinal: *next,
                address: env.address(crate::sched::OperationKind::Classical),
                operation,
                wires: vec![],
                controls: vec![],
                angle: None,
                class: None,
                state: None,
                invocation: vec![],
                event: Some(event),
                allocation: None,
                scratch: false,
                source_allocation: false,
            });
            *next += 1;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    fn walk_combinators(
        &self,
        c: &Json,
        env: &mut Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<bool, String> {
        if let Some(n) = c.get("for") {
            let name = string(n)?;
            let from = scalar(field(c, "from")?, env)?.usize()?;
            let to = scalar(field(c, "to")?, env)?.usize()?;
            for i in from..to {
                let mut local = env.clone();
                if env.source_point.is_some()
                    && !env.replaying_cleanup
                    && ((active.last().map(String::as_str) == Some("qloop") && name == "iteration")
                        || (active.last().map(String::as_str) == Some("qloop.while")
                            && name == "i"))
                {
                    local.source_iterations.push(
                        if env.reverse_iterations
                            && active.last().map(String::as_str) == Some("qloop")
                        {
                            to - 1 - i
                        } else {
                            i
                        },
                    );
                }
                local
                    .values
                    .insert(name.into(), Bound::Integer(R::from(i as u64)));
                self.walk(field(c, "body")?, &mut local, out, next, active)?;
            }
        } else if let Some(n) = c.get("each") {
            for w in wires(field(c, "in")?, env)? {
                let mut local = env.clone();
                local.values.insert(string(n)?.into(), Bound::Wire(vec![w]));
                self.walk(field(c, "body")?, &mut local, out, next, active)?;
            }
        } else if let Some(n) = c.get("invoke") {
            self.invoke(string(n)?, arr(field(c, "args")?)?, env, out, next, active)?;
        } else if c.get("inverse").is_some() {
            let start = out.len();
            let first_leaf = env.source_next.get();
            let first_call = env.call_index.get();
            let first_carrier = env.cleanup.carriers.borrow().len();
            let first_domain = env.call_domains.borrow().records.len();
            self.walk(field(c, "body")?, env, out, next, active)?;
            if out[start..].iter().any(|l| l.event.is_some()) {
                return Err(err("classical inverse requires retained §7.4 history"));
            }
            out[start..].reverse();
            for l in &mut out[start..] {
                l.adjoint();
            }
            if env.replaying_cleanup {
                // K80 instantiates a fresh caller space in emitted order.
                // This does not change raw inverse traversal or the
                // independently formed source spaces inside callees.
                let end_leaf = env.source_next.get();
                let end_call = env.call_index.get();
                let mut domains = env.call_domains.borrow_mut();
                if let Some(index) = domains.index(&env.call_index) {
                    let targets = &mut domains.records[index].domain.targets[first_call..end_call];
                    for target in targets.iter_mut() {
                        target.adjoint = !target.adjoint;
                    }
                    targets.reverse();
                }
                for domain in &mut domains.records[first_domain..] {
                    if let Some(index) = domain
                        .domain
                        .path
                        .get_mut(env.cleanup.prefix.len())
                        .and_then(|p| p.call_index.as_mut())
                    {
                        *index = first_call + (end_call - 1 - *index);
                    }
                }
                drop(domains);
                for l in &mut out[start..] {
                    if let Some(a) = &mut l.address {
                        if a.path.len() == 1 {
                            a.generated = first_leaf + (end_leaf - 1 - a.generated);
                        } else if let Some(index) = &mut a.path[0].call_index {
                            *index = first_call + (end_call - 1 - *index);
                        }
                    }
                    if let Some(call) = l.invocation.first_mut() {
                        call.call_index = first_call + (end_call - 1 - call.call_index);
                        call.adjoint = !call.adjoint;
                    }
                }
                for c in &mut env.cleanup.carriers.borrow_mut()[first_carrier..] {
                    if let Some(index) = c
                        .address
                        .path
                        .get_mut(env.cleanup.prefix.len())
                        .and_then(|p| p.call_index.as_mut())
                    {
                        *index = first_call + (end_call - 1 - *index);
                    }
                }
            }
        } else if let Some(n) = c.get("splice") {
            let Some(Bound::Body(b, cap)) = env.values.get(string(n)?) else {
                return Err(err("unbound body"));
            };
            let b = b.clone();
            let mut cap = *cap.clone();
            cap.add_controls(env.controls.clone());
            cap.inherit_address(env);
            cap.active_symbols = env.active_symbols.clone();
            // Captured actual syntax is finite; repeating a higher-order
            // recipe here is not a cycle in declared recipe dependencies.
            self.walk(&b, &mut cap, out, next, &mut vec![])?;
        } else if let Some(u) = c.get("under") {
            let mut local = env.clone();
            local.add_controls(wires(u, env)?);
            self.walk(field(c, "body")?, &mut local, out, next, active)?;
        } else if let Some(n) = c.get("declare") {
            let name = string(n)?;
            if env.values.contains_key(name) {
                return Err(err("declaration shadows binding"));
            }
            let width = scalar(field(c, "width")?, env)?.usize()?;
            let class = class(field(c, "class")?, env)?;
            if class.is_empty() {
                return Err(err("empty class"));
            }
            let state = string(field(c, "state")?)?.to_string();
            if !["clean", "borrowed"].contains(&state.as_str()) {
                return Err(err("invalid allocation state"));
            }
            let id = env.declarations.get();
            env.declarations.set(id + 1);
            let a = Rc::new(RefCell::new(Allocation {
                identity: format!("$decl{id}.{name}"),
                width,
                wires: (0..width)
                    .map(|i| format!("$decl{id}.{name}[{i}]"))
                    .collect(),
                class,
                state,
            }));
            let mut local = env.clone();
            local
                .values
                .insert(name.into(), Bound::Allocation(a.clone()));
            self.walk(field(c, "body")?, &mut local, out, next, active)?;
        } else if let Some(n) = c.get("acquire").or_else(|| c.get("relinquish")) {
            let Some(Bound::Allocation(a)) = env.values.get(string(n)?) else {
                return Err(err("unbound allocation"));
            };
            let a = a.borrow();
            let acquire = c.get("acquire").is_some();
            out.push(Leaf {
                ordinal: *next,
                address: env.address(if acquire {
                    crate::sched::OperationKind::Alloc
                } else {
                    crate::sched::OperationKind::Release
                }),
                operation: if acquire { "alloc" } else { "release" }.into(),
                wires: a.wires.clone(),
                controls: vec![],
                angle: None,
                class: Some(a.class.clone()),
                state: Some(a.state.clone()),
                invocation: vec![],
                event: None,
                allocation: Some(a.identity.clone()),
                scratch: acquire && env.scratch.contains(string(n)?),
                source_allocation: false,
            });
            *next += 1;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    fn walk_calls(
        &self,
        c: &Json,
        env: &mut Bindings,
        out: &mut Vec<Leaf>,
        next: &mut usize,
        active: &mut Vec<String>,
    ) -> Result<bool, String> {
        if let Some(n) = c.get("call") {
            let Some(Bound::Symbol {
                hash,
                registry,
                adjoint,
            }) = env.values.get(string(n)?)
            else {
                return Err(err("unbound call symbol"));
            };
            let hash = hash.clone();
            let registry = registry.clone();
            let adjoint = *adjoint;
            let name = registry.name(&hash)?;
            let formals = source::interface(&registry.object, name)?;
            let args = field(c, "args")?;
            let mut callee = if let Some(Bound::Arguments {
                hash: bound,
                values,
                oracle_invocation,
                oracle_identity,
            }) = args.as_str().and_then(|n| env.values.get(n))
            {
                if bound != &hash {
                    return Err(err("arguments closure mismatch"));
                }
                let mut bindings = Bindings::new();
                bindings.values = formals
                    .iter()
                    .zip(values)
                    .map(|(f, v)| Ok((string(field(f, "name")?)?.into(), v.clone())))
                    .collect::<Result<_, String>>()?;
                bindings.oracle_invocation = oracle_invocation.clone();
                bindings.oracle_identity = *oracle_identity;
                bindings
            } else {
                let mut bindings = bind_formals(&formals, arr(args)?, env, &Bindings::new())?;
                bindings.oracle_invocation = env.oracle_invocation.clone();
                bindings.oracle_identity = env.oracle_identity;
                bindings
            };
            callee.registry = Some(registry.clone());
            callee.coordinate = env.coordinate.clone();
            callee.cleanup = env.cleanup.clone();
            callee.call_domains = env.call_domains.clone();
            if let Some(caller) = &env.source_point {
                let mut edge = caller.clone();
                edge.iteration = env.source_iterations.clone();
                edge.call_index = Some(env.call_index.get());
                callee.cleanup.prefix.push(edge);
            }
            callee.declarations = env.declarations.clone();
            callee.active_symbols = env.active_symbols.clone();
            callee.add_controls(env.controls.clone());
            if callee.active_symbols.borrow().contains(&hash) {
                return Err(err("cyclic symbol expansion"));
            }
            let call_index = env.call_index.get();
            env.call_target(CallTarget {
                interface: registry.identities.interface[name].clone(),
                closure: hash.clone(),
                adjoint,
            });
            env.call_index.set(call_index + 1);
            callee.active_symbols.borrow_mut().push(hash.clone());
            let start = out.len();
            let result = (|| {
                let body = source::action_body(&registry.object, name, adjoint, &mut callee)?;
                self.walk(&body, &mut callee, out, &mut 0, &mut vec![])
            })();
            callee.active_symbols.borrow_mut().pop();
            result?;
            for leaf in &mut out[start..] {
                if let (Some(caller), Some(address)) =
                    (env.source_point.as_ref(), leaf.address.as_mut())
                {
                    let mut edge = caller.clone();
                    edge.iteration = env.source_iterations.clone();
                    edge.call_index = Some(call_index);
                    address.path.insert(0, edge);
                }
                leaf.invocation.insert(
                    0,
                    SemanticCall {
                        closure_hash: hash.clone(),
                        call_index,
                        adjoint,
                    },
                );
            }
        } else if let Some(n) = c.get("visit") {
            let Some(Bound::Records { rows, .. }) = env.values.get(string(n)?) else {
                return Err(err("unbound records"));
            };
            for row in rows.clone() {
                let mut local = env.clone();
                local.values.extend(row);
                if n.as_str() == Some("Frames")
                    && active.last().map(String::as_str) == Some("oracle.construct")
                    && env.oracle_address.is_some()
                {
                    local.coordinate.step =
                        Some(scalar(&Json::Str("index".into()), &local)?.usize()? + 1);
                }
                self.walk(field(c, "body")?, &mut local, out, next, active)?;
            }
        } else if let Some(n) = c.get("apply") {
            let Some(Bound::Templates(t, cap)) = env.values.get(string(n)?) else {
                return Err(err("unbound templates"));
            };
            let t = arr(t)?
                .get(scalar(field(c, "index")?, env)?.usize()?)
                .ok_or_else(|| err("template index out of range"))?
                .clone();
            let fs = arr(field(&t, "parameters")?)?;
            let args = arr(field(c, "args")?)?;
            let mut local = bind_formals(fs, args, env, cap)?;
            local.add_controls(env.controls.clone());
            local.inherit_address(env);
            if env.oracle_address.is_some()
                && active.last().map(String::as_str) == Some("step")
                && matches!(n.as_str(), Some("Rows" | "Next"))
            {
                self.address_row(&mut local, scalar(field(c, "index")?, env)?.usize()?)?;
            }
            local.active_symbols = env.active_symbols.clone();
            self.walk(field(&t, "body")?, &mut local, out, next, &mut vec![])?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }
}
/// Ownership before traversal direction is applied. Kind and the semantic
/// adjoint bit describe execution direction, so neither is an identity field.
#[derive(Clone, Copy)]
struct LifetimeOrigin<'a>(&'a Leaf);
impl PartialEq for LifetimeOrigin<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for LifetimeOrigin<'_> {}
impl PartialOrd for LifetimeOrigin<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for LifetimeOrigin<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        type PointKey<'a> = (
            &'a str,
            &'a str,
            &'a [usize],
            &'a [usize],
            Option<usize>,
            Option<(Option<usize>, &'a str)>,
        );
        fn point(p: &crate::sched::Point) -> PointKey<'_> {
            (
                &p.symbol,
                &p.point,
                &p.iteration,
                &p.instance,
                p.call_index,
                p.oracle.as_ref().map(|o| (o.step, o.phase.as_str())),
            )
        }
        let (a, b) = (self.0, other.0);
        let address = match (&a.address, &b.address) {
            (Some(a), Some(b)) => a
                .path
                .iter()
                .map(point)
                .cmp(b.path.iter().map(point))
                .then_with(|| {
                    (a.step, &a.phase, a.generated).cmp(&(b.step, &b.phase, b.generated))
                }),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        };
        address
            .then_with(|| {
                a.invocation
                    .iter()
                    .map(|c| (&c.closure_hash, c.call_index))
                    .cmp(b.invocation.iter().map(|c| (&c.closure_hash, c.call_index)))
            })
            .then_with(|| a.ordinal.cmp(&b.ordinal))
    }
}

fn resolve_lifetimes(leaves: &mut [Leaf]) -> Result<(), String> {
    // Raw inverse puts the acquire-side scratch mark on the closing leaf.
    // Pair by declaration identity before assigning physical occurrences.
    let mut open = BTreeMap::new();
    let mut marked = BTreeSet::new();
    let mut origins = BTreeMap::new();
    let mut metadata = vec![];
    for (i, l) in leaves.iter().enumerate() {
        if l.operation == "alloc" {
            let id = l
                .allocation
                .as_deref()
                .ok_or_else(|| err("allocation identity missing"))?;
            if open.insert(id, i).is_some() {
                return Err(err("double acquisition"));
            }
        } else if l.operation == "release" {
            let id = l
                .allocation
                .as_deref()
                .ok_or_else(|| err("release identity missing"))?;
            let start = open.remove(id).ok_or_else(|| err("unmatched release"))?;
            let acquisition = &leaves[start];
            let original = if acquisition.source_allocation {
                acquisition
            } else {
                l
            };
            if original.source_allocation {
                if acquisition.wires.len() != l.wires.len()
                    || (acquisition.source_allocation
                        && l.source_allocation
                        && (acquisition.class != l.class || acquisition.state != l.state))
                {
                    return Err(err("conflicting source lifetime metadata"));
                }
                metadata.push((start, i, original.class.clone(), original.state.clone()));
            }
            if l.scratch || leaves[start].scratch {
                marked.insert(start);
            }
            // Either endpoint may become the acquire under raw inverse. Their
            // unordered ownership pair determines the same forward lifetime.
            let origin = LifetimeOrigin(&leaves[start]).min(LifetimeOrigin(l));
            if origins.insert((id, origin), start).is_some() {
                return Err(err("duplicate allocation occurrence origin"));
            }
        }
    }
    for (id, start) in open {
        if origins
            .insert((id, LifetimeOrigin(&leaves[start])), start)
            .is_some()
        {
            return Err(err("duplicate allocation occurrence origin"));
        }
    }
    // Compact names, sorted by immutable ownership rather than the emitted
    // traversal order. Keys borrow the trace so large circuits need no copied
    // source paths or hashes per allocation.
    let actuals = origins
        .values()
        .enumerate()
        .map(|(rank, &start)| (start, format!("$alloc{rank}")))
        .collect::<BTreeMap<_, _>>();
    drop(origins);
    for (start, end, class, state) in metadata {
        for index in [start, end] {
            leaves[index].class = class.clone();
            leaves[index].state = state.clone();
        }
    }

    let mut wires = BTreeMap::<String, String>::new();
    let mut live = BTreeMap::<String, (String, Option<String>, Option<String>, bool, usize)>::new();
    let mut scratch = vec![];
    for (index, leaf) in leaves.iter_mut().enumerate() {
        if leaf.operation == "alloc" {
            leaf.scratch = marked.contains(&index);
            let id = leaf
                .allocation
                .clone()
                .ok_or_else(|| err("allocation identity missing"))?;
            if live.contains_key(&id) {
                return Err(err("double acquisition"));
            }
            if leaf.scratch && leaf.state.as_deref() != Some("clean") {
                return Err(err("library scratch must be clean"));
            }
            let actual = actuals[&index].clone();
            live.insert(
                id.clone(),
                (
                    actual.clone(),
                    leaf.class.clone(),
                    leaf.state.clone(),
                    leaf.scratch,
                    leaf.wires.len(),
                ),
            );
            if leaf.scratch {
                scratch.push(id);
            }
            for (bit, wire) in leaf.wires.iter_mut().enumerate() {
                let physical = format!("{actual}[{bit}]");
                if wires.insert(wire.clone(), physical.clone()).is_some() {
                    return Err(err("overlapping allocation"));
                }
                *wire = physical;
            }
            leaf.allocation = Some(actual);
        } else if leaf.operation == "release" {
            let id = leaf
                .allocation
                .clone()
                .ok_or_else(|| err("release identity missing"))?;
            let (actual, class, state, marked, width) =
                live.remove(&id).ok_or_else(|| err("unmatched release"))?;
            if class != leaf.class || state != leaf.state || width != leaf.wires.len() {
                return Err(err("release metadata mismatch"));
            }
            if marked && scratch.pop().as_ref() != Some(&id) {
                return Err(err("library scratch release is not LIFO"));
            }
            leaf.scratch = marked;
            leaf.allocation = Some(actual);
            for wire in &mut leaf.wires {
                *wire = wires
                    .remove(wire)
                    .ok_or_else(|| err("unmatched release wire"))?;
            }
        } else {
            let resolve = |wire: &String| -> Result<String, String> {
                if wire.starts_with("$decl") {
                    wires.get(wire).cloned().ok_or_else(|| {
                        err(format!(
                            "inactive allocation wire use: {wire} at leaf {index} {}",
                            leaf.operation
                        ))
                    })
                } else {
                    Ok(wire.clone())
                }
            };
            leaf.wires = leaf.wires.iter().map(resolve).collect::<Result<_, _>>()?;
            leaf.controls = leaf
                .controls
                .iter()
                .map(resolve)
                .collect::<Result<_, _>>()?;
        }
    }
    if !live.is_empty() {
        return Err(err("unmatched allocation/declaration left live"));
    }
    Ok(())
}
fn bind_formals(
    formals: &[Json],
    args: &[Json],
    outer: &Bindings,
    capture: &Bindings,
) -> Result<Bindings, String> {
    if formals.len() != args.len() {
        return Err(err("arity mismatch"));
    }
    validate_formals(formals)?;
    let mut env = capture.clone();
    env.locals.clear();
    for f in formals {
        let name = f
            .as_str()
            .map(Ok)
            .unwrap_or_else(|| string(field(f, "name")?))?;
        env.values.remove(name);
    }
    let mut names = BTreeSet::new();
    for (f, a) in formals.iter().zip(args) {
        let (name, ty) = if let Some(n) = f.as_str() {
            (n, "integer")
        } else {
            (string(field(f, "name")?)?, string(field(f, "type")?)?)
        };
        if !names.insert(name) {
            return Err(err("duplicate formal"));
        }
        let value = match ty {
            "integer" => Bound::Integer(scalar(a, outer)?),
            "wire" => {
                let w = wires(a, outer)?;
                if w.len() != scalar(field(f, "width")?, &env)?.usize()? {
                    return Err(err("wire width mismatch"));
                }
                if w.iter().collect::<BTreeSet<_>>().len() != w.len() {
                    return Err(err("duplicate wire actual"));
                }
                Bound::Wire(w)
            }
            "environment" => {
                let w = wires(a, outer)?;
                env.add_controls(w.iter().cloned());
                Bound::Wire(w)
            }
            "angle" => Bound::Angle(angle(a, outer)?),
            "symbol" => {
                if let Some(symbol @ Bound::Symbol { .. }) =
                    a.as_str().and_then(|n| outer.values.get(n))
                {
                    symbol.clone()
                } else {
                    let hash = string(a)?;
                    if hash.len() != 64
                        || !hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    {
                        return Err(err("invalid closure hash"));
                    }
                    let registry = outer
                        .registry
                        .clone()
                        .ok_or_else(|| err("linked closure registry missing"))?;
                    registry.name(hash)?;
                    Bound::Symbol {
                        hash: hash.into(),
                        registry,
                        adjoint: false,
                    }
                }
            }
            "arguments" => {
                let symbol = string(field(f, "symbol")?)?;
                let Some(Bound::Symbol { hash, registry, .. }) = env.values.get(symbol) else {
                    return Err(err("arguments symbol must be an earlier symbol formal"));
                };
                if let Some(Bound::Arguments {
                    hash: bound,
                    values,
                    oracle_invocation,
                    oracle_identity,
                }) = a.as_str().and_then(|n| outer.values.get(n))
                {
                    if bound != hash {
                        return Err(err("arguments closure mismatch"));
                    }
                    Bound::Arguments {
                        hash: hash.clone(),
                        values: values.clone(),
                        oracle_invocation: oracle_invocation.clone(),
                        oracle_identity: *oracle_identity,
                    }
                } else {
                    let name = registry.name(hash)?;
                    let oracle_invocation = if registry
                        .object
                        .function(name)
                        .is_some_and(|f| f.kind == crate::object::FnKind::Oracle)
                    {
                        oracle_source::invocation_budget(
                            &registry.object,
                            outer.oracle_invocation.as_ref(),
                        )?;
                        outer.oracle_invocation.clone()
                    } else {
                        None
                    };
                    let formals = source::interface(&registry.object, name)?;
                    let bindings = bind_formals(&formals, arr(a)?, outer, &Bindings::new())?;
                    let values = formals
                        .iter()
                        .map(|f| Ok(bindings.values[string(field(f, "name")?)?].clone()))
                        .collect::<Result<_, String>>()?;
                    Bound::Arguments {
                        hash: hash.clone(),
                        values,
                        oracle_invocation,
                        oracle_identity: outer.oracle_identity,
                    }
                }
            }
            "allocation" => match a.as_str().and_then(|n| outer.values.get(n)) {
                Some(Bound::Allocation(a)) => Bound::Allocation(a.clone()),
                _ => return Err(err("unbound allocation actual")),
            },
            "class" => Bound::Class(class(a, outer)?),
            "image" => Bound::Image(image(a, outer)?),
            "event" => {
                let value = match a.as_str().and_then(|n| outer.values.get(n)) {
                    Some(Bound::Event(e)) => e.clone(),
                    _ => a.clone(),
                };
                keys(&value, &["instruction", "point"])?;
                string(field(&value, "instruction")?)?;
                string(field(&value, "point")?)?;
                Bound::Event(value)
            }
            "region" => bind_region(f, a, outer, &env)?,
            "value" => {
                let ty = field(f, "isa_type")?;
                if let Some(Bound::Value(value, source_ty, identity)) =
                    a.as_str().and_then(|n| outer.values.get(n))
                {
                    if source_ty != ty {
                        return Err(err("value isa_type mismatch"));
                    }
                    Bound::Value(value.clone(), ty.clone(), *identity)
                } else {
                    let isa_ty = if let Some(ptr) = ty.get("ptr") {
                        keys(ty, &["ptr"])?;
                        let width = ptr
                            .as_usize()
                            .filter(|n| *n > 0)
                            .ok_or_else(|| err("invalid pointer width"))?;
                        crate::invocation::decode_args(
                            &Json::Arr(vec![a.clone()]),
                            &[crate::object::Type::Ptr],
                            width,
                        )?;
                        None
                    } else {
                        let text = string(ty)?;
                        let parsed = crate::object::Type::parse(text)?;
                        if !matches!(parsed, crate::object::Type::I(n) if n > 0)
                            && !matches!(
                                parsed,
                                crate::object::Type::F64 | crate::object::Type::Token
                            )
                        {
                            return Err(err("invalid value isa_type"));
                        }
                        if parsed.signature_text() != text {
                            return Err(err("noncanonical value isa_type"));
                        }
                        Some(parsed)
                    };
                    if let Some(t) = isa_ty {
                        crate::invocation::decode_args(&Json::Arr(vec![a.clone()]), &[t], 1)?;
                    }
                    Bound::Value(a.clone(), ty.clone(), None)
                }
            }
            "records" => bind_records(f, a, outer, &env)?,
            "templates" => {
                if let Some(Bound::Templates(t, e)) = a.as_str().and_then(|n| outer.values.get(n)) {
                    if arr(t)?.len() != scalar(field(f, "length")?, &env)?.usize()? {
                        return Err(err("templates length mismatch"));
                    }
                    Bound::Templates(t.clone(), e.clone())
                } else {
                    let ts = arr(a)?;
                    if ts.len() != scalar(field(f, "length")?, &env)?.usize()? {
                        return Err(err("templates length mismatch"));
                    }
                    for t in ts {
                        keys(t, &["parameters", "body"])?;
                        validate_formals(arr(field(t, "parameters")?)?)?;
                        syntax_inner(field(t, "body")?, outer.source_formed)?;
                    }
                    Bound::Templates(a.clone(), Box::new(outer.clone()))
                }
            }
            "body" => {
                if let Some(Bound::Body(b, e)) = a.as_str().and_then(|n| outer.values.get(n)) {
                    Bound::Body(b.clone(), e.clone())
                } else {
                    syntax_inner(a, outer.source_formed)?;
                    Bound::Body(a.clone(), Box::new(outer.clone()))
                }
            }
            _ => {
                return Err(err(format!(
                    "unavailable.reason: typed {ty} binding/linked-source formation is unavailable"
                )));
            }
        };
        env.locals.insert(name.into());
        env.values.insert(name.into(), value);
    }
    Ok(env)
}
fn bind_region(f: &Json, a: &Json, outer: &Bindings, env: &Bindings) -> Result<Bound, String> {
    let address = scalar(field(f, "address")?, env)?.usize()?;
    let cell = scalar(field(f, "cell")?, env)?.usize()?;
    let role = string(field(f, "role")?)?;
    if cell == 0 || !["in", "mut"].contains(&role) {
        return Err(err("invalid region formal"));
    }
    let value = if let Some(r @ Bound::Region { .. }) = a.as_str().and_then(|n| outer.values.get(n))
    {
        r.clone()
    } else {
        keys(a, &["address", "cell", "role", "origin", "cells"])?;
        let origin = string(field(a, "origin")?)?;
        let suffix = origin
            .strip_prefix("interface:")
            .or_else(|| origin.strip_prefix("local:"))
            .ok_or_else(|| err("invalid region origin"))?;
        if suffix.is_empty()
            || !suffix.bytes().enumerate().all(|(i, b)| {
                b.is_ascii_alphabetic() || b == b'_' || i > 0 && (b.is_ascii_digit() || b == b'.')
            })
        {
            return Err(err("invalid region origin NAME"));
        }
        Bound::Region {
            address: field(a, "address")?
                .as_usize()
                .ok_or_else(|| err("invalid region address"))?,
            cell: field(a, "cell")?
                .as_usize()
                .ok_or_else(|| err("invalid region cell"))?,
            role: string(field(a, "role")?)?.into(),
            origin: origin.into(),
            cells: wires(field(a, "cells")?, outer)?,
        }
    };
    let Bound::Region {
        address: actual_a,
        cell: actual_w,
        role: actual_role,
        cells,
        origin,
    } = &value
    else {
        unreachable!()
    };
    let width = u32::try_from(address)
        .ok()
        .and_then(|a| 1usize.checked_shl(a))
        .and_then(|n| n.checked_mul(cell))
        .ok_or_else(|| err("region width exceeds host limit"))?;
    if *actual_a != address
        || *actual_w != cell
        || actual_role != role
        || cells.len() != width
        || origin.is_empty()
    {
        return Err(err("region shape/role/width mismatch"));
    }
    if cells.iter().collect::<BTreeSet<_>>().len() != cells.len() {
        return Err(err("duplicate region wire"));
    }
    Ok(value)
}
fn bind_records(f: &Json, a: &Json, outer: &Bindings, env: &Bindings) -> Result<Bound, String> {
    let fields = field(f, "fields")?;
    let fs = arr(fields)?;
    validate_formals(fs)?;
    let names = fs
        .iter()
        .map(|f| {
            f.as_str()
                .map(Ok)
                .unwrap_or_else(|| string(field(f, "name")?))
        })
        .collect::<Result<Vec<_>, String>>()?;
    for f in fs {
        if f.as_str().is_none()
            && !["integer", "wire", "region", "class", "allocation", "image"]
                .contains(&string(field(f, "type")?)?)
        {
            return Err(err("invalid record field type"));
        }
    }
    let order = field(f, "order")?;
    let mut ordered = BTreeSet::new();
    for k in arr(order)? {
        let k = string(k)?;
        if !ordered.insert(k)
            || !fs.iter().any(|f| {
                f.as_str() == Some(k)
                    || f.get("name").and_then(Json::as_str) == Some(k)
                        && f.get("type").and_then(Json::as_str) == Some("integer")
            })
        {
            return Err(err("invalid record order"));
        }
    }
    let rows = if let Some(Bound::Records {
        fields: old_fields,
        order: old_order,
        rows,
    }) = a.as_str().and_then(|n| outer.values.get(n))
    {
        if old_fields != fields || old_order != order {
            return Err(err("record schema/order mismatch"));
        }
        // Revalidate the receiving widths with resolved identities, never wire names.
        for row in rows {
            let mut local = env.clone();
            for f in fs {
                let name = f
                    .as_str()
                    .map(Ok)
                    .unwrap_or_else(|| string(field(f, "name")?))?;
                let value = &row[name];
                if let Bound::Wire(w) = value
                    && w.len() != scalar(field(f, "width")?, &local)?.usize()?
                {
                    return Err(err("forwarded record width mismatch"));
                }
                if let Bound::Region {
                    address,
                    cell,
                    role,
                    ..
                } = value
                    && (*address != scalar(field(f, "address")?, &local)?.usize()?
                        || *cell != scalar(field(f, "cell")?, &local)?.usize()?
                        || role != string(field(f, "role")?)?)
                {
                    return Err(err("forwarded record region mismatch"));
                }
                local.values.insert(name.into(), value.clone());
            }
        }
        rows.clone()
    } else {
        let mut rows = vec![];
        for row in arr(a)? {
            keys(row, &names)?;
            let args = names
                .iter()
                .map(|n| field(row, n).cloned())
                .collect::<Result<Vec<_>, _>>()?;
            let local = bind_formals(fs, &args, outer, env)?;
            rows.push(
                names
                    .iter()
                    .map(|n| ((*n).into(), local.values[*n].clone()))
                    .collect::<BTreeMap<_, _>>(),
            );
        }
        rows
    };
    let mut previous = None;
    for row in &rows {
        let tuple = arr(order)?
            .iter()
            .map(|k| match row.get(string(k)?) {
                Some(Bound::Integer(n)) => Ok(n.clone()),
                _ => Err(err("noninteger record order field")),
            })
            .collect::<Result<Vec<_>, String>>()?;
        if !tuple.is_empty() && previous.as_ref().is_some_and(|p| p >= &tuple) {
            return Err(err("records not strictly ordered"));
        }
        previous = Some(tuple);
    }
    Ok(Bound::Records {
        fields: fields.clone(),
        order: order.clone(),
        rows,
    })
}
fn class(j: &Json, e: &Bindings) -> Result<String, String> {
    if let Some(Bound::Class(c)) = j.as_str().and_then(|n| e.values.get(n)) {
        return Ok(c.clone());
    }
    keys(j, &["class"])?;
    let c = string(field(j, "class")?)?;
    if c.is_empty() {
        return Err(err("empty class"));
    }
    Ok(c.into())
}
fn image(j: &Json, e: &Bindings) -> Result<String, String> {
    if let Some(Bound::Image(i)) = j.as_str().and_then(|n| e.values.get(n)) {
        return Ok(i.clone());
    }
    let s = string(j)?;
    if !s.len().is_multiple_of(2)
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(err("invalid image hex or unbound image"));
    }
    Ok(s.into())
}
fn validate_formals(fs: &[Json]) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for f in fs {
        let name = if let Some(n) = f.as_str() {
            n
        } else {
            string(field(f, "name")?)?
        };
        if !names.insert(name) {
            return Err(err("duplicate formal"));
        }
        if f.as_str().is_some() {
            continue;
        }
        let extra: &[&str] = match string(field(f, "type")?)? {
            "integer" | "angle" | "body" | "symbol" | "environment" | "event" | "image"
            | "class" | "allocation" => &[],
            "wire" => &["width"],
            "templates" => &["length"],
            "arguments" => &["symbol"],
            "value" => &["isa_type"],
            "records" => &["fields", "order"],
            "region" => &["address", "cell", "role"],
            _ => return Err(err("unknown formal type")),
        };
        let mut allowed = vec!["name", "type"];
        allowed.extend(extra);
        keys(f, &allowed)?;
        for k in extra {
            field(f, k)?;
        }
    }
    Ok(())
}
fn angle(j: &Json, e: &Bindings) -> Result<u64, String> {
    let s = string(j)?;
    if let Some(Bound::Angle(a)) = e.values.get(s) {
        return Ok(*a);
    }
    let hex = s
        .strip_prefix("0x")
        .filter(|h| {
            h.len() == 16
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(|| err("invalid angle bits"))?;
    let bits = u64::from_str_radix(hex, 16).map_err(err)?;
    if !f64::from_bits(bits).is_finite() {
        return Err(err("nonfinite angle"));
    }
    Ok(bits)
}
fn scalar(j: &Json, e: &Bindings) -> Result<R, String> {
    if let Json::Num(s) = j {
        let r = s.parse::<R>()?;
        if r.decimal().contains(['.', '/']) {
            return Err(err("noninteger scalar"));
        }
        return Ok(r);
    }
    if let Some(s) = j.as_str() {
        return match e.values.get(s) {
            Some(Bound::Integer(n)) => Ok(n.clone()),
            _ => Err(err(format!("unbound integer {s}"))),
        };
    }
    let a = arr(j)?;
    if a.is_empty() {
        return Err(err("empty scalar expression"));
    }
    let op = string(&a[0])?;
    if a.len() != 3 {
        return Err(err("scalar arity"));
    }
    let x = scalar(&a[1], e)?;
    let y = scalar(&a[2], e)?;
    Ok(match op {
        "+" => x.add(&y),
        "-" => x.sub(&y),
        "*" => x.mul(&y),
        "div" | "mod" => {
            if y <= R::zero() {
                return Err(err("nonpositive divisor"));
            }
            let d = x.div(&y)?;
            let q = R::zero().sub(&R::zero().sub(&d).ceil());
            if op == "div" { q } else { x.sub(&q.mul(&y)) }
        }
        "bit" => {
            let n = crate::bigint::BigUint::parse_decimal(&x.decimal())?;
            R::from(u64::from(n.bit(y.usize()?)))
        }
        _ => return Err(err("unknown scalar operator")),
    })
}
fn wires(j: &Json, e: &Bindings) -> Result<Vec<String>, String> {
    if let Some(s) = j.as_str() {
        if let Some(Bound::Wire(w)) = e.values.get(s) {
            return Ok(w.clone());
        }
        if let Some(Bound::Region { cells, .. }) = e.values.get(s) {
            return Ok(cells.clone());
        }
        if let Some(Bound::Allocation(a)) = e.values.get(s) {
            let a = a.borrow();
            return Ok(a.wires.clone());
        }
        if let Some((name, index)) = s.split_once('[') {
            let index = index
                .strip_suffix(']')
                .ok_or_else(|| err("invalid wire subscript"))?;
            let i = scalar(&json::parse(index)?, e)?.usize()?;
            let w = wires(&Json::Str(name.into()), e)?;
            return Ok(vec![
                w.get(i)
                    .ok_or_else(|| err("wire index outside width"))?
                    .clone(),
            ]);
        }
        return Err(err(format!("unbound wire {s}")));
    }
    if let Json::Arr(a) = j {
        let mut out = vec![];
        for x in a {
            out.extend(wires(x, e)?);
        }
        return Ok(out);
    }
    keys(j, &["range", "from", "to"])?;
    let w = wires(field(j, "range")?, e)?;
    let a = scalar(field(j, "from")?, e)?.usize()?;
    let b = scalar(field(j, "to")?, e)?.usize()?;
    w.get(a..b)
        .map(|x| x.to_vec())
        .ok_or_else(|| err("invalid wire range"))
}
/// Check the closed syntax even in unused constructor bodies. The authenticated
/// catalog may contain constructors whose execution remains explicitly staged.
fn syntax(body: &Json) -> Result<(), String> {
    syntax_inner(body, false)
}
fn syntax_inner(body: &Json, source_formed: bool) -> Result<(), String> {
    for c in arr(body)? {
        let variants: [(&str, &[&str]); 17] = [
            ("gate", &["gate", "wires", "params"]),
            ("alloc", &["alloc", "width", "class", "state"]),
            ("release", &["release"]),
            ("classical", &["classical", "point"]),
            ("for", &["for", "from", "to", "body"]),
            ("invoke", &["invoke", "args"]),
            ("inverse", &["inverse", "body"]),
            ("each", &["each", "in", "body"]),
            ("splice", &["splice"]),
            ("call", &["call", "args"]),
            ("under", &["under", "body"]),
            ("image", &["image", "wires"]),
            ("apply", &["apply", "index", "args"]),
            ("visit", &["visit", "body"]),
            ("declare", &["declare", "width", "class", "state", "body"]),
            ("acquire", &["acquire"]),
            ("relinquish", &["relinquish"]),
        ];
        if source_formed && c.get("$oracle_storage").is_some() {
            keys(c, &["$oracle_storage", "body"])?;
            for entry in arr(field(c, "$oracle_storage")?)? {
                keys(entry, &["name", "width"])?;
                string(field(entry, "name")?)?;
                field(entry, "width")?
                    .as_usize()
                    .ok_or_else(|| err("invalid source storage width"))?;
            }
            syntax_inner(field(c, "body")?, source_formed)?;
            continue;
        }
        if source_formed && c.get("$oracle_invocation").is_some() {
            keys(c, &["$oracle_invocation", "body"])?;
            string(field(c, "$oracle_invocation")?)?;
            syntax_inner(field(c, "body")?, source_formed)?;
            continue;
        }
        if source_formed && c.get("$source").is_some() {
            keys(c, &["$source", "body"])?;
            syntax_inner(field(c, "body")?, true)?;
            continue;
        }
        if source_formed && c.get("$source_allocation").is_some() {
            keys(c, &["$source_allocation", "body"])?;
            let metadata = field(c, "$source_allocation")?;
            keys(metadata, &["name", "class", "state"])?;
            string(field(metadata, "name")?)?;
            if string(field(metadata, "class")?)?.is_empty()
                || !["clean", "borrowed"].contains(&string(field(metadata, "state")?)?)
            {
                return Err(err("invalid source allocation metadata"));
            }
            syntax_inner(field(c, "body")?, true)?;
            continue;
        }
        if source_formed && c.get("$history_target").is_some() {
            keys(c, &["$history_target", "body"])?;
            let target = field(c, "$history_target")?;
            keys(target, &["target", "temporaries"])?;
            string(field(target, "target")?)?;
            for name in arr(field(target, "temporaries")?)? {
                string(name)?;
            }
            syntax_inner(field(c, "body")?, true)?;
            continue;
        }
        if source_formed && c.get("$cleanup_pending").is_some() {
            keys(c, &["$cleanup_pending", "body"])?;
            if field(c, "$cleanup_pending")? != &Json::Bool(true) {
                return Err(err("invalid pending source cleanup"));
            }
            syntax_inner(field(c, "body")?, true)?;
            continue;
        }
        if source_formed && c.get("$cleanup").is_some() {
            keys(c, &["$cleanup", "body"])?;
            let pair = field(c, "$cleanup")?;
            keys(
                pair,
                &[
                    "target",
                    "point",
                    "computing",
                    "uncomputing",
                    "perm",
                    "recompute_supported",
                    "input_request",
                    "allocations",
                    "targets",
                    "initialising",
                    "uninitialising",
                    "before_recompute",
                    "live_versions",
                    "zero_targets",
                    "release_targets",
                ],
            )?;
            syntax_inner(field(pair, "computing")?, true)?;
            syntax_inner(field(pair, "uncomputing")?, true)?;
            if let Some(outside) = pair.get("before_recompute") {
                syntax_inner(outside, true)?;
            }
            syntax_inner(field(c, "body")?, true)?;
            continue;
        }
        let allowed = variants
            .iter()
            .find(|(k, _)| c.get(k).is_some())
            .ok_or_else(|| err("unknown constructor"))?
            .1;
        keys(c, allowed)?;
        if let Some(v) = c.get("inverse")
            && v != &Json::Bool(true)
        {
            return Err(err("inverse must be true"));
        }
        if let Some(b) = c.get("body") {
            syntax_inner(b, source_formed)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    fn j(s: &str) -> Json {
        json::parse(s).unwrap()
    }

    #[test]
    fn scratch_mark_survives_aliases_and_inverse_release() {
        let mut c = Catalog::published().unwrap();
        let Json::Obj(doc) = &mut c.doc else { panic!() };
        let Json::Arr(recipes) = doc.get_mut("recipes").unwrap() else {
            panic!()
        };
        recipes.push(j(r#"{"kind":"marked.acquire","parameters":[{"name":"D","type":"allocation"}],"scratch":["D"],"body":[{"acquire":"D"}]}"#));
        recipes.push(j(r#"{"kind":"alias.release","parameters":[{"name":"Other","type":"allocation"}],"scratch":[],"body":[{"relinquish":"Other"}]}"#));
        let body = r#"[{"declare":"d","width":1,"class":{"class":"fast"},"state":"clean","body":[{"invoke":"marked.acquire","args":["d"]},{"invoke":"alias.release","args":["d"]}]}]"#;
        let mut b = Bindings::new();
        b.body("B", j(body)).unwrap();
        b.body("I", j(r#"[{"inverse":true,"body":[{"splice":"B"}]}]"#))
            .unwrap();
        let inverse = c.expand("adj", &[j(r#""I""#)], &b).unwrap();
        assert!(inverse.iter().all(|l| l.scratch));
        b.body("bad",j(r#"[{"declare":"d","width":1,"class":{"class":"fast"},"state":"clean","body":[{"declare":"e","width":1,"class":{"class":"fast"},"state":"clean","body":[{"invoke":"marked.acquire","args":["d"]},{"invoke":"marked.acquire","args":["e"]},{"invoke":"alias.release","args":["d"]},{"invoke":"alias.release","args":["e"]}]}]}]"#)).unwrap();
        assert!(
            c.expand("adj", &[j(r#""bad""#)], &b)
                .unwrap_err()
                .contains("LIFO")
        );
    }

    #[test]
    fn ordered_records_revalidate_shape_and_capture_wire_identities() {
        let f = j(
            r#"{"name":"Layout","type":"records","fields":["base","extent",{"name":"cells","type":"wire","width":["*",8,"extent"]}],"order":["base"]}"#,
        );
        let mut b = Bindings::new();
        b.wire("a", 8);
        b.wire("b", 16);
        let a = j(r#"[{"base":0,"extent":1,"cells":"a"},{"base":1,"extent":2,"cells":"b"}]"#);
        let records = bind_records(&f, &a, &b, &b).unwrap();
        b.values.insert("L".into(), records);
        b.wire("a", 1);
        let forwarded = bind_records(&f, &j(r#""L""#), &b, &b).unwrap();
        let Bound::Records { rows, .. } = forwarded else {
            panic!()
        };
        assert!(matches!(&rows[0]["cells"], Bound::Wire(w) if w.len() == 8));
        for a in [
            r#"[{"base":1,"extent":1,"cells":"b"}]"#,
            r#"[{"base":1,"extent":2,"cells":"b"},{"base":1,"extent":2,"cells":"b"}]"#,
            r#"[{"base":1,"extent":2,"cells":"b","extra":0}]"#,
        ] {
            assert!(bind_records(&f, &j(a), &b, &b).is_err());
        }
    }

    #[test]
    fn value_formals_preserve_bits_and_require_identical_types() {
        let fs = j(
            r#"[{"name":"x","type":"value","isa_type":"i128"},{"name":"f","type":"value","isa_type":"f64"},{"name":"tk","type":"value","isa_type":"token"}]"#,
        );
        let a = j(r#"["18446744073709551617","0x7ff8000000000001",["2","0","0","0"]]"#);
        let b = Bindings::new();
        let cap = bind_formals(arr(&fs).unwrap(), arr(&a).unwrap(), &b, &b).unwrap();
        let aliases = j(r#"["x","f","tk"]"#);
        bind_formals(arr(&fs).unwrap(), arr(&aliases).unwrap(), &cap, &b).unwrap();
        let changed = j(&fs.to_jcs().replace("i128", "i129"));
        assert!(bind_formals(arr(&changed).unwrap(), arr(&aliases).unwrap(), &cap, &b).is_err());
        for a in [
            r#"["340282366920938463463374607431768211456","0x7ff8000000000001",["2","0","0","0"]]"#,
            r#"["1","0x7ff8000000000001",["2","0","0"]]"#,
        ] {
            assert!(bind_formals(arr(&fs).unwrap(), arr(&j(a)).unwrap(), &b, &b).is_err());
        }
    }
}
