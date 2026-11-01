//! Published ISA 0.9 admission and byte-exact identity witnesses.
use bennett_vm::{identity, object};
fn examples() -> Vec<String> {
    let spec = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../bennett-isa-spec/spec/Bennett-ISA-Spec-0.9.md"
    ))
    .unwrap();
    spec.split("```\n")
        .filter_map(|part| {
            part.find("bennett-object isa=0.9")
                .map(|p| part[p..].to_string())
        })
        .filter(|s| s.starts_with("bennett-object") && !s.contains("```"))
        .collect()
}
#[test]
fn published_09_inc8_and_recursive_sum_hashes() {
    let xs = examples();
    assert_eq!(xs.len(), 10);
    for text in &xs[..2] {
        bennett_vm::load(text, false).unwrap();
    }
    let text = &xs[0];
    assert!(object::parse(&text.replacen("object=0.9", "object=0.8", 1)).is_err());
    assert!(bennett_vm::load(&text.replace("\"abi\":\"0.9\"", "\"abi\":\"0.8\""), true).is_err());
}
#[test]
fn layout_paths_are_injective_and_select_original_keys() {
    // Two dotted renderings collide, and values sort in the opposite order.
    let text = "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{}\n\nsection layout\n{\"A_tab\":2,\"H\":6,\"S_cells\":4,\"globals\":{},\"heap_base\":32,\"heap_limit_max\":32,\"interface_regions\":{\"a\":{\"c.db\":{\"base\":17}},\"a.c\":{\"db\":{\"base\":16}}},\"null_size\":16,\"ptr_width\":32}\n";
    let obj = object::parse(text).unwrap();
    let leaves = identity::layout_leaves(&obj);
    assert_eq!(
        leaves
            .iter()
            .filter(|(p, _)| p.contains("interface_regions"))
            .collect::<Vec<_>>(),
        vec![
            &(
                r#"["interface_regions","a","c.db"]"#.into(),
                r#"{"base":17}"#.into()
            ),
            &(
                r#"["interface_regions","a.c","db"]"#.into(),
                r#"{"base":16}"#.into()
            ),
        ]
    );
    assert!(leaves.windows(2).all(|p| p[0].0 < p[1].0));
}
