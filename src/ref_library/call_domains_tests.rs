//! K63's domain is constructor traversal, including calls with no leaves.
use super::*;
mod support {
    use crate as bennett_vm;
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/source09.rs"
    ));
}

fn j(text: &str) -> Json {
    json::parse(text).unwrap()
}
fn setup() -> (Catalog, Bindings, crate::identity::Identities) {
    let text = support::source(
        "action @empty (w:wire[1] mut) () () () ()\nregs { } bounds {0}\n^entry():\nqloop 0 {\nx w\n}\naction @leaf (w:wire[1] mut) () () () ()\n^entry():\nx w",
    );
    let (object, ids) = crate::load(&text, false).unwrap();
    let mut bindings = Bindings::new();
    bindings.link(&object).unwrap();
    for name in ["x", "y", "p"] {
        bindings.wire(name, 1);
    }
    (Catalog::published().unwrap(), bindings, ids)
}
fn call(hash: &str, wire: &str) -> Json {
    Json::obj(vec![
        ("invoke", Json::Str("action_call".into())),
        (
            "args",
            Json::Arr(vec![
                Json::Str(hash.into()),
                Json::Arr(vec![Json::Str(wire.into())]),
            ]),
        ),
    ])
}
fn wrap(key: &str, value: Json, body: Json) -> Json {
    Json::obj(vec![(key, value), ("body", body)])
}
fn add(catalog: &mut Catalog, recipe: Json) {
    let Json::Obj(doc) = &mut catalog.doc else {
        panic!()
    };
    let Json::Arr(recipes) = doc.get_mut("recipes").unwrap() else {
        panic!()
    };
    recipes.push(recipe);
}

#[test]
fn call_domains_cover_the_constructor_grammar_and_empty_callees() {
    let (mut catalog, mut bindings, ids) = setup();
    let empty = ids.closure["empty"].as_ref().unwrap();
    let leaf = ids.closure["leaf"].as_ref().unwrap();
    let e = call(empty, "x");
    let l = call(leaf, "x");
    bindings
        .body("saved", Json::Arr(vec![e.clone(), l.clone()]))
        .unwrap();
    add(
        &mut catalog,
        j(
            r#"{"kind":"visit.calls","parameters":[{"name":"R","type":"records","fields":["n"],"order":["n"]},{"name":"B","type":"body"}],"scratch":[],"body":[{"visit":"R","body":[{"splice":"B"}]}]}"#,
        ),
    );
    add(
        &mut catalog,
        j(
            r#"{"kind":"apply.calls","parameters":[{"name":"T","type":"templates","length":2}],"scratch":[],"body":[{"for":"i","from":0,"to":2,"body":[{"apply":"T","index":"i","args":[]}]}]}"#,
        ),
    );
    add(
        &mut catalog,
        j(
            r#"{"kind":"forward.calls","parameters":[{"name":"S","type":"symbol"},{"name":"A","type":"arguments","symbol":"S"}],"scratch":[],"body":[{"call":"S","args":"A"},{"call":"S","args":"A"}]}"#,
        ),
    );
    let templates = Json::Arr(vec![
        Json::obj(vec![
            ("parameters", j("[]")),
            ("body", Json::Arr(vec![e.clone()])),
        ]),
        Json::obj(vec![
            ("parameters", j("[]")),
            ("body", Json::Arr(vec![l.clone()])),
        ]),
    ]);
    let rows = j(r#"[{"n":0},{"n":1}]"#);
    let pair = Json::Arr(vec![e.clone(), l.clone()]);
    let cases = vec![
        (
            "direct",
            Json::Arr(vec![e.clone(), l.clone()]),
            vec![empty, leaf],
        ),
        (
            "for",
            j(&format!(
                r#"[{{"for":"i","from":0,"to":3,"body":[{}]}}]"#,
                e.to_jcs()
            )),
            vec![empty; 3],
        ),
        (
            "nested-for",
            j(&format!(
                r#"[{{"for":"i","from":0,"to":2,"body":[{{"for":"j","from":0,"to":3,"body":[{}]}}]}}]"#,
                e.to_jcs()
            )),
            vec![empty; 6],
        ),
        (
            "zero-for",
            j(&format!(
                r#"[{{"for":"i","from":0,"to":0,"body":[{}]}}]"#,
                e.to_jcs()
            )),
            vec![],
        ),
        (
            "each",
            j(&format!(
                r#"[{{"each":"w","in":["x","y"],"body":[{}]}}]"#,
                call(empty, "w").to_jcs()
            )),
            vec![empty; 2],
        ),
        (
            "under",
            Json::Arr(vec![wrap("under", Json::Str("p".into()), pair.clone())]),
            vec![empty, leaf],
        ),
        (
            "inverse",
            Json::Arr(vec![wrap("inverse", Json::Bool(true), pair)]),
            vec![empty, leaf],
        ),
        ("splice", j(r#"[{"splice":"saved"}]"#), vec![empty, leaf]),
        (
            "nested-body",
            j(&format!(
                r#"[{{"invoke":"adj","args":[[{}]]}}]"#,
                e.to_jcs()
            )),
            vec![empty],
        ),
        (
            "arguments-forwarding",
            j(&format!(
                r#"[{{"invoke":"forward.calls","args":["{leaf}",["x"]]}}]"#
            )),
            vec![leaf; 2],
        ),
        (
            "declare",
            j(&format!(
                r#"[{{"declare":"d","width":1,"class":{{"class":"any"}},"state":"clean","body":[{{"acquire":"d"}},{},{},{{"relinquish":"d"}}]}}]"#,
                call(leaf, "d").to_jcs(),
                call(leaf, "d").to_jcs()
            )),
            vec![leaf; 2],
        ),
        (
            "alloc",
            j(&format!(
                r#"[{{"alloc":"d","width":1,"class":"any","state":"clean"}},{},{},{{"release":"d"}}]"#,
                call(leaf, "d").to_jcs(),
                call(leaf, "d").to_jcs()
            )),
            vec![leaf; 2],
        ),
        (
            "visit",
            Json::Arr(vec![Json::obj(vec![
                ("invoke", Json::Str("visit.calls".into())),
                ("args", Json::Arr(vec![rows, Json::Arr(vec![e.clone()])])),
            ])]),
            vec![empty; 2],
        ),
        (
            "apply",
            Json::Arr(vec![Json::obj(vec![
                ("invoke", Json::Str("apply.calls".into())),
                ("args", Json::Arr(vec![templates])),
            ])]),
            vec![empty, leaf],
        ),
        (
            "non-call-leaves",
            j(&format!(
                r#"[{{"gate":"x","wires":["x"]}},{{"image":"","wires":"x"}},{{"classical":"const.i8","point":"event"}},{}]"#,
                e.to_jcs()
            )),
            vec![empty],
        ),
    ];
    for (name, body, expected) in cases {
        bindings.body("B", body).unwrap();
        let (leaves, calls) = catalog
            .expand_with_calls("adj", &[Json::Str("B".into())], &bindings)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            calls.domains()[0]
                .targets
                .iter()
                .map(|t| &t.closure)
                .collect::<Vec<_>>(),
            expected,
            "{name}"
        );
        for leaf in &leaves {
            if let Some(call) = leaf.invocation.first() {
                assert_eq!(&call.closure_hash, expected[call.call_index], "{name}");
            }
        }
        if name == "direct" || name == "inverse" || name == "splice" || name == "apply" {
            assert_eq!(
                leaves.len(),
                1,
                "zero-leaf call disappeared from {name}'s domain"
            );
            assert_eq!(leaves[0].invocation[0].call_index, 1);
        }
    }
}

#[test]
fn invalid_bound_calls_reject_arity_width_hash_and_cycles() {
    let (mut catalog, mut bindings, ids) = setup();
    let leaf = ids.closure["leaf"].as_ref().unwrap();
    add(
        &mut catalog,
        j(
            r#"{"kind":"recursive","parameters":[],"scratch":[],"body":[{"invoke":"recursive","args":[]}]}"#,
        ),
    );
    bindings.wire("wide", 2);
    for body in [
        j(r#"[{"call":"missing","args":[]}]"#),
        Json::Arr(vec![call(&"0".repeat(64), "x")]),
        Json::Arr(vec![call(leaf, "wide")]),
        j(r#"[{"invoke":"recursive","args":[]}]"#),
        j(&format!(
            r#"[{{"invoke":"action_call","args":["{leaf}",[]]}}]"#
        )),
        j(&format!(
            r#"[{{"invoke":"action_call","args":["{leaf}",["x","y"]]}}]"#
        )),
    ] {
        bindings.body("B", body).unwrap();
        assert!(
            catalog
                .expand_with_calls("adj", &[Json::Str("B".into())], &bindings)
                .is_err()
        );
    }
}
