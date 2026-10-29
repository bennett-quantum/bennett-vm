//! Well-formedness validation. Every rejection cites its rule; nothing
//! is silently repaired.

use crate::bigint::BigUint;
use crate::json::Json;
use crate::object::{FnKind, Function, Object, Param, Type};
use std::collections::{BTreeMap, BTreeSet};

pub fn err(rule: &str, msg: impl std::fmt::Display) -> String {
    format!("§{rule}: {msg}")
}

const KINDS: [&str; 7] = [
    "cc",
    "r",
    "oracle",
    "action",
    "perm_action",
    "lproc",
    "data",
];

pub fn validate(obj: &Object) -> Result<(), String> {
    validate_symbols(obj)?;
    validate_functions_match(obj)?;
    validate_layout(obj)?;
    validate_devices(obj)?;
    validate_memoryless(obj)?;
    for f in &obj.functions {
        validate_function(obj, f)?;
        crate::typing::classical(obj, f)?;
        crate::qvalidate::validate(obj, f)?;
    }
    validate_action_acyclic(obj)?;
    Ok(())
}

fn validate_symbols(obj: &Object) -> Result<(), String> {
    let mut indices: BTreeSet<usize> = BTreeSet::new();
    for sym in obj.symbols.values() {
        if !indices.insert(sym.index) {
            return Err(err("9.1", format!("duplicate symbol index {}", sym.index)));
        }
        if !KINDS.contains(&sym.kind.as_str()) {
            return Err(err(
                "9.1",
                format!("unknown symbol kind {:?} for @{}", sym.kind, sym.name),
            ));
        }
        let h = &sym.stored_hash;
        if h.len() != 64
            || !h
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(err(
                "9.2",
                format!(
                    "interface_hash for @{} is not 64 lowercase hex digits",
                    sym.name
                ),
            ));
        }
        validate_signature_schema(&sym.signature, &sym.kind)?;
        if obj.imports.contains_key(&sym.name) {
            return Err(err(
                "9.3",
                format!("@{} is both defined and imported", sym.name),
            ));
        }
    }
    for (name, imp) in &obj.imports {
        let kind = imp
            .signature
            .get("kind")
            .and_then(|k| k.as_str())
            .ok_or_else(|| err("9.1", format!("import {name:?} signature needs a kind")))?;
        if !KINDS.contains(&kind) {
            return Err(err(
                "9.1",
                format!("unknown kind {kind:?} in import {name:?} signature"),
            ));
        }
        validate_signature_schema(&imp.signature, kind)?;
        if let Some(p) = &imp.pinned_closure_hash
            && (p.len() != 64
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()))
        {
            return Err(err(
                "9.3",
                format!("pinned_closure_hash for import {name:?} is not 64 lowercase hex digits"),
            ));
        }
    }
    if let Some(layout) = &obj.layout
        && layout.ptr_width != obj.ptr_width
    {
        return Err(err(
            "9.1",
            format!(
                "layout ptr_width {} disagrees with header ptr_width {}",
                layout.ptr_width, obj.ptr_width
            ),
        ));
    }
    if obj.ptr_width == 0 || obj.ptr_width > 128 {
        return Err(err(
            "3.3",
            format!("ptr_width {} out of range", obj.ptr_width),
        ));
    }
    Ok(())
}

fn sig_fields(sig: &Json, want: &[&str], kind: &str) -> Result<(), String> {
    let m = sig.as_obj().ok_or_else(|| {
        err(
            "9.1",
            format!("signature for kind {kind} must be an object"),
        )
    })?;
    for k in m.keys() {
        if !want.contains(&k.as_str()) {
            return Err(err(
                "9.1",
                format!("unknown field {k:?} in {kind} signature"),
            ));
        }
    }
    for k in want {
        if !m.contains_key(*k) {
            return Err(err(
                "9.1",
                format!("{kind} signature is missing field {k:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_signature_schema(sig: &Json, kind: &str) -> Result<(), String> {
    match kind {
        "oracle" => {
            sig_fields(
                sig,
                &["tier", "kind", "in", "tgt", "fault", "mem", "args", "abi"],
                kind,
            )?;
            expect(sig, "tier", "q")?;
            expect(sig, "abi", "0.8")?;
            widths(sig, "in")?;
            sig.get("tgt")
                .and_then(|v| v.as_usize())
                .ok_or_else(|| err("9.1", "oracle tgt must be a width"))?;
            let f = sig
                .get("fault")
                .and_then(|v| v.as_usize())
                .ok_or_else(|| err("9.1", "oracle fault must be a width"))?;
            if f < 4 {
                return Err(err("5.4", format!("fault width F={f} is below 4")));
            }
            mem_entries(sig)?;
            type_strings(sig, "args")?;
        }
        "action" | "perm_action" => {
            sig_fields(
                sig,
                &["tier", "kind", "mut", "in", "mem", "args", "bounds", "abi"],
                kind,
            )?;
            expect(sig, "tier", "q")?;
            expect(sig, "abi", "0.8")?;
            widths(sig, "mut")?;
            widths(sig, "in")?;
            mem_entries(sig)?;
            type_strings(sig, "args")?;
            widths(sig, "bounds")?;
        }
        "lproc" => {
            sig_fields(sig, &["tier", "kind", "params", "result", "abi"], kind)?;
            expect(sig, "tier", "q")?;
            expect(sig, "abi", "0.8")?;
            widths(sig, "params")?;
            sig.get("result")
                .and_then(|v| v.as_usize())
                .ok_or_else(|| err("9.1", "lproc result must be a width"))?;
        }
        "cc" | "r" => {
            sig_fields(sig, &["tier", "kind", "params", "results", "abi"], kind)?;
            expect(sig, "tier", kind)?;
            expect(sig, "abi", "0.8")?;
            type_strings(sig, "params")?;
            type_strings(sig, "results")?;
        }
        "data" => {
            sig_fields(sig, &["tier", "kind", "size", "abi"], kind)?;
            expect(sig, "tier", "-")?;
            expect(sig, "abi", "0.8")?;
            sig.get("size")
                .and_then(|v| v.as_big())
                .ok_or_else(|| err("9.1", "data size must be an integer"))?;
        }
        _ => return Err(err("9.1", format!("unknown kind {kind:?}"))),
    }
    if sig.get("kind").and_then(|k| k.as_str()) != Some(kind) {
        return Err(err(
            "9.1",
            format!("signature kind disagrees with symbol kind {kind:?}"),
        ));
    }
    Ok(())
}

fn expect(sig: &Json, key: &str, want: &str) -> Result<(), String> {
    if sig.get(key).and_then(|v| v.as_str()) == Some(want) {
        Ok(())
    } else {
        Err(err(
            "9.1",
            format!("signature field {key:?} must be {want:?}"),
        ))
    }
}

fn widths(sig: &Json, key: &str) -> Result<Vec<usize>, String> {
    let arr = sig.get(key).and_then(|v| v.as_arr()).ok_or_else(|| {
        err(
            "9.1",
            format!("signature field {key:?} must be an array of widths"),
        )
    })?;
    arr.iter()
        .map(|v| {
            v.as_usize()
                .filter(|w| *w > 0 || key == "bounds")
                .ok_or_else(|| {
                    err(
                        "9.1",
                        format!("width in {key:?} must be a positive integer"),
                    )
                })
        })
        .collect()
}

fn type_strings(sig: &Json, key: &str) -> Result<Vec<Type>, String> {
    let arr = sig.get(key).and_then(|v| v.as_arr()).ok_or_else(|| {
        err(
            "9.1",
            format!("signature field {key:?} must be an array of types"),
        )
    })?;
    arr.iter()
        .map(|v| {
            let s = v
                .as_str()
                .ok_or_else(|| err("9.1", format!("type in {key:?} must be a string")))?;
            Type::parse(s).map_err(|e| err("9.1", e))
        })
        .collect()
}

fn mem_entries(sig: &Json) -> Result<Vec<(usize, usize, String)>, String> {
    let arr = sig
        .get("mem")
        .and_then(|v| v.as_arr())
        .ok_or_else(|| err("9.1", "signature mem must be an array"))?;
    let mut out = Vec::new();
    for e in arr {
        let m = e
            .as_obj()
            .ok_or_else(|| err("9.1", "mem entry must be an object"))?;
        for k in m.keys() {
            if !["A", "W", "role"].contains(&k.as_str()) {
                return Err(err("9.1", format!("unknown field {k:?} in mem entry")));
            }
        }
        let a = e
            .get("A")
            .and_then(|v| v.as_usize())
            .ok_or_else(|| err("9.1", "mem A must be an integer"))?;
        let w = e
            .get("W")
            .and_then(|v| v.as_usize())
            .filter(|w| *w > 0)
            .ok_or_else(|| err("9.1", "mem W must be a positive width"))?;
        let role = e
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err("9.1", "mem role must be a string"))?;
        if role != "in" && role != "mut" {
            return Err(err("5.1", format!("mem role {role:?} is not in or mut")));
        }
        out.push((a, w, role.to_string()));
    }
    Ok(out)
}

/// The signature a definition must carry (§9.1 schemas from §16.2 headers).
pub fn derived_signature(obj: &Object, name: &str) -> Result<Json, String> {
    let sym = obj
        .symbols
        .get(name)
        .ok_or_else(|| err("9.1", format!("unknown symbol @{name}")))?;
    if sym.kind == "data" {
        let hex = obj
            .data
            .get(name)
            .ok_or_else(|| err("9.1", format!("data symbol @{name} has no data line")))?;
        return Ok(Json::obj(vec![
            ("abi", Json::Str("0.8".into())),
            ("kind", Json::Str("data".into())),
            ("size", Json::u64((hex.len() / 2) as u64)),
            ("tier", Json::Str("-".into())),
        ]));
    }
    let f = obj
        .function(name)
        .ok_or_else(|| err("9.1", format!("symbol @{name} has no function definition")))?;
    let wlist = |params: &[Param]| -> Result<Vec<Json>, String> {
        params
            .iter()
            .map(|p| match p.ty {
                Type::Wire(n) | Type::Reg(n) => Ok(Json::u64(n as u64)),
                _ => Err(err(
                    "5.1",
                    format!(
                        "parameter {} of @{name} has non-wire type {}",
                        p.name,
                        p.ty.signature_text()
                    ),
                )),
            })
            .collect()
    };
    let args = |params: &[Param]| {
        Json::Arr(
            params
                .iter()
                .map(|p| Json::Str(p.ty.signature_text()))
                .collect(),
        )
    };
    let mem = |params: &[Param]| -> Result<Json, String> {
        let mut out = Vec::new();
        for p in params {
            let (a, w) = match p.ty {
                Type::Region(a, w) => (a, w),
                _ => {
                    return Err(err(
                        "5.1",
                        format!("mem parameter {} of @{name} is not a region", p.name),
                    ));
                }
            };
            let role = p.role.clone().ok_or_else(|| {
                err(
                    "5.1",
                    format!("mem parameter {} of @{name} needs a role", p.name),
                )
            })?;
            out.push(Json::obj(vec![
                ("A", Json::u64(a as u64)),
                ("W", Json::u64(w as u64)),
                ("role", Json::Str(role)),
            ]));
        }
        Ok(Json::Arr(out))
    };
    let one_width = |params: &[Param], what: &str| -> Result<usize, String> {
        if params.len() != 1 {
            return Err(err("5.1", format!("@{name} must have exactly one {what}")));
        }
        match params[0].ty {
            Type::Wire(n) | Type::Reg(n) => Ok(n),
            _ => Err(err("5.1", format!("{what} of @{name} has the wrong type"))),
        }
    };
    let empty_from = |from: usize| -> Result<(), String> {
        for i in from..5 {
            if !f.lists[i].is_empty() {
                return Err(err(
                    "16.2",
                    format!(
                        "header list {} of @{name} must be empty for its kind",
                        i + 1
                    ),
                ));
            }
        }
        Ok(())
    };
    match f.kind {
        FnKind::Oracle => Ok(Json::obj(vec![
            ("abi", Json::Str("0.8".into())),
            ("args", args(&f.lists[4])),
            (
                "fault",
                Json::u64(one_width(&f.lists[2], "fault bundle")? as u64),
            ),
            ("in", Json::Arr(wlist(&f.lists[0])?)),
            ("kind", Json::Str("oracle".into())),
            ("mem", mem(&f.lists[3])?),
            (
                "tgt",
                Json::u64(one_width(&f.lists[1], "target bundle")? as u64),
            ),
            ("tier", Json::Str("q".into())),
        ])),
        FnKind::Action | FnKind::PermAction => {
            empty_from(4)?;
            Ok(Json::obj(vec![
                ("abi", Json::Str("0.8".into())),
                ("args", args(&f.lists[3])),
                (
                    "bounds",
                    Json::Arr(f.bounds.iter().map(|b| Json::u64(*b as u64)).collect()),
                ),
                ("in", Json::Arr(wlist(&f.lists[1])?)),
                ("kind", Json::Str(f.kind.kind_str().into())),
                ("mem", mem(&f.lists[2])?),
                ("mut", Json::Arr(wlist(&f.lists[0])?)),
                ("tier", Json::Str("q".into())),
            ]))
        }
        FnKind::Lproc => {
            empty_from(2)?;
            Ok(Json::obj(vec![
                ("abi", Json::Str("0.8".into())),
                ("kind", Json::Str("lproc".into())),
                ("params", Json::Arr(wlist(&f.lists[0])?)),
                (
                    "result",
                    Json::u64(one_width(&f.lists[1], "result")? as u64),
                ),
                ("tier", Json::Str("q".into())),
            ]))
        }
        FnKind::Cc | FnKind::R => {
            empty_from(2)?;
            Ok(Json::obj(vec![
                ("abi", Json::Str("0.8".into())),
                ("kind", Json::Str(f.kind.kind_str().into())),
                ("params", args(&f.lists[0])),
                ("results", args(&f.lists[1])),
                ("tier", Json::Str(f.kind.kind_str().into())),
            ]))
        }
    }
}

fn validate_functions_match(obj: &Object) -> Result<(), String> {
    let mut fn_names: BTreeSet<&str> = BTreeSet::new();
    for f in &obj.functions {
        if !fn_names.insert(f.name.as_str()) {
            return Err(err(
                "9.1",
                format!("duplicate function definition @{}", f.name),
            ));
        }
        let sym = obj
            .symbols
            .get(&f.name)
            .ok_or_else(|| err("9.1", format!("function @{} has no symbol entry", f.name)))?;
        if sym.kind != f.kind.kind_str() {
            return Err(err(
                "9.1",
                format!(
                    "function @{} kind {} disagrees with its symbol kind {}",
                    f.name,
                    f.kind.kind_str(),
                    sym.kind
                ),
            ));
        }
    }
    for (name, sym) in &obj.symbols {
        if sym.kind == "data" {
            if !obj.data.contains_key(name) {
                return Err(err("9.1", format!("data symbol @{name} has no data line")));
            }
        } else if obj.function(name).is_none() {
            return Err(err(
                "9.1",
                format!("symbol @{name} has no function definition"),
            ));
        }
        let derived = derived_signature(obj, name)?;
        if derived != sym.signature {
            return Err(err(
                "9.1",
                format!(
                    "signature of @{name} disagrees with its definition: symbols say {}, definition derives {}",
                    sym.signature.to_jcs(),
                    derived.to_jcs()
                ),
            ));
        }
    }
    for name in obj.data.keys() {
        match obj.symbols.get(name) {
            Some(s) if s.kind == "data" => {}
            _ => return Err(err("9.1", format!("data line {name:?} has no data symbol"))),
        }
    }
    Ok(())
}

fn big_pow2(exp: usize) -> BigUint {
    BigUint::from_u64(1).shl(exp)
}
fn add_big(a: &BigUint, b: &BigUint) -> BigUint {
    a.add(b)
}

fn validate_layout(obj: &Object) -> Result<(), String> {
    let Some(layout) = &obj.layout else {
        return Ok(());
    };
    if layout.null_size.is_zero() {
        return Err(err("7.1", "layout null_size must be at least 1"));
    }
    // Ranges [lo, hi) with a label; all disjoint and inside [0, 2^H).
    let limit_h = big_pow2(layout.h);
    let limit_ptr = big_pow2(obj.ptr_width);
    let heap_hi = add_big(&layout.heap_base, &layout.heap_limit_max);
    if heap_hi.cmp_big(&limit_ptr) == std::cmp::Ordering::Greater {
        return Err(err("7.1", "heap_base + heap_limit_max exceeds 2^ptr_width"));
    }
    let mut ranges: Vec<(String, BigUint, BigUint)> = Vec::new();
    ranges.push((
        "null region".into(),
        BigUint::zero(),
        layout.null_size.clone(),
    ));
    ranges.push(("heap".into(), layout.heap_base.clone(), heap_hi.clone()));
    for (gname, g) in &layout.globals {
        if g.image != *gname {
            return Err(err(
                "9.2",
                format!(
                    "global {gname:?} has image {:?}; the image must be the global's own data symbol",
                    g.image
                ),
            ));
        }
        match obj.symbols.get(gname) {
            Some(s) if s.kind == "data" => {}
            _ => {
                return Err(err(
                    "9.2",
                    format!("global {gname:?} has no data symbol of the same name"),
                ));
            }
        }
        let data_len = obj
            .data
            .get(gname)
            .map(|h| (h.len() / 2) as u64)
            .unwrap_or(0);
        if BigUint::from_u64(data_len) != g.size {
            return Err(err(
                "9.2",
                format!(
                    "global {gname:?} size {} disagrees with its data symbol ({} bytes)",
                    g.size, data_len
                ),
            ));
        }
        ranges.push((
            format!("global {gname}"),
            g.base.clone(),
            add_big(&g.base, &g.size),
        ));
    }
    if layout.globals.len() > big_pow2(layout.a_tab).to_usize().unwrap_or(usize::MAX) {
        return Err(err("7.1", "the number of globals exceeds 2^A_tab"));
    }
    for (oracle, regions) in &layout.interface_regions {
        let f = obj
            .function(oracle)
            .filter(|f| f.kind == FnKind::Oracle)
            .ok_or_else(|| {
                err(
                    "7.2",
                    format!("interface_regions names @{oracle}, which is not an oracle"),
                )
            })?;
        let pre = layout.globals.len() + regions.len();
        if pre > big_pow2(layout.a_tab).to_usize().unwrap_or(usize::MAX) {
            return Err(err(
                "7.2",
                format!("oracle @{oracle} has more pre-entries than 2^A_tab"),
            ));
        }
        for (region, base) in regions {
            let param = f.lists[3]
                .iter()
                .find(|p| p.name == *region)
                .ok_or_else(|| {
                    err(
                        "7.2",
                        format!("oracle @{oracle} has no interface region named {region:?}"),
                    )
                })?;
            let (a, w) = match param.ty {
                Type::Region(a, w) => (a, w),
                _ => {
                    return Err(err(
                        "7.2",
                        format!("interface region {region:?} of @{oracle} is not a region type"),
                    ));
                }
            };
            if w != 8 {
                return Err(err(
                    "7.2",
                    format!(
                        "mapped interface region {region:?} of @{oracle} requires W=8, got {w}"
                    ),
                ));
            }
            let size = big_pow2(a);
            ranges.push((
                format!("region {oracle}.{region}"),
                base.clone(),
                add_big(base, &size),
            ));
        }
    }
    for (label, lo, hi) in &ranges {
        if hi.cmp_big(&limit_h) == std::cmp::Ordering::Greater {
            return Err(err(
                "7.2",
                format!("layout range {label} lies outside [0, 2^H)"),
            ));
        }
        if lo.cmp_big(hi) == std::cmp::Ordering::Greater {
            return Err(err("7.1", format!("layout range {label} is inverted")));
        }
    }
    for i in 0..ranges.len() {
        for j in i + 1..ranges.len() {
            let (_, alo, ahi) = &ranges[i];
            let (_, blo, bhi) = &ranges[j];
            let overlap = alo.cmp_big(bhi) == std::cmp::Ordering::Less
                && blo.cmp_big(ahi) == std::cmp::Ordering::Less;
            // Zero-length ranges overlap nothing.
            let az = alo == ahi;
            let bz = blo == bhi;
            if overlap && !az && !bz {
                return Err(err(
                    "7.1",
                    format!("layout ranges {} and {} overlap", ranges[i].0, ranges[j].0),
                ));
            }
        }
    }
    if let Some(img) = &layout.block_image
        && !obj.data.contains_key(img)
    {
        return Err(err(
            "9.1",
            format!("layout block_image {img:?} is not a data entry"),
        ));
    }
    Ok(())
}

fn validate_devices(obj: &Object) -> Result<(), String> {
    let Some(devices) = &obj.devices else {
        return Ok(());
    };
    let m = devices
        .as_obj()
        .ok_or_else(|| err("9.1", "devices must be an object"))?;
    for k in m.keys() {
        if !["models", "services"].contains(&k.as_str()) {
            return Err(err("9.1", format!("unknown field {k:?} in devices")));
        }
    }
    for part in ["models", "services"] {
        let Some(entries) = devices.get(part) else {
            continue;
        };
        let entries = entries
            .as_obj()
            .ok_or_else(|| err("9.1", format!("devices.{part} must be an object")))?;
        for (name, entry) in entries {
            let e = entry
                .as_obj()
                .ok_or_else(|| err("9.1", format!("devices.{part}.{name} must be an object")))?;
            for k in e.keys() {
                if k != "version" {
                    return Err(err(
                        "9.1",
                        format!("unknown field {k:?} in devices.{part}.{name}"),
                    ));
                }
            }
            let v = entry
                .get("version")
                .and_then(|x| x.as_str())
                .ok_or_else(|| {
                    err(
                        "9.1",
                        format!("devices.{part}.{name} needs a version string"),
                    )
                })?;
            if part == "models" && v != "0.8" {
                return Err(err(
                    "9.1",
                    format!("device model {name:?} version must be \"0.8\", got {v:?}"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_memoryless(obj: &Object) -> Result<(), String> {
    if obj.layout.is_some() {
        return Ok(());
    }
    const BANNED: [&str; 13] = [
        "lift.addr",
        "lift.boundscheck",
        "lift.load",
        "lift.store",
        "lift.alloc",
        "lift.free",
        "lift.call",
        "lift.ret",
        "alloc",
        "free",
        "load",
        "store",
        "hostcall",
    ];
    for f in &obj.functions {
        if f.kind == FnKind::Lproc {
            return Err(err(
                "7.2",
                format!("memoryless object contains lproc @{}", f.name),
            ));
        }
        if f.kind == FnKind::Oracle && !f.lists[3].is_empty() {
            return Err(err(
                "7.2",
                format!("memoryless oracle @{} declares mem interface slots", f.name),
            ));
        }
        let mut flat = Vec::new();
        f.flat_instrs(&mut flat);
        for ins in flat {
            let base = ins.op.split('.').take(2).collect::<Vec<_>>().join(".");
            if BANNED.contains(&ins.op.as_str()) || BANNED.contains(&base.as_str()) {
                return Err(err(
                    "7.2",
                    format!("memoryless object uses {} in @{}", ins.op, f.name),
                ));
            }
            // load.iN / store.iN
            if ins.op.starts_with("load.") || ins.op.starts_with("store.") {
                return Err(err(
                    "7.2",
                    format!("memoryless object uses {} in @{}", ins.op, f.name),
                ));
            }
        }
    }
    Ok(())
}

/// Parsed signature views used by executors too.
pub struct OracleSig {
    pub ins: Vec<usize>,
    pub tgt: usize,
    pub fault: usize,
    pub mem: Vec<(usize, usize, String)>,
    pub args: Vec<Type>,
}
pub fn oracle_sig(obj: &Object, name: &str) -> Result<OracleSig, String> {
    let sig = signature_of(obj, name)?;
    Ok(OracleSig {
        ins: widths(sig, "in")?,
        tgt: sig
            .get("tgt")
            .and_then(|v| v.as_usize())
            .ok_or_else(|| err("9.1", "oracle tgt missing"))?,
        fault: sig
            .get("fault")
            .and_then(|v| v.as_usize())
            .ok_or_else(|| err("9.1", "oracle fault missing"))?,
        mem: mem_entries(sig)?,
        args: type_strings(sig, "args")?,
    })
}
pub struct ActionSig {
    pub muts: Vec<usize>,
    pub ins: Vec<usize>,
    pub mem: Vec<(usize, usize, String)>,
    pub args: Vec<Type>,
    pub bounds: Vec<usize>,
}
pub fn action_sig(obj: &Object, name: &str) -> Result<ActionSig, String> {
    let sig = signature_of(obj, name)?;
    Ok(ActionSig {
        muts: widths(sig, "mut")?,
        ins: widths(sig, "in")?,
        mem: mem_entries(sig)?,
        args: type_strings(sig, "args")?,
        bounds: widths(sig, "bounds")?,
    })
}
pub fn signature_of<'a>(obj: &'a Object, name: &str) -> Result<&'a Json, String> {
    if let Some(s) = obj.symbols.get(name) {
        return Ok(&s.signature);
    }
    if let Some(i) = obj.imports.get(name) {
        return Ok(&i.signature);
    }
    Err(err("9.1", format!("unknown symbol @{name}")))
}
pub fn kind_of(obj: &Object, name: &str) -> Option<String> {
    if let Some(s) = obj.symbols.get(name) {
        return Some(s.kind.clone());
    }
    obj.imports
        .get(name)
        .and_then(|i| i.signature.get("kind"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string())
}
pub fn cc_sig(obj: &Object, name: &str) -> Result<(Vec<Type>, Vec<Type>), String> {
    let sig = signature_of(obj, name)?;
    Ok((type_strings(sig, "params")?, type_strings(sig, "results")?))
}
pub fn lproc_sig(obj: &Object, name: &str) -> Result<(Vec<usize>, usize), String> {
    let sig = signature_of(obj, name)?;
    Ok((
        widths(sig, "params")?,
        sig.get("result")
            .and_then(|v| v.as_usize())
            .ok_or_else(|| err("9.1", "lproc result missing"))?,
    ))
}

const CC_BINOPS: [&str; 8] = ["add", "sub", "mul", "and", "or", "xor", "shl", "lshr"];
const GATES_1: [&str; 6] = ["x", "h", "s", "sdg", "t", "tdg"];

/// Base mnemonic without type/predicate suffix beyond the first dot part
/// used by the schema (`icmp.eq` keeps both pieces for `lift.icmp.eq`).
pub fn is_int_literal(text: &str) -> bool {
    let t = text.strip_prefix('-').unwrap_or(text);
    !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit())
}

fn validate_function(obj: &Object, f: &Function) -> Result<(), String> {
    // Unique declared names across header lists and regs.
    let mut names: BTreeMap<&str, &Param> = BTreeMap::new();
    for list in f.lists.iter() {
        for p in list {
            if names.insert(p.name.as_str(), p).is_some() {
                return Err(err(
                    "5.1",
                    format!("duplicate name {:?} in header of @{}", p.name, f.name),
                ));
            }
        }
    }
    for p in &f.regs {
        if names.insert(p.name.as_str(), p).is_some() {
            return Err(err(
                "3.7",
                format!("duplicate location {:?} in @{}", p.name, f.name),
            ));
        }
        if matches!(f.kind, FnKind::Oracle | FnKind::Lproc) && !matches!(p.ty, Type::Reg(_)) {
            return Err(err(
                "3.7",
                format!("regs entry {:?} of @{} is not a reg type", p.name, f.name),
            ));
        }
    }
    // Roles on interface lists.
    match f.kind {
        FnKind::Oracle => {
            for p in &f.lists[0] {
                if p.role.as_deref() != Some("in") {
                    return Err(err(
                        "5.1",
                        format!("oracle input {:?} of @{} needs role in", p.name, f.name),
                    ));
                }
            }
            for p in &f.lists[3] {
                if p.role.as_deref() != Some("in") {
                    return Err(err(
                        "5.1",
                        format!(
                            "oracle mem region {:?} of @{} must have role in",
                            p.name, f.name
                        ),
                    ));
                }
            }
        }
        FnKind::Action | FnKind::PermAction => {
            for p in &f.lists[0] {
                if p.role.as_deref() != Some("mut") {
                    return Err(err(
                        "5.1",
                        format!(
                            "action mut bundle {:?} of @{} needs role mut",
                            p.name, f.name
                        ),
                    ));
                }
            }
            for p in &f.lists[1] {
                if p.role.as_deref() != Some("in") {
                    return Err(err(
                        "5.1",
                        format!("action in bundle {:?} of @{} needs role in", p.name, f.name),
                    ));
                }
            }
        }
        FnKind::Cc | FnKind::R => {
            for p in f.lists[0].iter().chain(f.lists[1].iter()) {
                match p.ty {
                    Type::I(n) if (1..=128).contains(&n) => {}
                    Type::I(n) => {
                        return Err(err(
                            "3.3",
                            format!("i{n} in @{} is outside 1..=128", f.name),
                        ));
                    }
                    Type::Ptr | Type::F64 | Type::Token | Type::Qreg(_) => {}
                    _ => {
                        return Err(err(
                            "3.3",
                            format!(
                                "parameter {:?} of @{} has a non-classical type",
                                p.name, f.name
                            ),
                        ));
                    }
                }
            }
            if f.has_regs_line && !f.regs.is_empty() {
                return Err(err(
                    "3.7",
                    format!("CC/R function @{} must not declare locations", f.name),
                ));
            }
        }
        FnKind::Lproc => {}
    }
    // Blocks: unique names, unique param names.
    let mut block_names: BTreeSet<&str> = BTreeSet::new();
    for b in &f.blocks {
        if !block_names.insert(b.name.as_str()) {
            return Err(err(
                "16.2",
                format!("duplicate block ^{} in @{}", b.name, f.name),
            ));
        }
        let mut bn: BTreeSet<&str> = BTreeSet::new();
        for p in &b.params {
            if !bn.insert(p.name.as_str()) {
                return Err(err(
                    "16.2",
                    format!("duplicate block parameter {:?} in @{}", p.name, f.name),
                ));
            }
        }
    }
    validate_terminators(f)?;
    let mut flat = Vec::new();
    f.flat_instrs(&mut flat);
    for ins in &flat {
        validate_instr(obj, f, ins)?;
    }
    if matches!(f.kind, FnKind::Action | FnKind::PermAction) {
        let found = collect_bounds(f);
        if found != f.bounds {
            return Err(err(
                "5.3",
                format!(
                    "bounds list of @{} declares {:?} but its qloops give {:?}",
                    f.name, f.bounds, found
                ),
            ));
        }
    }
    Ok(())
}

fn collect_bounds(f: &Function) -> Vec<usize> {
    fn walk(instrs: &[crate::object::Instr], out: &mut Vec<usize>) {
        for ins in instrs {
            if ins.op == "qloop"
                && let Some(n) = ins.operands.first().and_then(|s| s.parse::<usize>().ok())
            {
                out.push(n);
            }
            for arm in &ins.nested {
                walk(arm, out);
            }
        }
    }
    let mut out = Vec::new();
    for b in &f.blocks {
        walk(&b.instrs, &mut out);
    }
    out
}

const LIFTED_TERMINATORS: [&str; 6] = [
    "qjump",
    "qbranch",
    "qjumpr",
    "lift.ret",
    "lift.retval",
    "fault",
];

fn validate_terminators(f: &Function) -> Result<(), String> {
    match f.kind {
        FnKind::Oracle | FnKind::Lproc => {
            for b in &f.blocks {
                for (i, ins) in b.instrs.iter().enumerate() {
                    let is_term = LIFTED_TERMINATORS.contains(&ins.op.as_str());
                    if is_term && i != b.instrs.len() - 1 {
                        return Err(err(
                            "3.6",
                            format!(
                                "terminator {} is not the last instruction of ^{} in @{}",
                                ins.op, b.name, f.name
                            ),
                        ));
                    }
                    if ins.op == "lift.retval" && f.kind == FnKind::Lproc {
                        return Err(err(
                            "4.2",
                            format!("lift.retval is ill-formed in lproc @{}", f.name),
                        ));
                    }
                    if ins.op == "lift.ret" && f.kind == FnKind::Oracle {
                        return Err(err(
                            "4.2",
                            format!("lift.ret is ill-formed in oracle @{}", f.name),
                        ));
                    }
                }
                let last = &b.instrs[b.instrs.len() - 1];
                if !LIFTED_TERMINATORS.contains(&last.op.as_str()) {
                    return Err(err(
                        "3.6",
                        format!(
                            "block ^{} in @{} does not end in a terminator",
                            b.name, f.name
                        ),
                    ));
                }
            }
        }
        FnKind::Cc | FnKind::R => {
            for b in &f.blocks {
                let last = &b.instrs[b.instrs.len() - 1];
                if !["br", "ret", "trap"].contains(&last.op.as_str()) {
                    return Err(err(
                        "3.4",
                        format!("block ^{} in @{} must end with br or ret", b.name, f.name),
                    ));
                }
                for ins in &b.instrs[..b.instrs.len() - 1] {
                    if ["br", "ret"].contains(&ins.op.as_str()) {
                        return Err(err(
                            "3.4",
                            format!(
                                "{} is not the last instruction of ^{} in @{}",
                                ins.op, b.name, f.name
                            ),
                        ));
                    }
                }
            }
        }
        FnKind::Action | FnKind::PermAction => {}
    }
    Ok(())
}

pub fn at_name(token: &str) -> Option<&str> {
    token.strip_prefix('@')
}
/// `^name` or `^name(a, b)` -> (name, args).
pub fn branch_target(token: &str) -> Option<(String, Vec<String>)> {
    let t = token.strip_prefix('^')?;
    match t.find('(') {
        None => Some((t.to_string(), Vec::new())),
        Some(i) => {
            let inner = crate::object::paren_inner(&t[i..])?;
            Some((t[..i].to_string(), crate::object::split_list(inner)))
        }
    }
}
pub fn list_operand(token: &str) -> Result<Vec<String>, String> {
    let inner = crate::object::paren_inner(token).ok_or_else(|| {
        err(
            "4.8",
            format!("expected a parenthesised operand list, got {token:?}"),
        )
    })?;
    Ok(crate::object::split_list(inner))
}

fn arity(f: &Function, ins: &crate::object::Instr, want: usize) -> Result<(), String> {
    if ins.operands.len() != want {
        return Err(err(
            "4.8",
            format!(
                "{} in @{} takes {want} operands, got {} ({})",
                ins.op,
                f.name,
                ins.operands.len(),
                ins.raw
            ),
        ));
    }
    Ok(())
}
fn lhs_n(f: &Function, ins: &crate::object::Instr, want: usize) -> Result<(), String> {
    if ins.lhs.len() != want {
        return Err(err(
            "4.8",
            format!(
                "{} in @{} must produce {want} results, got {} ({})",
                ins.op,
                f.name,
                ins.lhs.len(),
                ins.raw
            ),
        ));
    }
    Ok(())
}

fn validate_instr(obj: &Object, f: &Function, ins: &crate::object::Instr) -> Result<(), String> {
    let op = ins.op.as_str();
    let is_lifted =
        op.starts_with("lift.") || ["qjump", "qbranch", "qjumpr", "fault"].contains(&op);
    let is_region_call = ["oracle_call", "action_call", "adj", "ctl"].contains(&op);
    match f.kind {
        FnKind::Oracle | FnKind::Lproc => {
            if !is_lifted {
                return Err(err(
                    "5.1",
                    format!("{op} is not a lifted row but appears in @{}", f.name),
                ));
            }
        }
        FnKind::Cc | FnKind::R => {
            if is_lifted {
                return Err(err(
                    "5.1",
                    format!("lifted row {op} outside an oracle body in @{}", f.name),
                ));
            }
            if (is_gate(op) || op.starts_with('q') && !op.starts_with("qreg.")) && !is_region_call {
                return Err(err(
                    "4.8",
                    format!("Q instruction {op} in classical function @{}", f.name),
                ));
            }
            if op == "effect" && f.kind != FnKind::R {
                return Err(err(
                    "4.1",
                    format!("effect is R only, used in CC @{}", f.name),
                ));
            }
            if (op == "hostcall" || op == "measure" || op.starts_with("qreg."))
                && f.kind != FnKind::Cc
            {
                return Err(err(
                    "4.1",
                    format!("{op} is CC only, used in R @{}", f.name),
                ));
            }
        }
        FnKind::Action | FnKind::PermAction => {
            if [
                "measure", "effect", "br", "call", "uncall", "trap", "hostcall",
            ]
            .contains(&op)
                || op.starts_with("qreg.")
                || op.starts_with("lift.")
            {
                return Err(err(
                    "5.1",
                    format!("a region must not contain {op} (@{})", f.name),
                ));
            }
        }
    }
    validate_opcode(obj, f, ins)
}

fn is_gate(op: &str) -> bool {
    GATES_1.contains(&op) || ["cx", "ccx", "mcx", "swap", "cz", "rx", "ry", "rz"].contains(&op)
}

fn suffix_width(op: &str, prefix: &str) -> Option<usize> {
    let rest = op.strip_prefix(prefix)?;
    let n = rest.strip_prefix('i')?;
    n.parse::<usize>().ok()
}

fn validate_opcode(obj: &Object, f: &Function, ins: &crate::object::Instr) -> Result<(), String> {
    let op = ins.op.as_str();
    // Adj/ctl wrap one region call.
    if op == "adj" || op == "ctl" {
        let (call_idx, _) = if op == "adj" { (0, 0) } else { (1, 0) };
        let inner = ins
            .operands
            .get(call_idx)
            .ok_or_else(|| err("4.7", format!("{op} in @{} wraps no call", f.name)))?;
        if inner != "oracle_call" && inner != "action_call" {
            return Err(err(
                "4.7",
                format!(
                    "{op} in @{} must wrap oracle_call or action_call, got {inner:?}",
                    f.name
                ),
            ));
        }
        let synthetic = crate::object::Instr {
            lhs: Vec::new(),
            op: inner.clone(),
            operands: ins.operands[call_idx + 1..].to_vec(),
            nested: Vec::new(),
            line: ins.line,
            raw: ins.raw.clone(),
        };
        lhs_n(f, ins, 0)?;
        return validate_region_call(obj, f, &synthetic);
    }
    if op == "oracle_call" || op == "action_call" {
        lhs_n(f, ins, 0)?;
        return validate_region_call(obj, f, ins);
    }
    // Classical / lifted arithmetic families share shapes.
    let base = op.strip_prefix("lift.").unwrap_or(op);
    if let Some(n) = suffix_width(base, "const.") {
        if n == 0 {
            return Err(err("3.3", format!("{op} has width 0 in @{}", f.name)));
        }
        arity(f, ins, 1)?;
        lhs_n(f, ins, 1)?;
        if !is_int_literal(&ins.operands[0]) {
            return Err(err(
                "16.1",
                format!(
                    "const operand {:?} in @{} is not an INT literal",
                    ins.operands[0], f.name
                ),
            ));
        }
        return Ok(());
    }
    if CC_BINOPS.contains(&base)
        || base == "ashr"
        || ["udiv", "urem", "sdiv", "srem"].contains(&base)
    {
        arity(f, ins, 2)?;
        lhs_n(f, ins, 1)?;
        return Ok(());
    }
    if let Some(pred) = base.strip_prefix("icmp.") {
        if !["eq", "ne", "ult", "ule", "slt", "sle"].contains(&pred) {
            return Err(err(
                "4.8",
                format!("unknown icmp predicate {pred:?} in @{}", f.name),
            ));
        }
        arity(f, ins, 2)?;
        lhs_n(f, ins, 1)?;
        return Ok(());
    }
    if base == "select" {
        arity(f, ins, 3)?;
        lhs_n(f, ins, 1)?;
        return Ok(());
    }
    for cast in ["zext.", "sext.", "trunc.", "ptrtoint."] {
        if suffix_width(base, cast).is_some() {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 1)?;
            return Ok(());
        }
    }
    match op {
        "inttoptr" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 1)
        }
        "alloc" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 1)
        }
        "free" | "trap" => {
            if op == "free" {
                arity(f, ins, 1)?;
            } else {
                arity(f, ins, 0)?;
            }
            lhs_n(f, ins, 0)
        }
        "br" => validate_br(obj, f, ins),
        "call" => validate_call(obj, f, ins),
        "ret" => {
            let (_, results) = cc_sig(obj, &f.name)?;
            if ins.operands.len() != results.len() {
                return Err(err(
                    "4.1",
                    format!(
                        "ret in @{} returns {} values for {} results",
                        f.name,
                        ins.operands.len(),
                        results.len()
                    ),
                ));
            }
            lhs_n(f, ins, 0)
        }
        "uncall" => validate_uncall(obj, f, ins),
        "hostcall" => {
            arity(f, ins, 3)?;
            lhs_n(f, ins, 1)?;
            let service = &ins.operands[0];
            let declared = obj
                .devices
                .as_ref()
                .and_then(|d| d.get("services"))
                .and_then(|s| s.get(service))
                .is_some();
            if !declared {
                return Err(err(
                    "10",
                    format!(
                        "hostcall service {service:?} in @{} is not declared in devices.services",
                        f.name
                    ),
                ));
            }
            Ok(())
        }
        "measure" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 1)
        }
        "effect" => {
            if ins.operands.is_empty() || !ins.operands[0].contains('.') {
                return Err(err(
                    "4.1",
                    format!("effect in @{} needs a dev.op operand", f.name),
                ));
            }
            lhs_n(f, ins, 0)
        }
        "qreg.new" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 1)
        }
        "qreg.free" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 0)
        }
        "qreg.slice" => {
            arity(f, ins, 3)?;
            lhs_n(f, ins, 1)
        }
        "qreg.concat" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 1)
        }
        _ if op.starts_with("load.") || op.starts_with("lift.load.") => {
            let n = op
                .rsplit('.')
                .next()
                .and_then(|t| t.strip_prefix('i'))
                .and_then(|t| t.parse::<usize>().ok())
                .unwrap_or(0);
            if n == 0 || !n.is_multiple_of(8) || n > 128 {
                return Err(err(
                    "3.3",
                    format!("{op} in @{} requires N in {{8,16,...,128}}", f.name),
                ));
            }
            arity(f, ins, 1)?;
            lhs_n(f, ins, 1)
        }
        _ if op.starts_with("store.") || op.starts_with("lift.store.") => {
            let n = op
                .rsplit('.')
                .next()
                .and_then(|t| t.strip_prefix('i'))
                .and_then(|t| t.parse::<usize>().ok())
                .unwrap_or(0);
            if n == 0 || !n.is_multiple_of(8) || n > 128 {
                return Err(err(
                    "3.3",
                    format!("{op} in @{} requires N in {{8,16,...,128}}", f.name),
                ));
            }
            arity(f, ins, 2)?;
            lhs_n(f, ins, 0)
        }
        _ => validate_lifted_or_q(obj, f, ins),
    }
}

fn callee_kind(obj: &Object, f: &Function, token: &str) -> Result<(String, String), String> {
    let name = at_name(token)
        .ok_or_else(|| {
            err(
                "4.8",
                format!("callee operand {token:?} in @{} must be a @name", f.name),
            )
        })?
        .to_string();
    let kind = kind_of(obj, &name)
        .ok_or_else(|| err("9.1", format!("@{} calls unknown symbol @{name}", f.name)))?;
    Ok((name, kind))
}

fn validate_br(_obj: &Object, f: &Function, ins: &crate::object::Instr) -> Result<(), String> {
    lhs_n(f, ins, 0)?;
    let targets: Vec<&String> = match ins.operands.len() {
        1 => vec![&ins.operands[0]],
        3 => vec![&ins.operands[1], &ins.operands[2]],
        n => {
            return Err(err(
                "4.1",
                format!("br in @{} takes 1 or 3 operands, got {n}", f.name),
            ));
        }
    };
    for t in targets {
        let (name, args) = branch_target(t)
            .ok_or_else(|| err("4.1", format!("malformed br target {t:?} in @{}", f.name)))?;
        let block = f.block(&name).ok_or_else(|| {
            err(
                "3.4",
                format!("br in @{} targets unknown block ^{name}", f.name),
            )
        })?;
        if args.len() != block.params.len() {
            return Err(err(
                "3.4",
                format!(
                    "br to ^{name} in @{} passes {} arguments for {} block parameters",
                    f.name,
                    args.len(),
                    block.params.len()
                ),
            ));
        }
    }
    Ok(())
}

fn validate_call(obj: &Object, f: &Function, ins: &crate::object::Instr) -> Result<(), String> {
    if ins.operands.is_empty() {
        return Err(err("4.1", format!("call in @{} needs a callee", f.name)));
    }
    let (name, kind) = callee_kind(obj, f, &ins.operands[0])?;
    if kind != "cc" && kind != "r" {
        return Err(err(
            "3.1",
            format!(
                "call in @{} targets @{name} of kind {kind}, not a CC/R function",
                f.name
            ),
        ));
    }
    if f.kind == FnKind::R && kind != "r" {
        return Err(err(
            "3.1",
            format!(
                "R function @{} may call only R functions, not @{name} ({kind})",
                f.name
            ),
        ));
    }
    let (params, results) = cc_sig(obj, &name)?;
    if ins.operands.len() - 1 != params.len() {
        return Err(err(
            "4.1",
            format!(
                "call @{name} in @{} passes {} arguments for {} parameters",
                f.name,
                ins.operands.len() - 1,
                params.len()
            ),
        ));
    }
    if ins.lhs.len() != results.len() {
        return Err(err(
            "4.1",
            format!(
                "call @{name} in @{} binds {} results for {} declared results",
                f.name,
                ins.lhs.len(),
                results.len()
            ),
        ));
    }
    Ok(())
}

fn validate_uncall(obj: &Object, f: &Function, ins: &crate::object::Instr) -> Result<(), String> {
    arity(f, ins, 3)?;
    lhs_n(f, ins, 0)?;
    let (name, kind) = callee_kind(obj, f, &ins.operands[0])?;
    if kind != "cc" && kind != "r" {
        return Err(err(
            "3.1",
            format!("uncall in @{} targets @{name} of kind {kind}", f.name),
        ));
    }
    let (params, results) = cc_sig(obj, &name)?;
    let args = list_operand(&ins.operands[1])?;
    let res = list_operand(&ins.operands[2])?;
    if args.len() != params.len() || res.len() != results.len() {
        return Err(err(
            "4.1",
            format!(
                "uncall @{name} in @{} has boundary counts {}/{} for signature {}/{}",
                f.name,
                args.len(),
                res.len(),
                params.len(),
                results.len()
            ),
        ));
    }
    Ok(())
}

fn validate_region_call(
    obj: &Object,
    f: &Function,
    ins: &crate::object::Instr,
) -> Result<(), String> {
    if ins.op == "oracle_call" {
        arity(f, ins, 7)?;
        let (name, kind) = callee_kind(obj, f, &ins.operands[0])?;
        if kind != "oracle" {
            return Err(err(
                "4.7",
                format!("oracle_call in @{} targets @{name} of kind {kind}", f.name),
            ));
        }
        let sig = oracle_sig(obj, &name)?;
        let ins_list = list_operand(&ins.operands[2])?;
        let mem_list = list_operand(&ins.operands[5])?;
        let arg_list = list_operand(&ins.operands[6])?;
        if ins_list.len() != sig.ins.len()
            || mem_list.len() != sig.mem.len()
            || arg_list.len() != sig.args.len()
        {
            return Err(err(
                "4.7",
                format!(
                    "oracle_call @{name} in @{} operand counts disagree with the callee interface",
                    f.name
                ),
            ));
        }
    } else {
        arity(f, ins, 5)?;
        let (name, kind) = callee_kind(obj, f, &ins.operands[0])?;
        if kind != "action" && kind != "perm_action" {
            return Err(err(
                "4.7",
                format!("action_call in @{} targets @{name} of kind {kind}", f.name),
            ));
        }
        if f.kind == FnKind::PermAction && kind != "perm_action" {
            return Err(err(
                "5.3",
                format!(
                    "permutation action @{} calls non-permutation action @{name}",
                    f.name
                ),
            ));
        }
        let sig = action_sig(obj, &name)?;
        let muts = list_operand(&ins.operands[1])?;
        let ins_list = list_operand(&ins.operands[2])?;
        let mem_list = list_operand(&ins.operands[3])?;
        let arg_list = list_operand(&ins.operands[4])?;
        if muts.len() != sig.muts.len()
            || ins_list.len() != sig.ins.len()
            || mem_list.len() != sig.mem.len()
            || arg_list.len() != sig.args.len()
        {
            return Err(err(
                "4.7",
                format!(
                    "action_call @{name} in @{} operand counts disagree with the callee interface",
                    f.name
                ),
            ));
        }
    }
    Ok(())
}

fn validate_lifted_or_q(
    obj: &Object,
    f: &Function,
    ins: &crate::object::Instr,
) -> Result<(), String> {
    let op = ins.op.as_str();
    match op {
        "lift.mov" | "lift.label" | "lift.addr" | "lift.alloc" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 1)?;
            if op == "lift.label" {
                let t = &ins.operands[0];
                let (name, args) = branch_target(t).ok_or_else(|| {
                    err(
                        "4.2",
                        format!("lift.label operand {t:?} in @{} is not a block", f.name),
                    )
                })?;
                if !args.is_empty() || f.block(&name).is_none() {
                    return Err(err(
                        "4.2",
                        format!("lift.label in @{} names unknown block ^{name}", f.name),
                    ));
                }
            }
            if op == "lift.addr" {
                let name = at_name(&ins.operands[0]).ok_or_else(|| {
                    err(
                        "4.2",
                        format!("lift.addr operand in @{} must be a @name", f.name),
                    )
                })?;
                let is_global = obj
                    .layout
                    .as_ref()
                    .is_some_and(|l| l.globals.contains_key(name));
                let is_region =
                    f.kind == FnKind::Oracle && f.lists[3].iter().any(|p| p.name == name);
                if !is_global && !is_region {
                    return Err(err(
                        "4.2",
                        format!(
                            "lift.addr @{name} in @{} names neither a global nor an interface region of the containing oracle",
                            f.name
                        ),
                    ));
                }
            }
            Ok(())
        }
        "lift.parmov" => {
            arity(f, ins, 1)?;
            let args = list_operand(&ins.operands[0])?;
            if args.len() != ins.lhs.len() || ins.lhs.is_empty() {
                return Err(err(
                    "4.2",
                    format!(
                        "lift.parmov in @{} moves {} values into {} destinations",
                        f.name,
                        args.len(),
                        ins.lhs.len()
                    ),
                ));
            }
            let distinct: BTreeSet<&String> = ins.lhs.iter().collect();
            if distinct.len() != ins.lhs.len() {
                return Err(err(
                    "3.3",
                    format!(
                        "lift.parmov destinations in @{} are not pairwise distinct",
                        f.name
                    ),
                ));
            }
            Ok(())
        }
        "lift.boundscheck" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 0)
        }
        "lift.free" | "lift.ret" | "lift.retval" | "qjumpr" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 0)
        }
        "lift.faultif" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 0)?;
            if !is_int_literal(&ins.operands[1]) {
                return Err(err(
                    "4.2",
                    format!(
                        "lift.faultif code in @{} must be an integer literal",
                        f.name
                    ),
                ));
            }
            Ok(())
        }
        "fault" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 0)?;
            if crate::bigint::BigUint::parse_int(&ins.operands[0]).is_err() {
                return Err(err(
                    "4.2",
                    format!("fault code in @{} must be an integer literal", f.name),
                ));
            }
            Ok(())
        }
        "lift.call" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 1)?;
            let (name, kind) = callee_kind(obj, f, &ins.operands[0])?;
            if kind != "lproc" {
                return Err(err(
                    "4.2",
                    format!("lift.call in @{} targets @{name} of kind {kind}", f.name),
                ));
            }
            let (params, _) = lproc_sig(obj, &name)?;
            let args = list_operand(&ins.operands[1])?;
            if args.len() != params.len() {
                return Err(err(
                    "3.3",
                    format!(
                        "lift.call @{name} in @{} passes {} arguments for {} parameters",
                        f.name,
                        args.len(),
                        params.len()
                    ),
                ));
            }
            Ok(())
        }
        "lift.oracle_call" | "lift.oracle_uncall" => {
            let synthetic = crate::object::Instr {
                lhs: Vec::new(),
                op: "oracle_call".into(),
                operands: ins.operands.clone(),
                nested: Vec::new(),
                line: ins.line,
                raw: ins.raw.clone(),
            };
            lhs_n(f, ins, 0)?;
            validate_region_call(obj, f, &synthetic)
        }
        "lift.perm_call" => {
            lhs_n(f, ins, 0)?;
            arity(f, ins, 5)?;
            let (name, kind) = callee_kind(obj, f, &ins.operands[0])?;
            if kind != "perm_action" {
                return Err(err(
                    "4.2",
                    format!(
                        "lift.perm_call in @{} targets @{name} of kind {kind}",
                        f.name
                    ),
                ));
            }
            let sig = action_sig(obj, &name)?;
            let muts = list_operand(&ins.operands[1])?;
            let ins_l = list_operand(&ins.operands[2])?;
            let mem_l = list_operand(&ins.operands[3])?;
            let arg_l = list_operand(&ins.operands[4])?;
            if muts.len() != sig.muts.len()
                || ins_l.len() != sig.ins.len()
                || mem_l.len() != sig.mem.len()
                || arg_l.len() != sig.args.len()
            {
                return Err(err(
                    "3.3",
                    format!(
                        "lift.perm_call @{name} in @{} operand counts disagree with the callee signature",
                        f.name
                    ),
                ));
            }
            Ok(())
        }
        "qjump" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 0)?;
            check_q_target(f, &ins.operands[0])
        }
        "qbranch" => {
            arity(f, ins, 3)?;
            lhs_n(f, ins, 0)?;
            check_q_target(f, &ins.operands[1])?;
            check_q_target(f, &ins.operands[2])
        }
        _ => validate_q(obj, f, ins),
    }
}

fn check_q_target(f: &Function, token: &str) -> Result<(), String> {
    let name = token.strip_prefix('^').ok_or_else(|| {
        err(
            "4.2",
            format!("branch target {token:?} in @{} must be a ^block", f.name),
        )
    })?;
    if f.block(name).is_none() {
        return Err(err(
            "3.6",
            format!("branch in @{} targets unknown block ^{name}", f.name),
        ));
    }
    Ok(())
}

fn validate_q(obj: &Object, f: &Function, ins: &crate::object::Instr) -> Result<(), String> {
    let op = ins.op.as_str();
    if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
        return Err(err(
            "4.8",
            format!("unknown opcode {op:?} in @{} ({})", f.name, ins.raw),
        ));
    }
    if f.kind == FnKind::PermAction
        && ["h", "s", "sdg", "t", "tdg", "cz", "rx", "ry", "rz"].contains(&op)
    {
        return Err(err(
            "5.3",
            format!(
                "permutation action @{} uses non-permutation gate {op}",
                f.name
            ),
        ));
    }
    match op {
        "x" | "h" | "s" | "sdg" | "t" | "tdg" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 0)
        }
        "cx" | "swap" | "cz" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 0)
        }
        "ccx" => {
            arity(f, ins, 3)?;
            lhs_n(f, ins, 0)
        }
        "mcx" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 0)?;
            list_operand(&ins.operands[0])?;
            Ok(())
        }
        "rx" | "ry" | "rz" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 0)
        }
        "qalloc" | "qborrow" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 1)
        }
        "qfree" | "qreturn" | "qpred" | "qunpred" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 0)
        }
        "qcopy" | "qload" => {
            lhs_n(f, ins, 1)?;
            if op == "qcopy" {
                arity(f, ins, 1)
            } else {
                arity(f, ins, 2)
            }
        }
        "quncopy" | "qxor" | "qexch" => {
            arity(f, ins, 2)?;
            lhs_n(f, ins, 0)
        }
        "qunload" | "qstore" | "qexchm" => {
            arity(f, ins, 3)?;
            lhs_n(f, ins, 0)
        }
        "qinplace" => {
            arity(f, ins, 4)?;
            lhs_n(f, ins, 0)?;
            for tok in [&ins.operands[0], &ins.operands[1]] {
                let (name, kind) = callee_kind(obj, f, tok)?;
                if kind != "oracle" {
                    return Err(err(
                        "4.5",
                        format!(
                            "qinplace in @{} names @{name} of kind {kind}, not an oracle",
                            f.name
                        ),
                    ));
                }
            }
            Ok(())
        }
        "qregion" => {
            if ins.operands.len() != 3 && ins.operands.len() != 4 {
                return Err(err(
                    "4.6",
                    format!("qregion in @{} takes 3 or 4 operands", f.name),
                ));
            }
            lhs_n(f, ins, 0)?;
            if ins.operands.len() == 4 {
                let name = at_name(&ins.operands[3]).ok_or_else(|| {
                    err(
                        "4.6",
                        format!("qregion image in @{} must be a @data reference", f.name),
                    )
                })?;
                if kind_of(obj, name).as_deref() != Some("data") {
                    return Err(err(
                        "4.6",
                        format!("qregion image @{name} in @{} is not a data symbol", f.name),
                    ));
                }
            }
            Ok(())
        }
        "qif" => {
            arity(f, ins, 1)?;
            lhs_n(f, ins, 0)?;
            if ins.nested.len() != 2 {
                return Err(err(
                    "4.6",
                    format!("qif in @{} needs then and else arms", f.name),
                ));
            }
            Ok(())
        }
        "qloop" => {
            lhs_n(f, ins, 0)?;
            if ins.nested.len() != 1 {
                return Err(err("4.6", format!("qloop in @{} needs one body", f.name)));
            }
            let ok =
                ins.operands.len() == 1 || (ins.operands.len() == 3 && ins.operands[1] == "while");
            if !ok {
                return Err(err(
                    "4.6",
                    format!("qloop in @{} has malformed operands ({})", f.name, ins.raw),
                ));
            }
            if !is_int_literal(&ins.operands[0]) {
                return Err(err(
                    "4.6",
                    format!("qloop count in @{} must be an integer literal", f.name),
                ));
            }
            Ok(())
        }
        _ => Err(err(
            "4.8",
            format!("unknown opcode {op:?} in @{} ({})", f.name, ins.raw),
        )),
    }
}

fn validate_action_acyclic(obj: &Object) -> Result<(), String> {
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in &obj.functions {
        if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
            continue;
        }
        let mut edges = Vec::new();
        let mut flat = Vec::new();
        f.flat_instrs(&mut flat);
        for ins in flat {
            let callee_tok: Option<&String> = match ins.op.as_str() {
                "action_call" => ins.operands.first(),
                "adj" => ins.operands.get(1),
                "ctl" => ins.operands.get(2),
                _ => None,
            };
            if let Some(tok) = callee_tok
                && let Some(name) = at_name(tok)
                && matches!(
                    kind_of(obj, name).as_deref(),
                    Some("action") | Some("perm_action")
                )
            {
                edges.push(name.to_string());
            }
        }
        graph.insert(f.name.clone(), edges);
    }
    fn visit(
        node: &str,
        graph: &BTreeMap<String, Vec<String>>,
        state: &mut BTreeMap<String, u8>,
    ) -> Result<(), String> {
        state.insert(node.to_string(), 1);
        for next in graph.get(node).cloned().unwrap_or_default() {
            match state.get(&next).copied().unwrap_or(0) {
                1 => {
                    return Err(err(
                        "5.1",
                        format!("action call graph has a cycle through @{next}"),
                    ));
                }
                0 => visit(&next, graph, state)?,
                _ => {}
            }
        }
        state.insert(node.to_string(), 2);
        Ok(())
    }
    let mut state: BTreeMap<String, u8> = BTreeMap::new();
    let nodes: Vec<String> = graph.keys().cloned().collect();
    for n in nodes {
        if state.get(&n).copied().unwrap_or(0) == 0 {
            visit(&n, &graph, &mut state)?;
        }
    }
    Ok(())
}
