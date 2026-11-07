use bennett_vm::{json::Json, object, validate};
#[allow(dead_code)]
pub fn source(functions: &str) -> String {
    source_at("0.9", functions)
}
pub fn source_at(v: &str, functions: &str) -> String {
    let text = format!(
        "bennett-object isa={v} object={v} launch={v} sched={v} cert={v} coverage={v} target={v} ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\n{functions}\n"
    );
    let mut obj = object::parse(&text).unwrap();
    for (index, f) in obj.functions.iter().enumerate() {
        obj.symbols.insert(
            f.name.clone(),
            object::Symbol {
                index,
                name: f.name.clone(),
                kind: f.kind.kind_str().into(),
                signature: Json::Null,
                stored_hash: "0".repeat(64),
            },
        );
    }
    let symbols = obj
        .symbols
        .values()
        .map(|s| {
            (
                s.index.to_string(),
                Json::obj(vec![
                    ("name", Json::Str(s.name.clone())),
                    ("kind", Json::Str(s.kind.clone())),
                    (
                        "signature",
                        validate::derived_signature(&obj, &s.name).unwrap(),
                    ),
                    ("interface_hash", Json::Str(s.stored_hash.clone())),
                ]),
            )
        })
        .collect();
    let mut text = text.replacen("\n{}\n", &format!("\n{}\n", Json::Obj(symbols).to_jcs()), 1);
    if obj
        .functions
        .iter()
        .any(|f| f.kind == object::FnKind::Oracle && !f.lists[3].is_empty())
    {
        let regions = obj
            .functions
            .iter()
            .filter(|f| f.kind == object::FnKind::Oracle)
            .enumerate()
            .map(|(index, f)| {
                (
                    f.name.clone(),
                    Json::Obj(
                        f.lists[3]
                            .iter()
                            .map(|p| {
                                (
                                    p.name.clone(),
                                    Json::obj(vec![("base", Json::u64(16 + index as u64 * 4))]),
                                )
                            })
                            .collect(),
                    ),
                )
            })
            .collect();
        let layout = Json::obj(vec![
            ("ptr_width", Json::u64(32)),
            ("H", Json::u64(6)),
            ("A_tab", Json::u64(2)),
            ("S_cells", Json::u64(4)),
            ("null_size", Json::u64(16)),
            ("heap_base", Json::u64(32)),
            ("heap_limit_max", Json::u64(32)),
            ("globals", Json::obj(vec![])),
            ("interface_regions", Json::Obj(regions)),
        ]);
        text.push_str(&format!("\nsection layout\n{}\n", layout.to_jcs()));
    }
    sign(&text)
}
/// Fill stored interface hashes after constructing this test's source.
pub fn sign(text: &str) -> String {
    let obj = object::parse(text).unwrap();
    let ids = bennett_vm::identity::compute(&obj).unwrap();
    let symbols = obj
        .symbols
        .values()
        .map(|s| {
            (
                s.index.to_string(),
                Json::obj(vec![
                    ("name", Json::Str(s.name.clone())),
                    ("kind", Json::Str(s.kind.clone())),
                    ("signature", s.signature.clone()),
                    ("interface_hash", Json::Str(ids.interface[&s.name].clone())),
                ]),
            )
        })
        .collect();
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let i = lines.iter().position(|s| s == "section symbols").unwrap();
    lines[i + 1] = Json::Obj(symbols).to_jcs();
    lines.join("\n") + "\n"
}
