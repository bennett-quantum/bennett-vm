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
