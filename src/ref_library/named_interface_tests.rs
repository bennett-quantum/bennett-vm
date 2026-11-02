//! Published K82 binding witnesses, independent of the producer implementation.
use super::*;
fn vectors() -> Json {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bennett-isa-spec/conformance/fixtures/accepted-k82-09/k82.json");
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}
fn bundles(value: &Json) -> Bindings {
    let mut bindings = Bindings::new();
    for (name, width) in value.as_obj().unwrap() {
        bindings.wire(name, width.as_usize().unwrap());
    }
    bindings
}
fn normalized(leaves: &[Leaf]) -> Json {
    let mut allocations = BTreeMap::new();
    let mut wires = BTreeMap::new();
    Json::Arr(
        leaves
            .iter()
            .map(|leaf| {
                let operands = if let Some(id) = &leaf.allocation {
                    let n = allocations.len();
                    let name = allocations
                        .entry(id.clone())
                        .or_insert_with(|| format!("#{n}"))
                        .clone();
                    for (bit, wire) in leaf.wires.iter().enumerate() {
                        wires.insert(wire.clone(), format!("{name}[{bit}]"));
                    }
                    vec![Json::Str(name)]
                } else {
                    assert!(leaf.controls.is_empty());
                    leaf.wires
                        .iter()
                        .map(|w| Json::Str(wires.get(w).unwrap_or(w).clone()))
                        .collect()
                };
                Json::Arr(vec![Json::Str(leaf.operation.clone()), Json::Arr(operands)])
            })
            .collect(),
    )
}
#[test]
fn published_k82_binding_cases() {
    let vectors = vectors();
    let cases = arr(field(&vectors, "cases").unwrap()).unwrap();
    assert_eq!(cases.len(), 16);
    let mut failures = vec![];
    for case in cases {
        // Custom finite witnesses use the same private evaluator; the public
        // Catalog::parse still authenticates the unchanged published catalog.
        let doc = Json::obj(vec![("recipes", field(case, "recipes").unwrap().clone())]);
        let interfaces = named_interface::collect(&doc).unwrap();
        let catalog = Catalog { doc, interfaces };
        for recipe in arr(field(case, "recipes").unwrap()).unwrap() {
            validate_formals(arr(field(recipe, "parameters").unwrap()).unwrap()).unwrap();
            syntax(field(recipe, "body").unwrap()).unwrap();
        }
        let bindings = bundles(field(case, "bundles").unwrap());
        let result = catalog.expand(
            string(field(case, "root").unwrap()).unwrap(),
            arr(field(case, "args").unwrap()).unwrap(),
            &bindings,
        );
        let expected = field(case, "expect").unwrap();
        let pass = match (&result, expected.as_str()) {
            (Err(_), Some("reject")) => true,
            (Ok(leaves), None) => normalized(leaves) == *expected,
            _ => false,
        };
        if !pass {
            failures.push(format!(
                "{}: result={result:?}, expected={expected:?}",
                string(field(case, "id").unwrap()).unwrap()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn published_k82_edge_digests() {
    let catalog = Catalog::published().unwrap();
    let vectors = vectors();
    let edges = arr(field(&vectors, "edges").unwrap()).unwrap();
    assert_eq!(edges.len(), 20);
    let mut count = 0;
    for edge in edges {
        let caller = string(field(edge, "caller").unwrap()).unwrap();
        for case in arr(field(edge, "instances").unwrap()).unwrap() {
            let leaves = catalog
                .expand(
                    caller,
                    arr(field(case, "args").unwrap()).unwrap(),
                    &bundles(field(case, "bundles").unwrap()),
                )
                .unwrap();
            assert_eq!(
                leaves.len(),
                field(case, "leaves").unwrap().as_usize().unwrap(),
                "{caller}"
            );
            assert_eq!(
                crate::sha256::sha256_hex(normalized(&leaves).to_jcs().as_bytes()),
                string(field(case, "sha256").unwrap()).unwrap(),
                "{caller}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 86);
}
#[test]
fn published_k82_interfaces_and_explicit_root_map() {
    let catalog = Catalog::published().unwrap();
    let vectors = vectors();
    let expected = field(&vectors, "interfaces").unwrap().as_obj().unwrap();
    assert_eq!(catalog.interfaces.len(), 54);
    assert_eq!(catalog.interfaces.len(), expected.len());
    for (name, interface) in &catalog.interfaces {
        let mut actual = interface.clone();
        actual.sort();
        assert_eq!(
            Json::Arr(actual.into_iter().map(Json::Str).collect()),
            expected[name],
            "{name}"
        );
    }
    let mut env = Bindings::new();
    env.wire("a", 1);
    assert_eq!(catalog.expand("gate.x", &[], &env).unwrap().len(), 1);
    env.wire("unrelated", 1);
    assert!(
        catalog
            .expand("gate.x", &[], &env)
            .unwrap_err()
            .contains("interface")
    );
}
