//! §9.2 identity: semantic envelopes, SCC hashes, object hash.

use crate::json::Json;
use crate::object::{Function, Object};
use crate::sha256::sha256_hex;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub struct Identities {
    pub object_hash: String,
    pub interface: BTreeMap<String, String>,
    pub closure: BTreeMap<String, Option<String>>,
    /// Closure-kind SCC hash per member (its own closure hash for a
    /// trivial SCC); used for the §3.6 lproc ordering.
    pub scc_hashes: BTreeMap<String, Option<String>>,
}

impl Identities {
    pub fn scc_hash_of(&self, name: &str) -> String {
        self.scc_hashes
            .get(name)
            .cloned()
            .flatten()
            .or_else(|| self.closure.get(name).cloned().flatten())
            .unwrap_or_default()
    }
}

/// `@name` symbol-reference tokens of a function, in first-reference
/// order. The header definition and the containing oracle's interface
/// region names are not references (§9.2).
pub fn references_of(obj: &Object, f: &Function) -> Vec<String> {
    let region_names: BTreeSet<&str> = if f.kind == crate::object::FnKind::Oracle {
        f.lists[3].iter().map(|p| p.name.as_str()).collect()
    } else {
        BTreeSet::new()
    };
    let mut out: Vec<String> = Vec::new();
    let mut first = true;
    for tok in scan_refs(&f.canonical) {
        if first {
            first = false;
            if tok == f.name {
                continue; // the definition in the header line
            }
        }
        if region_names.contains(tok.as_str()) {
            continue;
        }
        if !obj.symbols.contains_key(&tok) && !obj.imports.contains_key(&tok) {
            continue;
        }
        if !out.contains(&tok) {
            out.push(tok);
        }
    }
    out
}

/// All `@NAME` tokens in text order, skipping string literals.
fn scan_refs(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            i += 1;
            continue;
        }
        if c == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'.') {
                j += 1;
            }
            if j > start && (b[start].is_ascii_alphabetic() || b[start] == b'_') {
                out.push(text[start..j].to_string());
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

/// Replace `@name` reference tokens in canonical function bytes using
/// `map`; the header definition token is left unchanged.
fn replace_refs(f: &Function, map: &dyn Fn(&str) -> Option<String>) -> String {
    let b = f.canonical.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut in_str = false;
    let mut first = true;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c as char);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'.') {
                j += 1;
            }
            if j > start && (b[start].is_ascii_alphabetic() || b[start] == b'_') {
                let name = &f.canonical[start..j];
                let is_definition = first && name == f.name;
                first = false;
                match if is_definition { None } else { map(name) } {
                    Some(rep) => {
                        out.push('@');
                        out.push_str(&rep);
                    }
                    None => out.push_str(&f.canonical[i..j]),
                }
                i = j;
                continue;
            }
        }
        // Copy one UTF-8 character.
        let ch_len = utf8_len(b[i]);
        out.push_str(&f.canonical[i..(i + ch_len).min(b.len())]);
        i += ch_len;
    }
    out
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// Layout leaves of §9.2 as (dotted path, JCS value text), path-sorted.
pub fn layout_leaves(obj: &Object) -> Vec<(String, String)> {
    let mut leaves: Vec<(String, String)> = Vec::new();
    let Some(layout) = &obj.layout else {
        return leaves;
    };
    let j = &layout.json;
    for key in [
        "ptr_width",
        "H",
        "A_tab",
        "S_cells",
        "null_size",
        "heap_base",
        "heap_limit_max",
    ] {
        if let Some(v) = j.get(key) {
            leaves.push((key.to_string(), v.to_jcs()));
        }
    }
    if let Some(g) = j.get("globals").and_then(|v| v.as_obj()) {
        for (name, v) in g {
            leaves.push((format!("globals.{name}"), v.to_jcs()));
        }
    }
    if let Some(ir) = j.get("interface_regions").and_then(|v| v.as_obj()) {
        for (oracle, regions) in ir {
            if let Some(regions) = regions.as_obj() {
                for (region, v) in regions {
                    leaves.push((format!("interface_regions.{oracle}.{region}"), v.to_jcs()));
                }
            }
        }
    }
    if let Some(v) = j.get("block_image") {
        leaves.push(("block_image".to_string(), v.to_jcs()));
    }
    leaves.sort();
    leaves
}

/// Closure set of §9.2 for one symbol: (layout leaves, data names).
fn closure_set(obj: &Object, name: &str) -> (Vec<(String, String)>, Vec<String>) {
    let Some(sym) = obj.symbols.get(name) else {
        return (Vec::new(), Vec::new());
    };
    let all = layout_leaves(obj);
    let leaves: Vec<(String, String)> = match sym.kind.as_str() {
        "oracle" => all
            .into_iter()
            .filter(|(path, _)| match path.strip_prefix("interface_regions.") {
                Some(rest) => rest.split('.').next() == Some(name),
                None => true,
            })
            .collect(),
        "cc" | "r" => all,
        "lproc" => {
            let mut wanted: BTreeSet<String> = BTreeSet::new();
            if let Some(f) = obj.function(name) {
                let mut flat = Vec::new();
                f.flat_instrs(&mut flat);
                for ins in flat {
                    if ins.op == "lift.addr"
                        && let Some(tok) = ins.operands.first()
                        && let Some(g) = tok.strip_prefix('@')
                        && obj
                            .layout
                            .as_ref()
                            .is_some_and(|l| l.globals.contains_key(g))
                    {
                        wanted.insert(format!("globals.{g}"));
                    }
                }
            }
            all.into_iter()
                .filter(|(p, _)| wanted.contains(p))
                .collect()
        }
        _ => Vec::new(),
    };
    // Every data image those leaves name.
    let mut data: BTreeSet<String> = BTreeSet::new();
    if let Some(layout) = &obj.layout {
        for (path, _) in &leaves {
            if let Some(gname) = path.strip_prefix("globals.") {
                if let Some(g) = layout.globals.get(gname) {
                    data.insert(g.image.clone());
                }
            } else if path == "block_image"
                && let Some(img) = &layout.block_image
            {
                data.insert(img.clone());
            }
        }
    }
    (leaves, data.into_iter().collect())
}

/// Part (5c) device lines for a CC/R function.
fn device_lines(obj: &Object, f: &Function) -> Vec<String> {
    let mut flat = Vec::new();
    f.flat_instrs(&mut flat);
    let mut used: BTreeMap<String, String> = BTreeMap::new();
    let devices = obj.devices.as_ref();
    let entry = |kind: &str, name: &str| -> Option<String> {
        devices
            .and_then(|d| d.get(kind))
            .and_then(|m| m.get(name))
            .map(|v| v.to_jcs())
    };
    for ins in flat {
        if ins.op == "effect" {
            if let Some(dev_op) = ins.operands.first() {
                let dev = dev_op.split('.').next().unwrap_or("");
                if let Some(e) = entry("models", dev) {
                    used.insert(dev.to_string(), e);
                }
            }
        } else if ins.op == "hostcall"
            && let Some(service) = ins.operands.first()
            && let Some(e) = entry("services", service)
        {
            used.insert(service.clone(), e);
        }
    }
    used.into_iter()
        .map(|(name, e)| format!("device {name} {e}"))
        .collect()
}

/// Tarjan SCCs over defined symbols. Each SCC is name-sorted.
fn sccs(obj: &Object, refs: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    struct State {
        index: HashMap<String, usize>,
        low: HashMap<String, usize>,
        on_stack: BTreeSet<String>,
        stack: Vec<String>,
        next: usize,
        out: Vec<Vec<String>>,
    }
    fn strongconnect(
        v: &str,
        refs: &HashMap<String, Vec<String>>,
        defined: &BTreeSet<String>,
        st: &mut State,
    ) {
        st.index.insert(v.to_string(), st.next);
        st.low.insert(v.to_string(), st.next);
        st.next += 1;
        st.stack.push(v.to_string());
        st.on_stack.insert(v.to_string());
        let succ: Vec<String> = refs
            .get(v)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|r| defined.contains(r))
            .collect();
        for w in succ {
            if !st.index.contains_key(&w) {
                strongconnect(&w, refs, defined, st);
                let lw = st.low[&w];
                let lv = st.low[v];
                st.low.insert(v.to_string(), lv.min(lw));
            } else if st.on_stack.contains(&w) {
                let iw = st.index[&w];
                let lv = st.low[v];
                st.low.insert(v.to_string(), lv.min(iw));
            }
        }
        if st.low[v] == st.index[v] {
            let mut comp = Vec::new();
            loop {
                let w = st.stack.pop().unwrap();
                st.on_stack.remove(&w);
                let done = w == v;
                comp.push(w);
                if done {
                    break;
                }
            }
            comp.sort();
            st.out.push(comp);
        }
    }
    let defined: BTreeSet<String> = obj.symbols.keys().cloned().collect();
    let mut st = State {
        index: HashMap::new(),
        low: HashMap::new(),
        on_stack: BTreeSet::new(),
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for name in defined.iter() {
        if !st.index.contains_key(name) {
            strongconnect(name, refs, &defined, &mut st);
        }
    }
    // Tarjan emits sink SCCs first: dependencies before dependents.
    st.out
}

pub fn import_interface_hash(name: &str, signature: &Json) -> String {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend_from_slice(signature.to_jcs().as_bytes());
    sha256_hex(&bytes)
}

/// Compute both hash kinds. Returns an error the first time a closure
/// set names data the object does not define (validation reports it
/// too; hashing must never invent bytes).
pub fn compute(obj: &Object) -> Result<Identities, String> {
    let mut refs: HashMap<String, Vec<String>> = HashMap::new();
    for name in obj.symbols.keys() {
        let r = match obj.function(name) {
            Some(f) => references_of(obj, f),
            None => Vec::new(),
        };
        refs.insert(name.clone(), r);
    }
    let comps = sccs(obj, &refs);
    let (interface, _) = compute_kind(obj, &refs, &comps, true)?;
    let (closure_opts, scc_hashes) = compute_kind(obj, &refs, &comps, false)?;
    let interface: BTreeMap<String, String> = interface
        .into_iter()
        .map(|(k, v)| (k, v.expect("interface hash always exists")))
        .collect();
    Ok(Identities {
        object_hash: object_hash(&obj.text),
        interface,
        closure: closure_opts,
        scc_hashes,
    })
}

type HashMapByName = BTreeMap<String, Option<String>>;

fn compute_kind(
    obj: &Object,
    refs: &HashMap<String, Vec<String>>,
    comps: &[Vec<String>],
    interface: bool,
) -> Result<(HashMapByName, HashMapByName), String> {
    let mut hashes: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut scc_hashes: BTreeMap<String, Option<String>> = BTreeMap::new();
    for comp in comps {
        let in_comp: BTreeMap<&str, usize> = comp
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let self_ref = comp.len() == 1 && refs.get(&comp[0]).is_some_and(|r| r.contains(&comp[0]));
        let mut envelopes: Vec<Option<Vec<u8>>> = Vec::new();
        for member in comp {
            envelopes.push(build_envelope(
                obj, member, refs, &in_comp, &hashes, interface,
            )?);
        }
        if comp.len() == 1 && !self_ref {
            let member = &comp[0];
            let hash = envelopes[0].as_ref().map(|e| sha256_hex(e));
            scc_hashes.insert(member.clone(), hash.clone());
            hashes.insert(member.clone(), hash);
        } else {
            let scc_hash = if envelopes.iter().all(|e| e.is_some()) {
                let mut concat: Vec<u8> = Vec::new();
                for e in &envelopes {
                    concat.extend_from_slice(e.as_ref().unwrap());
                }
                Some(sha256_hex(&concat))
            } else {
                None
            };
            for (i, member) in comp.iter().enumerate() {
                scc_hashes.insert(member.clone(), scc_hash.clone());
                let hash = scc_hash
                    .as_ref()
                    .map(|h| sha256_hex(format!("{h}:{i}").as_bytes()));
                hashes.insert(member.clone(), hash);
            }
        }
    }
    Ok((hashes, scc_hashes))
}

fn build_envelope(
    obj: &Object,
    member: &str,
    refs: &HashMap<String, Vec<String>>,
    in_comp: &BTreeMap<&str, usize>,
    hashes: &BTreeMap<String, Option<String>>,
    interface: bool,
) -> Result<Option<Vec<u8>>, String> {
    let sym = obj.symbols.get(member).expect("symbol exists");
    // Hash for a reference outside this SCC; None => unknowable (closure).
    let resolve = |name: &str| -> Option<String> {
        if let Some(idx) = in_comp.get(name) {
            return Some(format!("${idx}"));
        }
        if obj.symbols.contains_key(name) {
            return hashes.get(name).cloned().flatten();
        }
        if let Some(imp) = obj.imports.get(name) {
            return if interface {
                Some(import_interface_hash(name, &imp.signature))
            } else {
                imp.pinned_closure_hash.clone()
            };
        }
        None
    };
    let member_refs = refs.get(member).cloned().unwrap_or_default();
    let mut part6: Vec<String> = Vec::new();
    for r in &member_refs {
        if in_comp.contains_key(r.as_str()) {
            continue;
        }
        match resolve(r) {
            Some(h) => part6.push(h),
            None => return Ok(None),
        }
    }
    let mut env = String::new();
    env.push_str("isa=0.8\n");
    env.push_str(&format!("ptr_width={}\n", obj.ptr_width));
    env.push_str(&sym.signature.to_jcs());
    env.push('\n');
    if sym.kind == "data" {
        let hex = obj
            .data
            .get(member)
            .ok_or_else(|| format!("§9.1: data symbol @{member} has no data line"))?;
        env.push_str(&format!("{member} {hex}\n"));
        return Ok(Some(env.into_bytes()));
    }
    let f = obj
        .function(member)
        .ok_or_else(|| format!("§9.1: symbol @{member} has no function definition"))?;
    let replaced = replace_refs(f, &|name: &str| {
        if in_comp.contains_key(name)
            || obj.symbols.contains_key(name)
            || obj.imports.contains_key(name)
        {
            resolve(name)
        } else {
            None
        }
    });
    env.push_str(&replaced);
    env.push('\n');
    let (leaves, data) = closure_set(obj, member);
    for d in &data {
        let hex = obj
            .data
            .get(d)
            .ok_or_else(|| format!("§9.2: closure set of @{member} names undefined data {d:?}"))?;
        env.push_str(&format!("data {d} {hex}\n"));
    }
    for (path, value) in &leaves {
        env.push_str(&format!("layout {path} {value}\n"));
    }
    if sym.kind == "cc" || sym.kind == "r" {
        for line in device_lines(obj, f) {
            env.push_str(&line);
            env.push('\n');
        }
    }
    for h in &part6 {
        env.push_str(h);
        env.push('\n');
    }
    Ok(Some(env.into_bytes()))
}

/// §9.2: SHA-256 of the object text minus the schedule, certificate and
/// coverage sections, each with its preceding blank line.
pub fn object_hash(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut headers: Vec<(usize, &str)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = line.strip_prefix("section ") {
            headers.push((i, name));
        }
    }
    let mut drop_lines: BTreeSet<usize> = BTreeSet::new();
    for (k, (hdr, name)) in headers.iter().enumerate() {
        if ["schedule", "certificate", "coverage"].contains(name) {
            let start = hdr.saturating_sub(1);
            let end = match headers.get(k + 1) {
                Some((next_hdr, _)) => next_hdr - 1,
                None => lines.len(),
            };
            for i in start..end {
                drop_lines.insert(i);
            }
        }
    }
    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop_lines.contains(i))
        .map(|(_, l)| *l)
        .collect();
    let mut bytes = kept.join("\n");
    if !kept.is_empty() {
        bytes.push('\n');
    }
    sha256_hex(bytes.as_bytes())
}

pub fn check_hashes(obj: &Object, ids: &Identities) -> Result<(), String> {
    // Report the first mismatch in local-index order.
    let mut syms: Vec<&crate::object::Symbol> = obj.symbols.values().collect();
    syms.sort_by_key(|s| s.index);
    for sym in syms {
        let computed = &ids.interface[&sym.name];
        if *computed != sym.stored_hash {
            return Err(format!(
                "§9.2: interface_hash mismatch for @{}: expected {}",
                sym.name, computed
            ));
        }
    }
    Ok(())
}

/// `check`/`hash` JSON output object.
pub fn check_json(obj: &Object, ids: &Identities) -> Json {
    let mut symbols: Vec<(&str, Json)> = Vec::new();
    for (name, iface) in &ids.interface {
        let closure = match &ids.closure[name] {
            Some(h) => Json::Str(h.clone()),
            None => Json::Null,
        };
        symbols.push((
            name.as_str(),
            Json::obj(vec![
                ("closure_hash", closure),
                ("interface_hash", Json::Str(iface.clone())),
            ]),
        ));
    }
    let unresolved: Vec<Json> = obj.imports.keys().map(|n| Json::Str(n.clone())).collect();
    let status = if obj.imports.is_empty() {
        "ok"
    } else {
        "unlinked"
    };
    let symbols_obj = {
        let mut m = BTreeMap::new();
        for (k, v) in symbols {
            m.insert(k.to_string(), v);
        }
        Json::Obj(m)
    };
    Json::obj(vec![
        ("object_hash", Json::Str(ids.object_hash.clone())),
        ("status", Json::Str(status.to_string())),
        ("symbols", symbols_obj),
        ("unresolved_imports", Json::Arr(unresolved)),
    ])
}
