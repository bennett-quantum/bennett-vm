//! §9.3 admission of externally resolved source. Certificate composition and
//! realised schedule execution remain separate consumers of this admitted graph.
use crate::{identity::Identities, json::Json, object::Object, validate::err};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug)]
pub struct Admission {
    pub identities: Identities,
    /// Exact resolved closures for recording in the linked certificate.
    pub resolved_imports: BTreeMap<String, String>,
}
fn merge(into: &mut Json, other: &Json, path: &str) -> Result<(), String> {
    match (into, other) {
        (Json::Obj(a), Json::Obj(b)) => {
            for (k, v) in b {
                if let Some(old) = a.get_mut(k) {
                    merge(old, v, &format!("{path}.{k}"))?;
                } else {
                    a.insert(k.clone(), v.clone());
                }
            }
            Ok(())
        }
        (a, b) if a == b => Ok(()),
        _ => Err(err("9.3", format!("conflicting {path} entries"))),
    }
}
fn merged_metadata(inputs: &[Object], layout: bool) -> Result<Option<Json>, String> {
    let mut value = None;
    let name = if layout { "layout" } else { "devices" };
    for obj in inputs {
        let next = if layout {
            obj.layout.as_ref().map(|v| &v.json)
        } else {
            obj.devices.as_ref()
        };
        if let Some(next) = next {
            if let Some(value) = &mut value {
                merge(value, next, name)?;
            } else {
                value = Some(next.clone());
            }
        }
    }
    Ok(value)
}
/// Verify source provenance, exact import signatures/pins and metadata merging,
/// then repeat normal admission on the complete resolved graph. No supplied
/// Object fields, cached index widths, or producer identity claims are trusted.
/// This function neither composes nor verifies a certificate: the returned
/// resolution ledger is input to that later stage of §9.3/§12.3.
pub fn admit(sources: &[&str], linked: &str) -> Result<Admission, String> {
    if sources.is_empty() {
        return Err(err("9.3", "no input objects"));
    }
    let mut inputs = vec![];
    let mut input_ids = vec![];
    for text in sources {
        let (obj, ids) = crate::load(text, false)?;
        inputs.push(obj);
        input_ids.push(ids);
    }
    // Parse before validation so conflicting metadata and source provenance are
    // diagnosed as linking failures, independent of row-space admission order.
    let candidate = crate::object::parse(linked)?;
    if !candidate.imports.is_empty() {
        return Err(err("9.3", "unresolved linked imports"));
    }
    for input in &inputs {
        if input.isa != candidate.isa || input.ptr_width != candidate.ptr_width {
            return Err(err("9.3", "input version/pointer width mismatch"));
        }
    }
    if candidate.layout.as_ref().map(|v| &v.json) != merged_metadata(&inputs, true)?.as_ref() {
        return Err(err("9.3", "linked layout is not the exact input merge"));
    }
    if candidate.devices.as_ref() != merged_metadata(&inputs, false)?.as_ref() {
        return Err(err("9.3", "linked devices are not the exact input merge"));
    }
    let mut names = BTreeSet::new();
    let mut data = BTreeMap::new();
    let mut original_closures = BTreeMap::new();
    for (input, ids) in inputs.iter().zip(&input_ids) {
        for (name, symbol) in &input.symbols {
            if let Some(hash) = &ids.closure[name]
                && let Some(previous) = original_closures.insert(name.clone(), hash.clone())
                && previous != *hash
            {
                return Err(err("9.3", format!("unequal duplicate closure @{name}")));
            }
            names.insert(name.clone());
            let actual = candidate
                .symbols
                .get(name)
                .ok_or_else(|| err("9.3", format!("missing definition @{name}")))?;
            if symbol.kind != actual.kind || symbol.signature != actual.signature {
                return Err(err("9.3", format!("definition signature mismatch @{name}")));
            }
            if let Some(f) = input.function(name)
                && candidate
                    .function(name)
                    .is_none_or(|actual| actual.canonical != f.canonical)
            {
                return Err(err("9.3", format!("rewritten source definition @{name}")));
            }
        }
        for (name, bytes) in &input.data {
            if let Some(old) = data.insert(name.clone(), bytes.clone())
                && old != *bytes
            {
                return Err(err("9.3", format!("conflicting data entry @{name}")));
            }
        }
        for (name, import) in &input.imports {
            let actual = candidate
                .symbols
                .get(name)
                .ok_or_else(|| err("9.3", format!("unresolved import @{name}")))?;
            if actual.signature != import.signature {
                return Err(err("9.3", format!("import signature mismatch @{name}")));
            }
        }
    }
    if names != candidate.symbols.keys().cloned().collect() || data != candidate.data {
        return Err(err(
            "9.3",
            "linked source contains extra or changed definitions/data",
        ));
    }
    // Repeats all five lproc-group checks, fixed-instance formation and the
    // complete per-oracle row-space/fault-width checks after import resolution.
    let (_, identities) = crate::load(linked, false)?;
    let mut resolved_imports = BTreeMap::new();
    for input in &inputs {
        for (name, import) in &input.imports {
            let closure = identities.closure[name]
                .as_ref()
                .ok_or_else(|| err("9.3", format!("unresolved closure @{name}")))?;
            if import
                .pinned_closure_hash
                .as_ref()
                .is_some_and(|p| p != closure)
            {
                return Err(err("9.3", format!("pinned closure mismatch @{name}")));
            }
            resolved_imports.insert(name.clone(), closure.clone());
        }
    }
    Ok(Admission {
        identities,
        resolved_imports,
    })
}

/// Source resolution result. The original objects retain schedules, certificates
/// and coverage for the composition/rebinding stage; `object` is the admitted
/// source graph, not a claim that those documents certify its new identities.
#[derive(Debug)]
pub struct Resolution {
    pub object: Object,
    pub admission: Admission,
    pub inputs: Vec<Object>,
}

/// Resolve every import against the supplied source objects and automatically
/// assemble a deterministic canonical graph. Re-indexing symbols does not
/// rewrite function bytes or invent invocation/certificate bindings.
pub fn resolve(sources: &[&str]) -> Result<Resolution, String> {
    if sources.is_empty() {
        return Err(err("9.3", "no input objects"));
    }
    let inputs = sources
        .iter()
        .map(|s| crate::load(s, false).map(|(o, _)| o))
        .collect::<Result<Vec<_>, _>>()?;
    let first = &inputs[0];
    let mut definitions = BTreeMap::new();
    let mut functions = BTreeMap::new();
    let mut data = BTreeMap::new();
    for input in &inputs {
        if input.isa != first.isa || input.ptr_width != first.ptr_width {
            return Err(err("9.3", "input version/pointer width mismatch"));
        }
        for (name, symbol) in &input.symbols {
            if let Some(old) = definitions.insert(name.clone(), symbol.signature.clone())
                && old != symbol.signature
            {
                return Err(err("9.3", format!("unequal duplicate definition @{name}")));
            }
        }
        for f in &input.functions {
            if let Some(old) = functions.insert(f.name.clone(), f.canonical.clone())
                && old != f.canonical
            {
                return Err(err(
                    "9.3",
                    format!("unequal duplicate definition @{}", f.name),
                ));
            }
        }
        for (name, bytes) in &input.data {
            if let Some(old) = data.insert(name.clone(), bytes.clone())
                && old != *bytes
            {
                return Err(err("9.3", format!("conflicting data entry @{name}")));
            }
        }
    }
    for input in &inputs {
        for (name, import) in &input.imports {
            let signature = definitions
                .get(name)
                .ok_or_else(|| err("9.3", format!("unresolved import @{name}")))?;
            if signature != &import.signature {
                return Err(err("9.3", format!("import signature mismatch @{name}")));
            }
        }
    }
    let symbols = Json::Obj(
        definitions
            .iter()
            .enumerate()
            .map(|(index, (name, sig))| {
                (
                    index.to_string(),
                    Json::obj(vec![
                        ("name", Json::Str(name.clone())),
                        ("kind", sig.get("kind").unwrap().clone()),
                        ("signature", sig.clone()),
                        ("interface_hash", Json::Str("0".repeat(64))),
                    ]),
                )
            })
            .collect(),
    );
    let mut text = format!(
        "{}\n\nsection symbols\n{}\n",
        first.text.lines().next().unwrap(),
        symbols.to_jcs()
    );
    if !functions.is_empty() {
        text.push_str(&format!(
            "\nsection functions\n{}\n",
            functions.values().cloned().collect::<Vec<_>>().join("\n")
        ));
    }
    if !data.is_empty() {
        text.push_str("\nsection data\n");
        for (name, bytes) in &data {
            text.push_str(&format!("{name} {bytes}\n"));
        }
    }
    for (name, value) in [
        ("devices", merged_metadata(&inputs, false)?),
        ("layout", merged_metadata(&inputs, true)?),
    ] {
        if let Some(value) = value {
            text.push_str(&format!("\nsection {name}\n{}\n", value.to_jcs()));
        }
    }
    // Identities change transitively when an unresolved reference is resolved.
    // Compute them from canonical bytes, then re-admit with the stored hashes.
    let (object, ids) = crate::load(&text, true)?;
    let mut symbols = symbols;
    if let Json::Obj(entries) = &mut symbols {
        for (index, entry) in entries {
            let name = object
                .symbols
                .values()
                .find(|s| s.index.to_string() == *index)
                .unwrap()
                .name
                .clone();
            if let Json::Obj(fields) = entry {
                fields.insert(
                    "interface_hash".into(),
                    Json::Str(ids.interface[&name].clone()),
                );
            }
        }
    }
    let start = text.find("section symbols\n").unwrap() + "section symbols\n".len();
    let end = start + text[start..].find('\n').unwrap();
    text.replace_range(start..end, &symbols.to_jcs());
    let admission = admit(sources, &text)?;
    let (object, _) = crate::load(&text, false)?;
    Ok(Resolution {
        object,
        admission,
        inputs,
    })
}
