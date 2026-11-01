//! Admission after external name resolution must repeat the complete checks.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::{json::Json, link};
use support::{sign, source};
const LAYOUT: &str = "\nsection layout\n{\"A_tab\":2,\"H\":6,\"S_cells\":4,\"globals\":{},\"heap_base\":32,\"heap_limit_max\":32,\"interface_regions\":{},\"null_size\":16,\"ptr_width\":32}\n";
const CALLEE: &str = "lproc @f (n:reg[1]) (r:reg[1]) () () ()\n^entry():\nlift.ret n";
fn objects(pc: usize, pin: Option<&str>) -> (String, String, String) {
    let root = format!(
        "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\nregs {{ r:reg[1], pc:reg[{pc}] }}\n^entry():\nr = lift.call @f (x)\npc = lift.label ^done\nqjumpr pc\n^done():\nlift.retval r"
    );
    let full = sign(&(source(&format!("{root}\n{CALLEE}")) + LAYOUT));
    let obj = bennett_vm::object::parse(&full).unwrap();
    let symbols = Json::Obj(
        obj.symbols
            .values()
            .filter(|s| s.name != "f")
            .map(|s| {
                (
                    s.index.to_string(),
                    Json::obj(vec![
                        ("name", Json::Str(s.name.clone())),
                        ("kind", Json::Str(s.kind.clone())),
                        ("signature", s.signature.clone()),
                        ("interface_hash", Json::Str(s.stored_hash.clone())),
                    ]),
                )
            })
            .collect(),
    );
    let mut import = vec![("signature", obj.symbols["f"].signature.clone())];
    if let Some(pin) = pin {
        import.push(("pinned_closure_hash", Json::Str(pin.into())));
    }
    let imports = Json::obj(vec![("f", Json::obj(import))]);
    let mut text = full.replace(&format!("\n{CALLEE}\n"), "\n");
    let start = text.find("section symbols\n").unwrap() + "section symbols\n".len();
    let end = start + text[start..].find('\n').unwrap();
    text.replace_range(start..end, &symbols.to_jcs());
    text = text.replace(
        "\n\nsection functions",
        &format!(
            "\n\nsection imports\n{}\n\nsection functions",
            imports.to_jcs()
        ),
    );
    (sign(&text), sign(&(source(CALLEE) + LAYOUT)), full)
}
#[test]
fn import_signatures_pins_and_duplicate_definitions_are_checked() {
    let (root, provider, full) = objects(3, None);
    let admission = link::admit(&[&root, &provider, &provider], &full).unwrap();
    let (_, ids) = bennett_vm::load(&provider, false).unwrap();
    assert_eq!(
        admission.resolved_imports["f"],
        ids.closure["f"].clone().unwrap()
    );
    let (root, provider, full) = objects(3, Some(ids.closure["f"].as_ref().unwrap()));
    link::admit(&[&root, &provider], &full).unwrap();
    let (root, provider, full) = objects(3, Some(&"0".repeat(64)));
    let e = link::admit(&[&root, &provider], &full).unwrap_err();
    assert!(e.contains("pinned"), "{e}");
}
#[test]
fn resolved_import_rows_recheck_complete_index_widths() {
    let (root, provider, full) = objects(1, None);
    bennett_vm::load(&root, false).unwrap(); // Imported row space is not known yet.
    bennett_vm::load(&provider, false).unwrap();
    let e = link::admit(&[&root, &provider], &full).unwrap_err();
    assert!(e.contains("index space"), "{e}");
}
#[test]
fn link_admission_rejects_rewritten_source_and_conflicting_layout() {
    let (root, provider, full) = objects(3, None);
    let changed = sign(&full.replace("lift.retval r", "lift.retval x"));
    let e = link::admit(&[&root, &provider], &changed).unwrap_err();
    assert!(e.contains("source") || e.contains("interface"), "{e}");
    let conflict = sign(
        &provider
            .replace("\"heap_base\":32", "\"heap_base\":33")
            .replace("\"heap_limit_max\":32", "\"heap_limit_max\":33"),
    );
    let e = link::admit(&[&root, &conflict], &full).unwrap_err();
    assert!(e.contains("layout"), "{e}");
}

#[test]
fn all_five_import_groups_are_matched_even_when_the_import_is_unused() {
    let (root, provider, full) = objects(3, None);
    let root = root.replace("r = lift.call @f (x)", "r = lift.const.i1 0");
    let full = sign(&full.replace("r = lift.call @f (x)", "r = lift.const.i1 0"));
    for (from, to) in [
        ("\"params\":[1]", "\"params\":[2]"),
        ("\"result\":1", "\"result\":2"),
        ("\"tokens\":[]", "\"tokens\":[\"token\"]"),
        ("\"args\":[]", "\"args\":[\"f64\"]"),
        ("\"mem\":[]", "\"mem\":[{\"A\":1,\"W\":8,\"role\":\"in\"}]"),
    ] {
        // Only mutate the import line, preserving the root header signature.
        let mut lines = root.lines().map(str::to_string).collect::<Vec<_>>();
        let i = lines.iter().position(|s| s == "section imports").unwrap();
        lines[i + 1] = lines[i + 1].replace(from, to);
        let changed = sign(&(lines.join("\n") + "\n"));
        let e = link::admit(&[&changed, &provider], &full).unwrap_err();
        assert!(e.contains("import signature mismatch"), "{from}: {e}");
    }
}
