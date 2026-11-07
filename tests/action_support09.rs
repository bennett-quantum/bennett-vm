//! Published K83 vectors: support coordinates, identities and exact inverses.
use bennett_vm::{
    json::Json,
    ref_library::{Bindings, Catalog, Leaf},
};
use std::{collections::BTreeMap, path::PathBuf};

fn fixtures() -> PathBuf {
    crate::pins::isa().join("conformance/fixtures/accepted-k83-09")
}
fn vectors() -> Json {
    bennett_vm::json::parse(&std::fs::read_to_string(fixtures().join("k83.json")).unwrap()).unwrap()
}
fn trace(leaves: &[Leaf], all: bool) -> Json {
    let mut owners = BTreeMap::new();
    let mut wires = BTreeMap::new();
    for l in leaves.iter().filter(|l| l.operation == "alloc") {
        let name = format!("a{}", owners.len());
        owners.insert(l.allocation.clone().unwrap(), name.clone());
        for (bit, wire) in l.wires.iter().enumerate() {
            wires.insert(wire.clone(), (name.clone(), bit));
        }
    }
    Json::Arr(
        leaves
            .iter()
            .filter_map(|l| {
                let a = l.address.as_ref().unwrap();
                let p = a.path.last().unwrap();
                if !all
                    && !p.point.starts_with('$')
                    && l.operation != "alloc"
                    && l.operation != "release"
                {
                    return None;
                }
                let (owner, bit) = if let Some(id) = &l.allocation {
                    (owners[id].clone(), Json::Null)
                } else {
                    let wire = l.wires.last().unwrap();
                    let (owner, bit) = wires.get(wire).cloned().unwrap_or_else(|| {
                        let (name, bit) = wire.split_once('[').unwrap();
                        (name.into(), bit.trim_end_matches(']').parse().unwrap())
                    });
                    (owner, Json::u64(bit as u64))
                };
                let indices =
                    |v: &[usize]| Json::Arr(v.iter().map(|n| Json::u64(*n as u64)).collect());
                Some(Json::Arr(vec![
                    Json::Str(l.operation.clone()),
                    Json::Str(p.point.clone()),
                    Json::u64(a.generated as u64),
                    Json::Str(owner),
                    bit,
                    indices(&p.iteration),
                    indices(&p.instance),
                    a.path
                        .iter()
                        .rev()
                        .nth(1)
                        .map_or(Json::Null, |p| indices(&p.iteration)),
                ]))
            })
            .collect(),
    )
}
fn check_basis(leaves: &[Leaf], case: &Json) {
    let bundles = case.get("bundles").unwrap().as_arr().unwrap();
    let width: usize = bundles
        .iter()
        .map(|b| b.as_arr().unwrap()[1].as_usize().unwrap())
        .sum();
    for basis in 0..1usize << width {
        let mut state = BTreeMap::new();
        let mut offset = 0;
        for bundle in bundles {
            let b = bundle.as_arr().unwrap();
            for bit in 0..b[1].as_usize().unwrap() {
                state.insert(
                    format!("{}[{bit}]", b[0].as_str().unwrap()),
                    basis & (1 << offset) != 0,
                );
                offset += 1;
            }
        }
        let mut expected = state.clone();
        // The published gap/arms functions flip t; the four image functions
        // have no interface mutation, for either external control value.
        if let Some(t) = expected.get_mut("t[0]") {
            *t ^= true;
        }
        for leaf in leaves {
            match leaf.operation.as_str() {
                "alloc" => {
                    for wire in &leaf.wires {
                        assert!(state.insert(wire.clone(), false).is_none());
                    }
                }
                "release" => {
                    for wire in &leaf.wires {
                        assert_eq!(state.remove(wire), Some(false));
                    }
                }
                "x" | "cx" | "ccx" | "mcx" => {
                    let (target, controls) = leaf.wires.split_last().unwrap();
                    if controls.iter().chain(&leaf.controls).all(|w| state[w]) {
                        *state.get_mut(target).unwrap() ^= true;
                    }
                }
                op => panic!("unexpected permutation leaf {op}"),
            }
        }
        assert_eq!(state, expected);
    }
}

#[test]
fn published_action_support_vectors() {
    let catalog = Catalog::published().unwrap();
    for case in vectors().get("cases").unwrap().as_arr().unwrap() {
        let name = case.get("name").unwrap().as_str().unwrap();
        let text =
            std::fs::read_to_string(fixtures().join(case.get("object").unwrap().as_str().unwrap()))
                .unwrap();
        let (obj, ids) = bennett_vm::load(&text, false).unwrap();
        assert_eq!(
            bennett_vm::sha256::sha256_hex(text.as_bytes()),
            case.get("sha256").unwrap().as_str().unwrap(),
            "{name}"
        );
        let entry = case.get("entry").unwrap().as_str().unwrap();
        for adjoint in [false, true] {
            let leaves = catalog
                .lower_action(&obj, entry, &Json::Arr(vec![]), adjoint, 0)
                .unwrap();
            let mut expected = case.get("trace").unwrap().clone();
            if adjoint && name == "repeated-image-81" {
                // K62 traverses source qloop visits in reverse while each
                // callee still creates/restores its image in forward order.
                let rows = expected.as_arr().unwrap();
                expected = Json::Arr(
                    rows.chunks(6)
                        .rev()
                        .enumerate()
                        .flat_map(|(n, group)| {
                            group.iter().map(move |row| {
                                let mut row = row.as_arr().unwrap().clone();
                                row[3] = Json::Str(format!("a{n}"));
                                Json::Arr(row)
                            })
                        })
                        .collect(),
                );
            }
            assert_eq!(
                trace(
                    &leaves,
                    case.get("complete_trace") == Some(&Json::Bool(true))
                ),
                expected,
                "{name}, source adjoint={adjoint}"
            );
            check_basis(&leaves, case);
            let widths: Vec<_> = leaves
                .iter()
                .filter(|l| l.operation == "alloc")
                .map(|l| l.wires.len())
                .collect();
            let expected: Vec<_> = case
                .get("allocation_widths")
                .unwrap()
                .as_obj()
                .unwrap()
                .values()
                .map(|n| n.as_usize().unwrap())
                .collect();
            assert_eq!(widths, expected, "{name}");
        }
        // Raw library inversion keeps the forward support coordinates and
        // allocation identities; source-adjoint formation is checked above.
        let mut env = Bindings::new();
        env.link(&obj).unwrap();
        for bundle in case.get("bundles").unwrap().as_arr().unwrap() {
            let b = bundle.as_arr().unwrap();
            env.wire(b[0].as_str().unwrap(), b[1].as_usize().unwrap());
        }
        let call = Json::obj(vec![
            ("invoke", Json::Str("action_call".into())),
            (
                "args",
                Json::Arr(vec![
                    Json::Str(ids.closure[entry].clone().unwrap()),
                    case.get("actuals").unwrap().clone(),
                ]),
            ),
        ]);
        env.body("forward", Json::Arr(vec![call.clone()])).unwrap();
        env.body(
            "backward",
            Json::Arr(vec![Json::obj(vec![
                ("inverse", Json::Bool(true)),
                ("body", Json::Arr(vec![call])),
            ])]),
        )
        .unwrap();
        let forward = catalog
            .expand("adj", &[Json::Str("forward".into())], &env)
            .unwrap();
        let backward = catalog
            .expand("adj", &[Json::Str("backward".into())], &env)
            .unwrap();
        check_basis(&forward, case);
        check_basis(&backward, case);
        for (f, r) in forward.iter().rev().zip(&backward) {
            let mut address = f.address.clone().unwrap();
            address.kind = r.address.as_ref().unwrap().kind;
            assert_eq!(Some(address), r.address, "{name}");
            assert_eq!(f.wires, r.wires, "{name}");
            assert_eq!(f.allocation, r.allocation, "{name}");
            assert_eq!(f.controls, r.controls, "{name}");
            assert_eq!(f.invocation, r.invocation, "{name}");
            let inverse = match f.operation.as_str() {
                "alloc" => "release",
                "release" => "alloc",
                op => op,
            };
            assert_eq!(inverse, r.operation, "{name}");
        }
        assert_eq!(forward.len(), backward.len(), "{name}");
    }
}

#[test]
fn published_action_support_schedule_vectors() {
    for case in vectors().get("schedule_cases").unwrap().as_arr().unwrap() {
        let name = case.get("name").unwrap().as_str().unwrap();
        let text =
            std::fs::read_to_string(fixtures().join(case.get("object").unwrap().as_str().unwrap()))
                .unwrap();
        assert_eq!(
            bennett_vm::sha256::sha256_hex(text.as_bytes()),
            case.get("sha256").unwrap().as_str().unwrap(),
            "{name}"
        );
        let result = bennett_vm::load(&text, false);
        if case.get("accepted") == Some(&Json::Bool(true)) {
            result.unwrap();
        } else {
            assert!(result.unwrap_err().contains("§11"), "{name}");
        }
    }
}
