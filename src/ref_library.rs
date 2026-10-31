//! Independent consumer of the published §11.1 catalog. Source linking is a
//! separate prerequisite; successful catalog authentication is not a schedule verdict.
use crate::{
    json::{self, Json},
    target::{Rational as R, arr, field, keys, string},
};
use std::{
    cell::RefCell,
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
}
#[derive(Clone, Debug)]
struct Allocation {
    wires: Vec<String>,
    class: String,
    state: String,
    live: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    values: BTreeMap<String, Bound>,
    controls: BTreeSet<String>,
}
impl Bindings {
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub ordinal: usize,
    pub operation: String,
    pub wires: Vec<String>,
    pub controls: BTreeSet<String>,
    pub angle: Option<u64>,
    pub class: Option<String>,
    pub state: Option<String>,
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
        if let Some(a) = &mut self.angle {
            *a ^= 1 << 63;
        }
    }
}
pub struct Catalog {
    doc: Json,
}
impl Catalog {
    pub fn published() -> Result<Self, String> {
        let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bennett-isa-spec/spec");
        let bound = spec.join("ref-0.9.json");
        let path = if bound.exists() {
            bound
        } else {
            spec.join("ref-0.9-draft.json")
        };
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
            syntax(field(r, "body")?)?;
        }
        Ok(Self { doc })
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
        let mut out = vec![];
        let mut next = 0;
        let mut active = vec![];
        self.invoke(kind, args, bindings, &mut out, &mut next, &mut active)?;
        Ok(out)
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
        let mut env = outer.clone();
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
                    env.controls.extend(w.iter().cloned());
                    Bound::Wire(w)
                }
                "angle" => Bound::Angle(angle(a, outer)?),
                "templates" => {
                    if let Some(Bound::Templates(t, e)) =
                        a.as_str().and_then(|n| outer.values.get(n))
                    {
                        Bound::Templates(t.clone(), e.clone())
                    } else {
                        let ts = arr(a)?;
                        if ts.len() != scalar(field(f, "length")?, &env)?.usize()? {
                            return Err(err("templates length mismatch"));
                        }
                        for t in ts {
                            keys(t, &["parameters", "body"])?;
                            arr(field(t, "parameters")?)?;
                            syntax(field(t, "body")?)?;
                        }
                        Bound::Templates(a.clone(), Box::new(outer.clone()))
                    }
                }
                "body" => {
                    if let Some(Bound::Body(b, e)) = a.as_str().and_then(|n| outer.values.get(n)) {
                        Bound::Body(b.clone(), e.clone())
                    } else {
                        syntax(a)?;
                        Bound::Body(a.clone(), Box::new(outer.clone()))
                    }
                }
                _ => {
                    return Err(err(format!(
                        "unavailable.reason: typed {ty} binding/linked-source formation is staged in bennett-vm-47t"
                    )));
                }
            };
            env.values.insert(name.into(), value);
        }
        active.push(kind.into());
        let result = self.walk(field(r, "body")?, &mut env, out, next, active);
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
            if let Some(g) = c.get("gate") {
                let op = string(g)?;
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
                    operation: op.into(),
                    wires: ws,
                    controls: env.controls.clone(),
                    angle: a,
                    class: None,
                    state: None,
                });
                *next += 1;
            } else if let Some(n) = c.get("for") {
                let name = string(n)?;
                let from = scalar(field(c, "from")?, env)?.usize()?;
                let to = scalar(field(c, "to")?, env)?.usize()?;
                for i in from..to {
                    let mut local = env.clone();
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
                self.walk(field(c, "body")?, env, out, next, active)?;
                out[start..].reverse();
                for l in &mut out[start..] {
                    l.adjoint();
                }
            } else if let Some(n) = c.get("splice") {
                let Some(Bound::Body(b, cap)) = env.values.get(string(n)?) else {
                    return Err(err("unbound body"));
                };
                let b = b.clone();
                let mut cap = *cap.clone();
                cap.controls.extend(env.controls.clone());
                self.walk(&b, &mut cap, out, next, active)?;
            } else if let Some(u) = c.get("under") {
                let mut local = env.clone();
                local.controls.extend(wires(u, env)?);
                self.walk(field(c, "body")?, &mut local, out, next, active)?;
            } else if let Some(n) = c.get("declare") {
                let name = string(n)?;
                if env.values.contains_key(name) {
                    return Err(err("declaration shadows binding"));
                }
                let width = scalar(field(c, "width")?, env)?.usize()?;
                let class = string(field(field(c, "class")?, "class")?)?.to_string();
                if class.is_empty() {
                    return Err(err("empty class"));
                }
                let state = string(field(c, "state")?)?.to_string();
                if !["clean", "borrowed"].contains(&state.as_str()) {
                    return Err(err("invalid allocation state"));
                }
                let id = *next;
                let a = Rc::new(RefCell::new(Allocation {
                    wires: (0..width)
                        .map(|i| format!("$alloc{id}.{name}[{i}]"))
                        .collect(),
                    class,
                    state,
                    live: false,
                }));
                let mut local = env.clone();
                local
                    .values
                    .insert(name.into(), Bound::Allocation(a.clone()));
                self.walk(field(c, "body")?, &mut local, out, next, active)?;
                if a.borrow().live {
                    return Err(err("declaration left live"));
                }
            } else if let Some(n) = c.get("acquire").or_else(|| c.get("relinquish")) {
                let Some(Bound::Allocation(a)) = env.values.get(string(n)?) else {
                    return Err(err("unbound allocation"));
                };
                let mut a = a.borrow_mut();
                let acquire = c.get("acquire").is_some();
                if a.live == acquire {
                    return Err(err("invalid allocation lifetime"));
                }
                a.live = acquire;
                out.push(Leaf {
                    ordinal: *next,
                    operation: if acquire { "alloc" } else { "release" }.into(),
                    wires: a.wires.clone(),
                    controls: BTreeSet::new(),
                    angle: None,
                    class: Some(a.class.clone()),
                    state: Some(a.state.clone()),
                });
                *next += 1;
            } else if let Some(n) = c.get("apply") {
                let Some(Bound::Templates(t, cap)) = env.values.get(string(n)?) else {
                    return Err(err("unbound templates"));
                };
                let t = arr(t)?
                    .get(scalar(field(c, "index")?, env)?.usize()?)
                    .ok_or_else(|| err("template index out of range"))?
                    .clone();
                let mut local = *cap.clone();
                local.controls.extend(env.controls.clone());
                let fs = arr(field(&t, "parameters")?)?;
                let args = arr(field(c, "args")?)?;
                if fs.len() != args.len() {
                    return Err(err("template arity"));
                }
                let mut names = BTreeSet::new();
                for (f, a) in fs.iter().zip(args) {
                    let name = string(field(f, "name")?)?;
                    if !names.insert(name) {
                        return Err(err("duplicate formal"));
                    }
                    if string(field(f, "type")?)? != "wire" {
                        return Err(err(
                            "unavailable.reason: non-wire template formal staged in bennett-vm-47t",
                        ));
                    }
                    let w = wires(a, env)?;
                    if w.len() != scalar(field(f, "width")?, &local)?.usize()? {
                        return Err(err("template width mismatch"));
                    }
                    local.values.insert(name.into(), Bound::Wire(w));
                }
                self.walk(field(&t, "body")?, &mut local, out, next, active)?;
            } else {
                return Err(err(
                    "unavailable.reason: allocation/descriptor/template/symbol/image/classical expansion staged in bennett-vm-47t; catalog identity alone is not executable support",
                ));
            }
        }
        Ok(())
    }
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
        if let Some(Bound::Allocation(a)) = e.values.get(s) {
            let a = a.borrow();
            if !a.live {
                return Err(err("inactive allocation wire use"));
            }
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
        let allowed = variants
            .iter()
            .find(|(k, _)| c.get(k).is_some())
            .ok_or_else(|| err("unknown constructor"))?
            .1;
        keys(c, allowed)?;
        if let Some(b) = c.get("body") {
            syntax(b)?;
        }
    }
    Ok(())
}
