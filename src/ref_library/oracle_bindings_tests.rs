//! K77's context is captured with arguments, outside the positional vector.
use super::*;

fn j(text: &str) -> Json {
    json::parse(text).unwrap()
}

fn context(steps: u64) -> Json {
    j(&format!(r#"{{"token":["{steps}","0","0","0"]}}"#))
}

fn source_bindings(context: Option<Json>) -> (Catalog, Bindings, String) {
    // This test forms a 0.9 object from the published inc8 source, then computes
    // its identities locally; the old stored interface hash is not an identity
    // supplied to the registry.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bennett-isa-spec/reference-interpreter/tests/fixtures/inc8.bisa");
    let text = std::fs::read_to_string(path).unwrap().replace("0.8", "0.9");
    let (object, ids) = crate::load(&text, true).unwrap();
    let mut bindings = Bindings::new();
    bindings.link(&object).unwrap();
    bindings.wire("x", 8);
    bindings.wire("t", 8);
    bindings.wire("f", 4);
    bindings.oracle_invocation = context;
    (
        Catalog::published().unwrap(),
        bindings,
        ids.closure["inc8"].clone().unwrap(),
    )
}

fn captured(steps: u64) -> (Catalog, Bindings) {
    let (catalog, source, hash) = source_bindings(Some(context(steps)));
    let formals =
        arr(field(catalog.recipe("oracle_call").unwrap(), "parameters").unwrap()).unwrap();
    let bindings = bind_formals(
        formals,
        &[Json::Str(hash), j(r#"["x","t","f"]"#)],
        &source,
        &source,
    )
    .unwrap();
    (catalog, bindings)
}

fn forwarding(adapter: &str) {
    for (steps, site) in [
        (2, Some(context(3))),
        (3, Some(context(2))),
        (2, None),
        (2, Some(j(r#"{"token":["bad","0","0","0"]}"#))),
    ] {
        let (catalog, mut bindings) = captured(steps);
        bindings.oracle_invocation = site.clone();
        let call = j(r#"[{"call":"S","args":"A"}]"#);
        let body = match adapter {
            "call" => call,
            "invoke" => j(r#"[{"invoke":"oracle_call","args":["S","A"]}]"#),
            "splice" => {
                bindings.body("C", call).unwrap();
                j(r#"[{"splice":"C"}]"#)
            }
            "apply" => j(
                r#"[{"invoke":"qregion","args":[1,1,"",[{"parameters":[{"name":"cell","type":"wire","width":1}],"body":[{"call":"S","args":"A"}]}],[]]}]"#,
            ),
            _ => panic!("unknown adapter"),
        };
        bindings.body("B", body).unwrap();
        let leaves = catalog.expand("adj", &[j(r#""B""#)], &bindings).unwrap();
        let actual = leaves.iter().filter_map(|l| l.address.as_ref()?.step).max();
        assert_eq!(
            actual,
            Some(steps as usize),
            "{adapter}: captured token must determine the frame domain"
        );
        assert_eq!(
            bindings.oracle_invocation, site,
            "nested call changed its caller context"
        );
    }
}

#[test]
fn k77_arguments_capture_through_call() {
    forwarding("call");
}
#[test]
fn k77_arguments_capture_through_invoke() {
    forwarding("invoke");
}
#[test]
fn k77_arguments_capture_through_splice() {
    forwarding("splice");
}
#[test]
fn k77_arguments_capture_through_apply() {
    forwarding("apply");
}

#[test]
fn k77_argument_capture_validates_context_before_expansion() {
    for invalid in [
        None,
        Some(j("[]")),
        Some(j(r#"{"token":["2","0","0"]}"#)),
        Some(j(r#"{"token":[2,"0","0","0"]}"#)),
        Some(j(r#"{"token":["02","0","0","0"]}"#)),
        Some(j(r#"{"token":["2","0","0","0"],"extra":0}"#)),
        Some(j(r#"{"token":["2","1","0","0"]}"#)),
        Some(j(r#"{"token":["18446744073709551616","0","0","0"]}"#)),
    ] {
        let (catalog, source, hash) = source_bindings(invalid.clone());
        let formals =
            arr(field(catalog.recipe("oracle_call").unwrap(), "parameters").unwrap()).unwrap();
        assert!(
            bind_formals(
                formals,
                &[Json::Str(hash), j(r#"["x","t","f"]"#)],
                &source,
                &source
            )
            .is_err(),
            "invalid oracle_invocation captured: {invalid:?}"
        );
    }
}

fn two_tokens(inverse: bool) -> Vec<Leaf> {
    let (catalog, mut bindings) = captured(2);
    let (_, enough) = captured(3);
    bindings
        .values
        .insert("Enough".into(), enough.values["A"].clone());
    bindings.oracle_invocation = Some(context(1));
    bindings.wire("c", 1);
    bindings.add_controls(vec!["c[0]".into()]);
    let body = j(r#"[{"invoke":"oracle_call","args":["S","A"]},{"call":"S","args":"Enough"}]"#);
    let body = if inverse {
        Json::Arr(vec![Json::obj(vec![
            ("inverse", Json::Bool(true)),
            ("body", body),
        ])])
    } else {
        body
    };
    bindings.body("B", body).unwrap();
    catalog.expand("adj", &[j(r#""B""#)], &bindings).unwrap()
}

fn replay(leaves: &[Leaf], mut state: BTreeMap<String, bool>) -> BTreeMap<String, bool> {
    for leaf in leaves {
        match leaf.operation.as_str() {
            "alloc" => {
                for wire in &leaf.wires {
                    assert!(state.insert(wire.clone(), false).is_none());
                }
            }
            "release" => {
                for wire in &leaf.wires {
                    assert_eq!(state.remove(wire), Some(false), "dirty {wire}");
                }
            }
            "x" | "cx" | "ccx" | "mcx" => {
                let (target, controls) = leaf.wires.split_last().unwrap();
                if controls
                    .iter()
                    .chain(&leaf.controls)
                    .all(|wire| state[wire])
                {
                    *state.get_mut(target).unwrap() ^= true;
                }
            }
            "swap" => {
                if leaf.controls.iter().all(|wire| state[wire]) {
                    let (a, b) = (state[&leaf.wires[0]], state[&leaf.wires[1]]);
                    state.insert(leaf.wires[0].clone(), b);
                    state.insert(leaf.wires[1].clone(), a);
                }
            }
            operation => panic!("unexpected operation {operation}"),
        }
    }
    state
}

#[test]
fn k77_two_token_witness_keeps_frames_forward_occurrences_and_complete_inverse() {
    let forward = two_tokens(false);
    let inverse = two_tokens(true);
    for (leaves, order) in [(&forward, vec![0, 1]), (&inverse, vec![1, 0])] {
        let mut seen = vec![];
        let mut steps = BTreeMap::<usize, usize>::new();
        let mut allocations = BTreeSet::new();
        for leaf in leaves {
            if leaf.operation == "alloc" {
                assert!(allocations.insert(leaf.allocation.clone().unwrap()));
            }
            let call = leaf.invocation.first().unwrap();
            if seen.last() != Some(&call.call_index) {
                seen.push(call.call_index);
            }
            if let Some(step) = leaf.address.as_ref().unwrap().step {
                let max = steps.entry(call.call_index).or_default();
                *max = (*max).max(step);
            }
        }
        assert_eq!(seen, order);
        assert_eq!(steps, [(0, 2), (1, 3)].into_iter().collect());
    }
    assert_eq!(forward.len(), inverse.len());
    for (f, r) in forward.iter().rev().zip(&inverse) {
        let mut address = f.address.clone().unwrap();
        address.kind = r.address.as_ref().unwrap().kind;
        assert_eq!(Some(address), r.address);
        assert_eq!(f.ordinal, r.ordinal);
        assert_eq!(f.invocation[0].call_index, r.invocation[0].call_index);
        assert_eq!(f.allocation, r.allocation);
    }
    // K77's published inc8 witness, over every input, both control values and
    // nonzero target/fault seeds. Clean release checks every private bit.
    for x in 0..256 {
        for control in [false, true] {
            let mut initial = BTreeMap::from([("c[0]".into(), control)]);
            for (name, width, value) in [("x", 8, x), ("t", 8, 0xa5), ("f", 4, 10)] {
                for bit in 0..width {
                    initial.insert(format!("{name}[{bit}]"), value & (1 << bit) != 0);
                }
            }
            let mut expected = initial.clone();
            if control {
                for bit in 0..8 {
                    *expected.get_mut(&format!("t[{bit}]")).unwrap() ^=
                        ((x + 1) & 255) & (1 << bit) != 0;
                }
                *expected.get_mut("f[0]").unwrap() ^= true;
            }
            let output = replay(&forward, initial.clone());
            assert_eq!(output, expected, "input {x}, control {control}");
            assert_eq!(replay(&inverse, output), initial);
        }
    }
}
