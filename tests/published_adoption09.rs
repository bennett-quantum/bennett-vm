//! Published K85/K93 cases remain the admission and execution oracle.
use bennett_vm::{
    action::{self, QuantumState},
    bigint::BigUint,
    json::{self, Json},
    ref_library::Catalog,
};
use std::path::PathBuf;
fn fixtures() -> PathBuf {
    crate::pins::isa().join("conformance/fixtures/accepted-k93-09")
}
fn read(path: impl AsRef<std::path::Path>) -> Json {
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}
#[test]
fn published_k85_admission_cases() {
    let cases = crate::pins::isa().join("conformance/cases/D");
    let mut failures = vec![];
    let mut checked = 0;
    for entry in std::fs::read_dir(cases).unwrap() {
        let dir = entry.unwrap().path();
        if !dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("D-k85-")
        {
            continue;
        }
        let case = read(dir.join("case.json"));
        let raw = std::fs::read_to_string(dir.join(case.get("object").unwrap().as_str().unwrap()))
            .unwrap();
        let result = bennett_vm::load(&raw, false);
        let expected = case.get("expect").unwrap();
        let accepted = expected.get("exit").unwrap().as_u64() == Some(0);
        if result.is_ok() != accepted
            || (!accepted && !result.as_ref().unwrap_err().contains("§12.2"))
        {
            failures.push(format!(
                "{}: expected accepted={accepted}, got {:?}",
                case.get("id").unwrap().as_str().unwrap(),
                result.err()
            ));
        }
        checked += 1;
    }
    assert_eq!(
        checked, 28,
        "every published K85 admission case participates"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn published_k93_basis_columns_and_inverses() {
    let vectors = read(fixtures().join("k93.json"));
    for vector in vectors.get("vectors").unwrap().as_arr().unwrap() {
        let raw = std::fs::read_to_string(
            fixtures().join(vector.get("object").unwrap().as_str().unwrap()),
        )
        .unwrap();
        let (object, _) = bennett_vm::load(&raw, true).unwrap();
        let mutables = vector.get("mutables").unwrap().as_usize().unwrap();
        for row in vector.get("rows").unwrap().as_arr().unwrap() {
            for adjoint in [false, true] {
                let (before, after) = if adjoint {
                    ("after", "before")
                } else {
                    ("before", "after")
                };
                let mut state = QuantumState::new();
                let bundles = vector
                    .get("widths")
                    .unwrap()
                    .as_arr()
                    .unwrap()
                    .iter()
                    .map(|n| state.alloc_wires(n.as_usize().unwrap()))
                    .collect::<Vec<_>>();
                for (wires, value) in bundles
                    .iter()
                    .zip(row.get(before).unwrap().as_arr().unwrap())
                {
                    state.prepare(wires, &BigUint::from_u64(value.as_u64().unwrap()));
                }
                action::execute_action_inner(
                    &object,
                    &mut state,
                    "a",
                    &bundles[..mutables],
                    &bundles[mutables..],
                    &[],
                    &[],
                    adjoint,
                    &[],
                )
                .unwrap();
                let mut expected = QuantumState::new();
                for (wires, value) in bundles
                    .iter()
                    .zip(row.get(after).unwrap().as_arr().unwrap())
                {
                    let target = expected.alloc_wires(wires.len());
                    expected.prepare(&target, &BigUint::from_u64(value.as_u64().unwrap()));
                }
                assert!(
                    state.exact_equals(&expected),
                    "{}: complete basis column with phase +1, adjoint={adjoint}",
                    vector.get("name").unwrap().as_str().unwrap()
                );
            }
        }
    }
}
#[test]
fn published_k93_mixed_source_operand_order() {
    let catalog = Catalog::parse(
        &std::fs::read_to_string(crate::pins::isa().join("spec/ref-0.9.json")).unwrap(),
    )
    .unwrap();
    for (name, expected) in [
        ("mixed-sb", vec!["s[0]", "b[0]", "b[1]", "t[0]"]),
        ("mixed-bs", vec!["b[0]", "b[1]", "s[0]", "t[0]"]),
        ("predicate", vec!["s[0]", "b[0]", "b[1]", "t[0]"]),
    ] {
        let raw = std::fs::read_to_string(fixtures().join(format!("{name}.bisa"))).unwrap();
        let (object, _) = bennett_vm::load(&raw, true).unwrap();
        for adjoint in [false, true] {
            let leaves = catalog
                .lower_action(&object, "a", &Json::Arr(vec![]), adjoint, 0)
                .unwrap();
            let gate = leaves.iter().find(|l| l.operation == "mcx").unwrap();
            assert_eq!(gate.wires, expected, "{name} adjoint={adjoint}");
            assert_eq!(
                gate.controls,
                if name == "predicate" {
                    vec!["r[0]"]
                } else {
                    vec![]
                }
            );
        }
    }
}

#[test]
fn published_k93_negative_operands_remain_rejected() {
    let cases = crate::pins::isa().join("conformance/cases/F");
    let mut checked = 0;
    for entry in std::fs::read_dir(cases).unwrap() {
        let dir = entry.unwrap().path();
        if !dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("F-k93-")
        {
            continue;
        }
        let case = read(dir.join("case.json"));
        let raw = std::fs::read_to_string(dir.join(case.get("object").unwrap().as_str().unwrap()))
            .unwrap();
        let expected = case
            .get("expect")
            .unwrap()
            .get("exit")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(
            bennett_vm::load(&raw, false).is_ok(),
            expected == 0,
            "{}",
            case.get("id").unwrap().as_str().unwrap()
        );
        checked += 1;
    }
    assert_eq!(checked, 9);
}
#[test]
fn published_k93_target_request_keeps_flattened_arity_and_control_order() {
    let raw = std::fs::read_to_string(fixtures().join("predicate.bisa")).unwrap();
    let (object, _) = bennett_vm::load(&raw, false).unwrap();
    let mut target =
        read(crate::pins::isa().join("conformance/cases/E/E-matching-first-entry/target.json"));
    let Json::Obj(ref mut fields) = target else {
        panic!("target object");
    };
    fields.insert("gates".into(), json::parse(r#"[{"classes":["*"],"controls":{"max":"unbounded","min":0},"cost":{"time":1},"epsilon":0,"op":"mcx"}]"#).unwrap());
    let target = bennett_vm::target::Target::parse(&target.to_jcs()).unwrap();
    for adjoint in [false, true] {
        let mut state = QuantumState::new();
        state.set_target(target.clone()).unwrap();
        let t = state.target_alloc(1, "fast").unwrap();
        let r = state.target_alloc(1, "fast").unwrap();
        let s = state.target_alloc(1, "fast").unwrap();
        let b = state.target_alloc(2, "fast").unwrap();
        for (wire, value) in [(&r, 1), (&s, 1), (&b, 3)] {
            state.prepare(wire, &BigUint::from_u64(value));
        }
        action::execute_action_inner(
            &object,
            &mut state,
            "a",
            std::slice::from_ref(&t),
            &[r.clone(), s.clone(), b.clone()],
            &[],
            &[],
            adjoint,
            &[],
        )
        .unwrap();
        assert_eq!(state.constant_value(&t), Some(BigUint::from_u64(1)));
        assert_eq!(state.emitted.len(), 1);
        let gate = &state.emitted[0].gate;
        assert_eq!(gate.own_controls, 3);
        assert_eq!(gate.controls, [s, b, r].concat());
        assert_eq!(gate.targets, t);
        assert_eq!(gate.classes, vec!["fast"; 5]);
        assert_eq!(
            state.target_accounting().unwrap().get("time"),
            Some(&Json::Str("1".into()))
        );
    }
}

#[test]
fn published_token_free_universal_adjoint_claim_remains_admissible() {
    let dir = crate::pins::isa().join("conformance/cases/F/F-adjoint-table-published");
    let case = read(dir.join("case.json"));
    let raw =
        std::fs::read_to_string(dir.join(case.get("object").unwrap().as_str().unwrap())).unwrap();
    bennett_vm::load(&raw, false).unwrap();
}

#[test]
fn published_adjoint_nonempty_witness_is_rejected_at_admission() {
    let dir = crate::pins::isa().join("conformance/cases/F/F-adjoint-table-witness-nonempty");
    let case = read(dir.join("case.json"));
    let raw =
        std::fs::read_to_string(dir.join(case.get("object").unwrap().as_str().unwrap())).unwrap();
    let error = bennett_vm::load(&raw, false).unwrap_err();
    assert!(
        error.contains("§12.1") && error.contains("witness"),
        "{error}"
    );
}
#[test]
fn published_k93_entangled_external_reference_and_source_identity() {
    let vectors = read(fixtures().join("k93.json"));
    let raw = std::fs::read_to_string(fixtures().join("witness.bisa")).unwrap();
    let (object, ids) = bennett_vm::load(&raw, false).unwrap();
    assert_eq!(
        ids.object_hash,
        vectors
            .get("witness_object_hash")
            .unwrap()
            .as_str()
            .unwrap()
    );
    assert_eq!(
        ids.closure["a"].as_deref(),
        vectors.get("witness_closure_hash").unwrap().as_str()
    );
    let state = |which: &str| {
        let rows = vectors
            .get("reference_witness")
            .unwrap()
            .get(which)
            .unwrap()
            .as_arr()
            .unwrap();
        let mut state = QuantumState::new();
        state.alloc_wires(5); // p, q, c[0], c[1], untouched reference R
        let values = rows
            .iter()
            .map(|row| {
                let values = row.as_arr().unwrap();
                let key = values
                    .iter()
                    .zip([0, 1, 2, 4])
                    .fold(0, |key, (value, shift)| {
                        key | (value.as_u64().unwrap() << shift)
                    });
                (
                    BigUint::from_u64(key),
                    (std::f64::consts::FRAC_1_SQRT_2, 0.),
                )
            })
            .collect();
        state.set_state(values);
        state
    };
    let before = state("before");
    let mut actual = before.clone();
    action::execute_action_inner(
        &object,
        &mut actual,
        "a",
        &[vec![0], vec![1]],
        &[vec![2, 3]],
        &[],
        &[],
        false,
        &[],
    )
    .unwrap();
    assert!(actual.exact_equals(&state("after")));
    action::execute_action_inner(
        &object,
        &mut actual,
        "a",
        &[vec![0], vec![1]],
        &[vec![2, 3]],
        &[],
        &[],
        true,
        &[],
    )
    .unwrap();
    assert!(actual.exact_equals(&before));
}
