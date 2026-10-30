//! ISA §8 target contract. No vendor fields or cross-repository crate dependencies.
use crate::json::{self, Json};
pub use crate::rational::Rational;
use std::collections::{BTreeMap, BTreeSet};
type R = Rational;
type Map = BTreeMap<String, Json>;
pub(crate) fn field<'a>(j: &'a Json, k: &str) -> Result<&'a Json, String> {
    j.get(k).ok_or_else(|| format!("§8: missing {k}"))
}
pub(crate) fn obj(j: &Json) -> Result<&Map, String> {
    j.as_obj().ok_or("§8: expected object".into())
}
pub(crate) fn arr(j: &Json) -> Result<&Vec<Json>, String> {
    j.as_arr().ok_or("§8: expected array".into())
}
pub(crate) fn string(j: &Json) -> Result<&str, String> {
    j.as_str().ok_or("§8: expected string".into())
}
pub(crate) fn keys(j: &Json, allowed: &[&str]) -> Result<(), String> {
    for k in obj(j)?.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(format!("§8: unknown field {k}"));
        }
    }
    Ok(())
}
pub fn number(j: &Json) -> Result<R, String> {
    match j {
        Json::Num(s) => s.parse(),
        _ => Err("§8.1: expected a JSON number".into()),
    }
}
pub(crate) fn decimal(j: &Json) -> Result<R, String> {
    match j {
        Json::Num(s) | Json::Str(s) => s.parse(),
        _ => Err("§8: expected decimal".into()),
    }
}
fn nonnegative(r: R) -> Result<R, String> {
    if r.negative() {
        Err("§8.1: costs must be nonnegative and finite".into())
    } else {
        Ok(r)
    }
}
fn limit(j: &Json) -> Result<Option<usize>, String> {
    if j.as_str() == Some("unbounded") {
        Ok(None)
    } else {
        Ok(Some(number(j)?.usize()?))
    }
}
fn boolean(j: &Json) -> Result<bool, String> {
    match j {
        Json::Bool(b) => Ok(*b),
        _ => Err("§8: expected boolean".into()),
    }
}
fn check_expr(j: &Json, bound: &[&str]) -> Result<BTreeSet<String>, String> {
    if matches!(j, Json::Num(_)) {
        number(j)?;
        return Ok(BTreeSet::new());
    }
    let mut vars = BTreeSet::new();
    if let Some(v) = j.get("var") {
        keys(j, &["var"])?;
        let v = string(v)?;
        if !bound.contains(&v) {
            return Err(format!("§8.1: unbound {v}"));
        }
        vars.insert(v.into());
    } else if let Some(op) = j.get("op") {
        keys(j, &["op", "args"])?;
        let op = string(op)?;
        let n = match op {
            "+" | "-" | "*" | "/" | "max" | "min" => 2,
            "ceil" => 1,
            "log2inv_eps" => 0,
            _ => return Err(format!("§8.1: unknown expression operator {op}")),
        };
        let args = arr(field(j, "args")?)?;
        if args.len() != n {
            return Err(format!("§8.1: {op} requires {n} arguments"));
        }
        if op == "log2inv_eps" {
            if !bound.contains(&"eps") {
                return Err("§8.1: unbound eps".into());
            }
            vars.insert("eps".into());
        }
        for a in args {
            vars.extend(check_expr(a, bound)?);
        }
    } else if j.get("table").is_some() {
        keys(j, &["table", "index"])?;
        string(field(j, "table")?)?;
        let i = string(field(j, "index")?)?;
        match i{"k"=>{if !bound.contains(&"k"){return Err("§8.1: unbound k".into());}vars.insert("k".into());},"eps_decade"=>return Err("unimplemented: eps_decade index (milestone 4); §8.1 specifies no rounding/index rule".into()),_=>return Err("§8.1: invalid table index name".into())}
    } else {
        return Err("§8.1: invalid expression AST".into());
    }
    Ok(vars)
}
pub fn eval(
    j: &Json,
    vars: &BTreeMap<String, R>,
    tables: &BTreeMap<String, Vec<R>>,
) -> Result<R, String> {
    let bound: Vec<_> = vars.keys().map(String::as_str).collect();
    let referenced = check_expr(j, &bound)?;
    if referenced.contains("eps")
        && let Some(e) = vars.get("eps")
        && (e <= &R::zero() || e > &R::from(1))
    {
        return Err("§8.1: eps outside (0,1]".into());
    }
    if matches!(j, Json::Num(_)) {
        return number(j);
    }
    if let Some(v) = j.get("var") {
        return Ok(vars[string(v)?].clone());
    }
    if let Some(t) = j.get("table") {
        let t = tables.get(string(t)?).ok_or("§8.1: unknown table")?;
        let i = vars.get("k").ok_or("§8.1: unbound k")?.usize()?;
        return t
            .get(i)
            .cloned()
            .ok_or("§8.1: table index outside array".into());
    }
    let op = string(field(j, "op")?)?;
    let a = arr(field(j, "args")?)?
        .iter()
        .map(|a| eval(a, vars, tables))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match op {
        "+" => a[0].add(&a[1]),
        "-" => a[0].sub(&a[1]),
        "*" => a[0].mul(&a[1]),
        "/" => a[0].div(&a[1])?,
        "max" => a[0].clone().max(a[1].clone()),
        "min" => a[0].clone().min(a[1].clone()),
        "ceil" => a[0].ceil(),
        "log2inv_eps" => {
            let mut e = vars.get("eps").ok_or("§8.1: unbound eps")?.clone();
            let mut n = 0;
            while e < R::from(1) {
                e = e.mul(&R::from(2));
                n += 1;
            }
            R::from(n)
        }
        _ => unreachable!(),
    })
}
#[derive(Clone, Debug)]
pub struct Class {
    pub capacity: Option<usize>,
    pub dirty_rate: R,
    pub qubit_second: R,
}
#[derive(Clone, Debug)]
pub struct Gate {
    pub op: String,
    pub controls: Vec<usize>,
    pub own_controls: usize,
    pub targets: Vec<usize>,
    /// In control order then target order.
    pub classes: Vec<String>,
    pub theta: Option<R>,
}
#[derive(Clone, Debug)]
pub struct Leaf {
    pub gate: Gate,
    pub cost: BTreeMap<String, R>,
    pub epsilon: R,
}
#[derive(Clone, Debug)]
pub struct Lowered {
    pub leaves: Vec<Leaf>,
    pub ancillae: usize,
    pub ancilla_class: Option<String>,
    pub dirty: bool,
}
#[derive(Clone, Debug)]
pub struct Target {
    pub hash: String,
    pub document: Json,
    pub classes: BTreeMap<String, Class>,
    pub tables: BTreeMap<String, Vec<R>>,
}
impl Target {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let j = json::parse(raw)?;
        if j.to_jcs() != raw {
            return Err("§8.1: target must be in RFC 8785 canonical form".into());
        }
        keys(
            &j,
            &[
                "ptr_widths_supported",
                "resources",
                "classes",
                "gates",
                "schemes",
                "qram",
                "capabilities",
                "objective",
                "tables",
                "imports",
            ],
        )?;
        for k in [
            "ptr_widths_supported",
            "resources",
            "classes",
            "gates",
            "schemes",
            "qram",
            "capabilities",
            "objective",
            "tables",
            "imports",
        ] {
            field(&j, k)?;
        }
        let widths = arr(field(&j, "ptr_widths_supported")?)?;
        if widths.is_empty() {
            return Err("§8.3: empty pointer widths".into());
        }
        for w in widths {
            if ![Some(32), Some(64)].contains(&w.as_u64()) {
                return Err("§8.3: invalid pointer width".into());
            }
        }
        let mut classes = BTreeMap::new();
        for (name, c) in obj(field(&j, "classes")?)? {
            if name == "any" || name == "*" {
                return Err("§8.3: reserved class name".into());
            }
            keys(c, &["capacity", "dirty_rate", "qubit_second"])?;
            classes.insert(
                name.clone(),
                Class {
                    capacity: limit(field(c, "capacity")?)?,
                    dirty_rate: nonnegative(number(field(c, "dirty_rate")?)?)?,
                    qubit_second: nonnegative(number(field(c, "qubit_second")?)?)?,
                },
            );
        }
        if classes.is_empty() {
            return Err("§8.3: target has no classes".into());
        }
        let resources = obj(field(&j, "resources")?)?;
        for (name, kind) in [
            ("time", "time"),
            ("peak_qubits", "peak"),
            ("qubit_second", "occupancy"),
        ] {
            if resources
                .get(name)
                .and_then(|v| v.get("kind"))
                .and_then(Json::as_str)
                != Some(kind)
            {
                return Err(format!(
                    "§8.2: mandatory resource {name} must have kind {kind}"
                ));
            }
        }
        for v in resources.values() {
            keys(v, &["kind", "unit", "rate"])?;
            if !["count", "time", "occupancy", "peak"].contains(&string(field(v, "kind")?)?) {
                return Err("§8.2: unknown resource kind".into());
            }
            string(field(v, "unit")?)?;
            if let Some(rate) = v.get("rate") {
                for (c, n) in obj(rate)? {
                    if !classes.contains_key(c) {
                        return Err("§8.2: unknown rate class".into());
                    }
                    nonnegative(number(n)?)?;
                }
            }
        }
        let mut tables = BTreeMap::new();
        for (n, t) in obj(field(&j, "tables")?)? {
            tables.insert(
                n.clone(),
                arr(t)?
                    .iter()
                    .map(|v| nonnegative(number(v)?))
                    .collect::<Result<Vec<_>, String>>()?,
            );
        }
        let cap = field(&j, "capabilities")?;
        keys(
            cap,
            &[
                "measurement",
                "mbu",
                "max_parallel",
                "max_predicate_depth",
                "classical_step_time",
            ],
        )?;
        if boolean(field(cap, "mbu")?)? {
            return Err("§6.4: capabilities.mbu must be false".into());
        }
        for k in ["max_parallel", "max_predicate_depth"] {
            if limit(field(cap, k)?)? == Some(0) {
                return Err(format!("§8.3: {k} must be positive"));
            }
        }
        nonnegative(number(field(cap, "classical_step_time")?)?)?;
        let measure = field(cap, "measurement")?;
        keys(measure, &["mid_circuit", "feedforward_latency"])?;
        boolean(field(measure, "mid_circuit")?)?;
        nonnegative(number(field(measure, "feedforward_latency")?)?)?;
        let objective = field(&j, "objective")?;
        keys(objective, &["weighted", "lexicographic"])?;
        if obj(objective)?.len() != 1 {
            return Err("§8.3: objective requires weighted OR lexicographic".into());
        }
        if let Some(w) = objective.get("weighted") {
            for (n, v) in obj(w)? {
                if !resources.contains_key(n) {
                    return Err("§8.3: unknown objective resource".into());
                }
                nonnegative(number(v)?)?;
            }
        } else {
            for n in arr(field(objective, "lexicographic")?)? {
                if !resources.contains_key(string(n)?) {
                    return Err("§8.3: unknown objective resource".into());
                }
            }
        }
        for import in obj(field(&j, "imports")?)?.values() {
            keys(import, &["version", "map"])?;
            string(field(import, "version")?)?;
            for mapping in obj(field(import, "map")?)?.values() {
                let a = arr(mapping)?;
                if a.len() != 2 || !resources.contains_key(string(&a[0])?) {
                    return Err("§8.3: invalid import resource mapping".into());
                }
                nonnegative(number(&a[1])?)?;
            }
        }
        let target = Self {
            hash: crate::sha256::sha256_hex(raw.as_bytes()),
            document: j,
            classes,
            tables,
        };
        for g in arr(field(&target.document, "gates")?)? {
            target.validate_gate(g)?;
        }
        let mut names = BTreeSet::new();
        for s in arr(field(&target.document, "schemes")?)? {
            target.validate_scheme(s)?;
            if !names.insert(string(field(s, "name")?)?) {
                return Err("§8.3: duplicate scheme name".into());
            }
        }
        if let q @ Json::Obj(_) = field(&target.document, "qram")? {
            keys(q, &["load", "store", "exch"])?;
            for k in ["load", "store", "exch"] {
                let e = field(q, k)?;
                keys(e, &["cost"])?;
                target.validate_cost(field(e, "cost")?, &["a", "w", "c"])?;
            }
        } else if field(&target.document, "qram")? != &Json::Null {
            return Err("§8.3: qram must be null or an object".into());
        }
        Ok(target)
    }
    fn static_expr(&self, e: &Json) -> Result<Option<R>, String> {
        if matches!(e, Json::Num(_)) {
            return number(e).map(Some);
        }
        if e.get("var").is_some() {
            return Ok(None);
        }
        if let Some(t) = e.get("table") {
            if !self.tables.contains_key(string(t)?) {
                return Err("§8.1: unknown table".into());
            }
            return Ok(None);
        }
        let args = arr(field(e, "args")?)?;
        let values = args
            .iter()
            .map(|v| self.static_expr(v))
            .collect::<Result<Vec<_>, _>>()?;
        let op = string(field(e, "op")?)?;
        if op == "/" && values[1].as_ref().is_some_and(R::is_zero) {
            return Err("§8.1: division by zero".into());
        }
        if op == "-" && args[0] == args[1] {
            return Ok(Some(R::zero()));
        }
        if op == "log2inv_eps" || values.iter().any(Option::is_none) {
            return Ok(None);
        }
        // All variables are absent except syntactically canceled subtrees.
        // Evaluate folded arguments with exact arithmetic directly.
        let a = args
            .iter()
            .map(|v| self.static_expr(v).map(Option::unwrap))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(match op {
            "+" => a[0].add(&a[1]),
            "-" => a[0].sub(&a[1]),
            "*" => a[0].mul(&a[1]),
            "/" => a[0].div(&a[1])?,
            "min" => a[0].clone().min(a[1].clone()),
            "max" => a[0].clone().max(a[1].clone()),
            "ceil" => a[0].ceil(),
            _ => unreachable!(),
        }))
    }
    fn validate_cost(&self, cost: &Json, bound: &[&str]) -> Result<BTreeSet<String>, String> {
        let mut vars = BTreeSet::new();
        field(cost, "time")?;
        for (n, e) in obj(cost)? {
            if !obj(field(&self.document, "resources")?)?.contains_key(n) {
                return Err(format!("§8.2: undeclared resource {n}"));
            }
            vars.extend(check_expr(e, bound)?);
            if let Some(value) = self.static_expr(e)? {
                nonnegative(value)?;
            }
            if matches!(e, Json::Num(_)) {
                nonnegative(number(e)?)?;
            }
        }
        Ok(vars)
    }
    fn validate_gate(&self, g: &Json) -> Result<(), String> {
        keys(
            g,
            &["op", "classes", "controls", "params", "epsilon", "cost"],
        )?;
        registry(string(field(g, "op")?)?)?;
        controls(field(g, "controls")?)?;
        params(g)?;
        let cs = arr(field(g, "classes")?)?;
        if cs.is_empty() {
            return Err("§8.3: empty class pattern".into());
        }
        for c in cs {
            let c = string(c)?;
            if c != "*" && !self.classes.contains_key(c) {
                return Err(format!("§8.3: unknown class {c}"));
            }
        }
        let mut vars = check_expr(field(g, "epsilon")?, &["k", "c", "eps"])?;
        let constant_epsilon = self.static_expr(field(g, "epsilon")?)?;
        if let Some(value) = &constant_epsilon {
            nonnegative(value.clone())?;
        }
        vars.extend(self.validate_cost(field(g, "cost")?, &["k", "c", "eps"])?);
        if let Some(e) = constant_epsilon
            && e.is_zero()
            && vars.contains("eps")
        {
            return Err("§8.1: epsilon zero entry references eps".into());
        }
        Ok(())
    }
    fn validate_scheme(&self, s: &Json) -> Result<(), String> {
        keys(s, &["name", "realises", "ancillae", "template"])?;
        let name = string(field(s, "name")?)?;
        if name == "native" {
            return Err("§8.3: reserved scheme name".into());
        }
        let r = field(s, "realises")?;
        keys(r, &["op", "controls", "params"])?;
        registry(string(field(r, "op")?)?)?;
        controls(field(r, "controls")?)?;
        params(r)?;
        let a = field(s, "ancillae")?;
        keys(a, &["count", "state", "class"])?;
        check_expr(field(a, "count")?, &["k", "c", "theta"])?;
        if !["clean", "dirty"].contains(&string(field(a, "state")?)?) {
            return Err("§8.3: invalid ancilla state".into());
        }
        if !self.classes.contains_key(string(field(a, "class")?)?) {
            return Err("§8.3: unknown ancilla class".into());
        }
        for t in arr(field(s, "template")?)? {
            keys(t, &["gate", "wires", "params", "repeat"])?;
            registry(string(field(t, "gate")?)?)?;
            let mut bound = vec!["k", "c", "theta"];
            if let Some(rep) = t.get("repeat") {
                keys(rep, &["var", "from", "to"])?;
                if string(field(rep, "var")?)? != "i" {
                    return Err("§8.3: repeat variable must be i".into());
                }
                check_expr(field(rep, "from")?, &bound)?;
                check_expr(field(rep, "to")?, &bound)?;
                bound.push("i");
            }
            for w in arr(field(t, "wires")?)? {
                let w = string(w)?;
                if w != "t" && w != "t2" {
                    let (_, e) = subscript(w)?;
                    check_expr(&e, &bound)?;
                }
            }
            if let Some(ps) = t.get("params") {
                for p in arr(ps)? {
                    check_expr(
                        p,
                        if t.get("repeat").is_some() {
                            &["theta", "i"]
                        } else {
                            &["theta"]
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub fn supports_ptr(&self, n: usize) -> bool {
        self.document
            .get("ptr_widths_supported")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .any(|v| v.as_usize() == Some(n))
    }
    pub fn class_for(
        &self,
        requested: &str,
        n: usize,
        used: &BTreeMap<String, usize>,
    ) -> Result<String, String> {
        let mut classes = self.classes.iter().collect::<Vec<_>>();
        classes.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
        for (c, v) in classes {
            if requested != "any" && requested != c {
                continue;
            }
            let total = used
                .get(c)
                .copied()
                .unwrap_or(0)
                .checked_add(n)
                .ok_or("§8.4: capacity overflow")?;
            if v.capacity.is_none_or(|cap| total <= cap) {
                return Ok(c.clone());
            }
        }
        Err(format!(
            "§8.4: no class capacity for {requested}, {n} live qubits"
        ))
    }
    fn native(&self, g: &Gate, eps: &R) -> Result<Option<Leaf>, String> {
        for e in arr(field(&self.document, "gates")?)? {
            if !covers(e, g)? {
                continue;
            }
            let mut vars = BTreeMap::from([
                ("k".into(), R::from(g.own_controls as u64)),
                ("c".into(), R::from(g.controls.len() as u64)),
            ]);
            let epsilon = field(e, "epsilon")?;
            let exact = matches!(epsilon, Json::Num(_)) && number(epsilon)?.is_zero();
            if !exact {
                vars.insert("eps".into(), eps.clone());
            }
            let error = nonnegative(eval(epsilon, &vars, &self.tables)?)?;
            if error > *eps {
                continue;
            }
            let mut cost = BTreeMap::new();
            for (n, c) in obj(field(e, "cost")?)? {
                cost.insert(n.clone(), nonnegative(eval(c, &vars, &self.tables)?)?);
            }
            return Ok(Some(Leaf {
                gate: g.clone(),
                cost,
                epsilon: error,
            }));
        }
        Ok(None)
    }
    pub fn lower(&self, g: &Gate, eps: &R, choice: Option<&str>) -> Result<Lowered, String> {
        if g.own_controls > g.controls.len()
            || own_controls(&g.op, g.own_controls + g.targets.len())? != g.own_controls
        {
            return Err("§8.3: invalid own control/target arity".into());
        }
        if matches!(g.op.as_str(), "rx" | "ry" | "rz") != g.theta.is_some() {
            return Err("§8.3: gate parameter arity mismatch".into());
        }
        let wires = g.controls.iter().chain(&g.targets).collect::<BTreeSet<_>>();
        if wires.len() != g.controls.len() + g.targets.len() {
            return Err("§3.3: gate operands must be disjoint".into());
        }
        if eps.negative() || eps > &R::from(1) {
            return Err("§8.1: eps outside [0,1]".into());
        }
        registry(&g.op)?;
        if g.classes.len() != g.controls.len() + g.targets.len()
            || g.classes.iter().any(|c| !self.classes.contains_key(c))
        {
            return Err("§8.3: gate operand classes mismatch".into());
        }
        if choice.is_none_or(|s| s == "native") {
            if let Some(leaf) = self.native(g, eps)? {
                return Ok(Lowered {
                    leaves: vec![leaf],
                    ancillae: 0,
                    ancilla_class: None,
                    dirty: false,
                });
            }
            if choice == Some("native") {
                return Err(format!(
                    "§8.3: unsupported native {} at c={}",
                    g.op,
                    g.controls.len()
                ));
            }
        }
        for s in arr(field(&self.document, "schemes")?)? {
            if choice.is_some_and(|n| s.get("name").and_then(Json::as_str) != Some(n))
                || !covers(field(s, "realises")?, g)?
            {
                continue;
            }
            let mut vars = BTreeMap::from([
                ("k".into(), R::from(g.own_controls as u64)),
                ("c".into(), R::from(g.controls.len() as u64)),
            ]);
            if let Some(theta) = &g.theta {
                vars.insert("theta".into(), theta.clone());
            }
            let a = field(s, "ancillae")?;
            let n = eval(field(a, "count")?, &vars, &self.tables)?.usize()?;
            let class = string(field(a, "class")?)?.to_string();
            let base = g
                .controls
                .iter()
                .chain(&g.targets)
                .max()
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("§8.3: wire index overflow")?;
            let mut leaves = vec![];
            for entry in arr(field(s, "template")?)? {
                let range = if let Some(rep) = entry.get("repeat") {
                    let from = eval(field(rep, "from")?, &vars, &self.tables)?.usize()?;
                    let to = eval(field(rep, "to")?, &vars, &self.tables)?.usize()?;
                    if to < from {
                        return Err("§8.3: reversed repeat bounds".into());
                    }
                    from..to
                } else {
                    0..1
                };
                for i in range {
                    if entry.get("repeat").is_some() {
                        vars.insert("i".into(), R::from(i as u64));
                    }
                    let mut wires = vec![];
                    let mut classes = vec![];
                    for w in arr(field(entry, "wires")?)? {
                        let w = string(w)?;
                        let (wire, cls) = if w == "t" || w == "t2" {
                            let i = usize::from(w == "t2");
                            (
                                *g.targets.get(i).ok_or("§8.3: missing template target")?,
                                g.classes[g.controls.len() + i].clone(),
                            )
                        } else {
                            let (which, e) = subscript(w)?;
                            let index = eval(&e, &vars, &self.tables)?.usize()?;
                            match which {
                                "c" => (
                                    *g.controls
                                        .get(index)
                                        .ok_or("§8.3: control subscript outside array")?,
                                    g.classes
                                        .get(index)
                                        .ok_or("§8.3: missing control class")?
                                        .clone(),
                                ),
                                "a" => {
                                    if index >= n {
                                        return Err("§8.3: ancilla subscript outside array".into());
                                    }
                                    (
                                        base.checked_add(index)
                                            .ok_or("§8.3: wire index overflow")?,
                                        class.clone(),
                                    )
                                }
                                _ => unreachable!(),
                            }
                        };
                        wires.push(wire);
                        classes.push(cls);
                    }
                    let op = string(field(entry, "gate")?)?.to_string();
                    let own = own_controls(&op, wires.len())?;
                    let theta = if let Some(p) = entry.get("params") {
                        let p = arr(p)?;
                        if p.len() != 1 {
                            return Err("§8.3: template params must contain theta".into());
                        }
                        Some(eval(&p[0], &vars, &self.tables)?)
                    } else {
                        None
                    };
                    let gate = Gate {
                        op,
                        controls: wires[..own].to_vec(),
                        own_controls: own,
                        targets: wires[own..].to_vec(),
                        classes,
                        theta,
                    };
                    leaves.push(self.native(&gate, eps)?.ok_or_else(|| {
                        format!(
                            "§8.3: unsupported template gate {}; schemes do not nest",
                            gate.op
                        )
                    })?);
                }
            }
            let total = leaves.iter().fold(R::zero(), |e, g| e.add(&g.epsilon));
            if total > *eps {
                return Err("§8.3: scheme charged error exceeds epsilon_share".into());
            }
            return Ok(Lowered {
                leaves,
                ancillae: n,
                ancilla_class: Some(class),
                dirty: string(field(a, "state")?)? == "dirty",
            });
        }
        Err(format!(
            "§8.3: unsupported gate {} at c={}",
            g.op,
            g.controls.len()
        ))
    }
    pub fn import_quantity(
        &self,
        model: &str,
        version: &str,
        quantity: &str,
        value: &R,
    ) -> Result<(String, R), String> {
        let im = field(field(&self.document, "imports")?, model)?;
        if string(field(im, "version")?)? != version {
            return Err("§8.3: import model version mismatch".into());
        }
        let m = arr(field(field(im, "map")?, quantity)?)?;
        Ok((
            string(&m[0])?.into(),
            nonnegative(value.mul(&number(&m[1])?))?,
        ))
    }
}
fn registry(op: &str) -> Result<(), String> {
    if crate::action::is_gate_name(op) {
        Ok(())
    } else {
        Err(format!("§17.2: unknown gate name {op}"))
    }
}
pub fn own_controls(op: &str, wires: usize) -> Result<usize, String> {
    let (n, t) = match op {
        "cx" => (1, 1),
        "ccx" => (2, 1),
        "mcx" => (
            wires.checked_sub(1).ok_or("§8.3: mcx requires a target")?,
            1,
        ),
        "swap" | "cz" => (0, 2),
        _ => (0, 1),
    };
    if wires != n + t {
        return Err(format!("§8.3: invalid {op} operand count"));
    }
    Ok(n)
}
fn controls(j: &Json) -> Result<(usize, Option<usize>), String> {
    keys(j, &["min", "max"])?;
    let min = number(field(j, "min")?)?.usize()?;
    let max = limit(field(j, "max")?)?;
    if max.is_some_and(|m| m < min) {
        return Err("§8.3: reversed control range".into());
    }
    Ok((min, max))
}
fn params(j: &Json) -> Result<bool, String> {
    if let Some(p) = j.get("params") {
        if arr(p)?.as_slice() != [Json::Str("theta".into())] {
            return Err("§8.3: params must be [theta]".into());
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
fn covers(e: &Json, g: &Gate) -> Result<bool, String> {
    if string(field(e, "op")?)? != g.op {
        return Ok(false);
    }
    let (min, max) = controls(field(e, "controls")?)?;
    if g.controls.len() < min
        || max.is_some_and(|m| g.controls.len() > m)
        || params(e)? != g.theta.is_some()
    {
        return Ok(false);
    }
    if let Some(cs) = e.get("classes") {
        let cs = arr(cs)?;
        let matches = |p: &Json, c: &str| p.as_str() == Some("*") || p.as_str() == Some(c);
        if cs.len() == 1 {
            return Ok(g.classes.iter().all(|c| matches(&cs[0], c)));
        }
        if cs.len() != g.classes.len() {
            return Ok(false);
        }
        return Ok(cs.iter().zip(&g.classes).all(|(p, c)| matches(p, c)));
    }
    Ok(true)
}
fn subscript(w: &str) -> Result<(&str, Json), String> {
    let (name, inner) = w.split_once('[').ok_or("§8.3: invalid template wire")?;
    if !["c", "a"].contains(&name) {
        return Err("§8.3: invalid template wire".into());
    }
    let inner = inner
        .strip_suffix(']')
        .ok_or("§8.3: missing subscript bracket")?;
    let e = if ["k", "c", "i", "theta"].contains(&inner) {
        Json::obj(vec![("var", Json::Str(inner.into()))])
    } else {
        json::parse(inner)?
    };
    Ok((name, e))
}

#[derive(Clone, Debug)]
pub struct Event {
    pub start: R,
    pub duration: R,
    pub wires: Vec<usize>,
    pub cost: BTreeMap<String, R>,
}
#[derive(Clone, Debug)]
pub struct LiveInterval {
    pub wire: usize,
    pub class: String,
    pub start: R,
    pub end: R,
    pub dirty: bool,
}
impl Target {
    /// Aggregates the scheduled leaves, never logical scheme pseudo-events (§8.2).
    pub fn account(&self, events: &[Event], live: &[LiveInterval]) -> Result<Json, String> {
        let resources = obj(field(&self.document, "resources")?)?;
        let mut counts: BTreeMap<String, R> = resources
            .iter()
            .filter(|(_, v)| v.get("kind").and_then(Json::as_str) == Some("count"))
            .map(|(k, _)| (k.clone(), R::zero()))
            .collect();
        let mut end = R::zero();
        for (i, e) in events.iter().enumerate() {
            nonnegative(e.start.clone())?;
            nonnegative(e.duration.clone())?;
            end = end.max(e.start.add(&e.duration));
            let mut unique = BTreeSet::new();
            for w in &e.wires {
                if !unique.insert(w) {
                    return Err("§8.2: duplicate event wire".into());
                }
                if !live.iter().any(|l| {
                    l.wire == *w && l.start <= e.start && l.end >= e.start.add(&e.duration)
                }) {
                    return Err("§8.2: gate outside wire live interval".into());
                }
            }
            for other in &events[..i] {
                if e.start < other.start.add(&other.duration)
                    && other.start < e.start.add(&e.duration)
                    && e.wires.iter().any(|w| other.wires.contains(w))
                {
                    return Err("§11: overlapping dependent wire operations".into());
                }
            }
            for (name, value) in &e.cost {
                nonnegative(value.clone())?;
                let spec = resources
                    .get(name)
                    .ok_or_else(|| format!("§8.2: undeclared resource {name}"))?;
                match string(field(spec, "kind")?)? {
                    "count" => {
                        let v = counts.get_mut(name).unwrap();
                        *v = v.add(value);
                    }
                    "time" => {
                        if value != &e.duration {
                            return Err("§8.2: event time cost differs from duration".into());
                        }
                    }
                    _ => return Err("§8.2: occupancy and peak are derived from liveness".into()),
                }
            }
        }
        let max_parallel = limit(field(
            field(&self.document, "capabilities")?,
            "max_parallel",
        )?)?;
        if let Some(max) = max_parallel {
            for e in events {
                let simultaneous = events
                    .iter()
                    .filter(|g| {
                        !g.wires.is_empty()
                            && g.start <= e.start
                            && g.start.add(&g.duration) > e.start
                    })
                    .count();
                if simultaneous > max {
                    return Err("§8.3: max_parallel exceeded".into());
                }
            }
        }
        for (i, l) in live.iter().enumerate() {
            nonnegative(l.start.clone())?;
            if l.end < l.start {
                return Err("§8.2: reversed live interval".into());
            }
            if !self.classes.contains_key(&l.class) {
                return Err("§8.2: unknown live class".into());
            }
            end = end.max(l.end.clone());
            for other in &live[..i] {
                if other.wire == l.wire && l.start < other.end && other.start < l.end {
                    return Err("§8.2: overlapping wire ownership intervals; borrowing replaces owner charge".into());
                }
            }
        }
        let mut peak = BTreeMap::new();
        for (c, class) in &self.classes {
            let mut changes = BTreeMap::<R, i64>::new();
            for l in live.iter().filter(|l| &l.class == c) {
                *changes.entry(l.start.clone()).or_default() += 1;
                *changes.entry(l.end.clone()).or_default() -= 1;
            }
            let mut now = 0;
            let mut max = 0;
            for delta in changes.values() {
                now += delta;
                max = max.max(now);
            }
            if class.capacity.is_some_and(|n| max as usize > n) {
                return Err(format!("§8.4: peak capacity exceeded for {c}: {max}"));
            }
            peak.insert(c.clone(), Json::Str(max.to_string()));
        }
        let mut result = counts
            .into_iter()
            .map(|(k, v)| (k, Json::Str(v.decimal())))
            .collect::<BTreeMap<_, _>>();
        for (name, spec) in resources {
            match string(field(spec, "kind")?)? {
                "time" => {
                    result.insert(name.clone(), Json::Str(end.decimal()));
                }
                "peak" => {
                    result.insert(name.clone(), Json::Obj(peak.clone()));
                }
                "occupancy" => {
                    let mut cost = self
                        .classes
                        .keys()
                        .map(|c| (c.clone(), R::zero()))
                        .collect::<BTreeMap<_, _>>();
                    for l in live {
                        let rate = if l.dirty {
                            self.classes[&l.class].dirty_rate.clone()
                        } else {
                            spec.get("rate")
                                .and_then(|m| m.get(&l.class))
                                .map(number)
                                .transpose()?
                                .unwrap_or(R::from(1))
                        };
                        let c = cost.get_mut(&l.class).unwrap();
                        *c = c.add(&l.end.sub(&l.start).mul(&rate));
                    }
                    result.insert(
                        name.clone(),
                        Json::Obj(
                            cost.into_iter()
                                .map(|(c, r)| (c, Json::Str(r.decimal())))
                                .collect(),
                        ),
                    );
                }
                _ => {}
            }
        }
        Ok(Json::Obj(result))
    }
    /// VM diagnostic event input, not an extension of the §8 target schema.
    pub fn account_json(&self, j: &Json) -> Result<Json, String> {
        let mut events = vec![];
        for e in arr(field(j, "leaves")?)? {
            events.push(Event {
                start: decimal(field(e, "start")?)?,
                duration: decimal(field(e, "duration")?)?,
                wires: e
                    .get("wires")
                    .map(arr)
                    .transpose()?
                    .map(|a| {
                        a.iter()
                            .map(|w| decimal(w)?.usize())
                            .collect::<Result<Vec<_>, String>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
                cost: obj(field(e, "cost")?)?
                    .iter()
                    .map(|(n, v)| Ok((n.clone(), decimal(v)?)))
                    .collect::<Result<_, String>>()?,
            });
        }
        let mut live = vec![];
        for l in arr(field(j, "live_intervals")?)? {
            live.push(LiveInterval {
                wire: decimal(field(l, "wire")?)?.usize()?,
                class: string(field(l, "class")?)?.into(),
                start: decimal(field(l, "start")?)?,
                end: decimal(field(l, "end")?)?,
                dirty: l.get("dirty").map(boolean).transpose()?.unwrap_or(false),
            });
        }
        self.account(&events, &live)
    }
}

impl Target {
    /// Guard paths whose gate-level ordinals/scratch require the absent §11 library.
    /// Gate-only actions use the published §4.4 matrices after actual target expansion.
    pub fn validate_object(&self, object: &crate::object::Object) -> Result<(), String> {
        if !self.supports_ptr(object.ptr_width) {
            return Err("§8.3: target does not support object ptr_width".into());
        }
        for f in &object.functions {
            if !matches!(
                f.kind,
                crate::object::FnKind::Action
                    | crate::object::FnKind::PermAction
                    | crate::object::FnKind::Oracle
                    | crate::object::FnKind::Lproc
            ) {
                continue;
            }
            let mut instructions = vec![];
            f.flat_instrs(&mut instructions);
            for ins in instructions {
                if crate::action::is_gate_name(&ins.op)
                    || [
                        "qalloc",
                        "qfree",
                        "qborrow",
                        "qreturn",
                        "action_call",
                        "adj",
                        "ctl",
                        "qpred",
                        "qunpred",
                        "qcopy",
                        "quncopy",
                        "qxor",
                        "qexch",
                        "qif",
                        "qloop",
                        "qregion",
                        "qregion_release",
                    ]
                    .contains(&ins.op.as_str())
                {
                    continue;
                }
                return Err(format!(
                    "§8/§11: unimplemented: gate-level expansion for {} (milestone 4); required ref library is unpublished (bennett-vm-47t)",
                    ins.op
                ));
            }
        }
        Ok(())
    }
}
