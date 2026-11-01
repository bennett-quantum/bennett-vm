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

fn bound_source() -> String {
    let text = examples()[1].clone();
    text.replace(
        "(db:region[4,8] in) ()",
        "(db:region[4,8] in) (tk:token, a:i8, b:i8)",
    )
    .replace(
        "(r:reg[8]) () () ()",
        "(r:reg[8]) (tk:token token) (a:i8 classical) (db:region[4,8] in)",
    )
    .replace("@sum (n, acc, base)", "@sum (n, acc, base) (tk) (a) (db)")
    .replace(
        "@sum (n2, acc2, base)",
        "@sum (n2, acc2, base) (tk) (a) (db)",
    )
}
fn resign(text: &str) -> String {
    let mut obj = object::parse(text).unwrap();
    let names = obj.symbols.keys().cloned().collect::<Vec<_>>();
    for name in &names {
        obj.symbols.get_mut(name).unwrap().signature =
            bennett_vm::validate::derived_signature(&obj, name).unwrap();
    }
    let entries = names
        .iter()
        .map(|n| {
            let s = &obj.symbols[n];
            (
                s.index.to_string(),
                bennett_vm::json::Json::obj(vec![
                    ("name", bennett_vm::json::Json::Str(n.clone())),
                    ("kind", bennett_vm::json::Json::Str(s.kind.clone())),
                    ("signature", s.signature.clone()),
                    (
                        "interface_hash",
                        bennett_vm::json::Json::Str(s.stored_hash.clone()),
                    ),
                ]),
            )
        })
        .collect();
    let line = bennett_vm::json::Json::Obj(entries).to_jcs();
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let pos = lines.iter().position(|s| s == "section symbols").unwrap();
    lines[pos + 1] = line;
    lines.join("\n") + "\n"
}
#[test]
fn typed_lproc_binding_groups_and_immutable_origins() {
    let text = resign(&bound_source());
    bennett_vm::load(&text, true).unwrap();
    for (from, to) in [
        ("tk:token token", "tk:token classical"),
        ("a:i8 classical", "a:i8 token"),
        ("(tk) (a) (db)", "(tk) (acc) (db)"),
        ("(tk) (a) (db)", "(tk) (1) (db)"),
        ("(tk) (a) (db)", "(a) (a) (db)"),
        ("(tk) (a) (db)", "(tk) (a) ()"),
        ("(tk) (a) (db)", "()"),
        ("n2 = lift.sub n one", "a = lift.sub n one"),
    ] {
        assert!(
            bennett_vm::load(&resign(&text.replace(from, to)), true).is_err(),
            "accepted {from} -> {to}"
        );
    }
}
#[test]
fn fixed_instances_close_recursive_forwarding_by_position() {
    let text = resign(&bound_source().replace(
        "lift.retval r",
        "r = lift.call @sum (n, acc, base) (tk) (b) (db)\nlift.retval r",
    ));
    let (obj, _) = bennett_vm::load(&text, true).unwrap();
    let instances = bennett_vm::instances::for_oracle(&obj, "sum_db").unwrap();
    assert_eq!(
        instances
            .iter()
            .map(|i| (&*i.symbol, i.key.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("sum_db", vec![]),
            ("sum", vec![0, 1, 0]),
            ("sum", vec![0, 2, 0])
        ]
    );
}

#[test]
fn all_published_09_objects_verify_stored_hashes() {
    for (i, text) in examples().iter().enumerate() {
        bennett_vm::load(text, false).unwrap_or_else(|e| panic!("example {}: {e}", i + 1));
    }
}

#[test]
fn published_interfaces_reject_wrong_roles_types_and_widths() {
    let xs = examples();
    for (index, from, to) in [
        (2, "regs {  }", "regs { ghost:reg[1] }"),
        (3, "measure.i3 q2", "measure.i129 q2"),
        (3, "qreg.from_id id2", "qreg.from_id p"),
        (
            5,
            "budget.mint @inc8 s zero zero zero",
            "budget.mint @inc8 zero zero zero n",
        ),
        (
            6,
            "qinplace @g @ginv tk x (y) () (a)",
            "qinplace @g @ginv tk x",
        ),
        (9, "(tk) (a) (db)", "(tk) (x) (db)"),
        (
            9,
            "(r:reg[2]) (tk:token token)",
            "(r:reg[3]) (tk:token token)",
        ),
    ] {
        assert!(xs[index].contains(from), "missing test trigger {from}");
        assert!(
            bennett_vm::load(&resign(&xs[index].replace(from, to)), true).is_err(),
            "accepted {from} -> {to}"
        );
    }
}

#[test]
fn lproc_fixed_values_execute_and_steps_reverse_complete_state() {
    use bennett_vm::{
        bigint::BigUint as B,
        value::{Budget, Value},
    };
    let text = examples()[9].clone();
    let start = text.find("lproc @worker").unwrap();
    let end = text.find("oracle @plus_db").unwrap();
    let worker = text[start..end]
        .replace("tk", "budget")
        .replace("(a:i2 classical)", "(arg:i2 classical)")
        .replace("(a)", "(arg)")
        .replace("(tk, a)", "(budget, arg)")
        .replace("(budget, a)", "(budget, arg)")
        .replace("db:", "store:")
        .replace("(db)", "(store)");
    let text = format!("{}{}{}", &text[..start], worker, &text[end..]);
    let (obj, _) = bennett_vm::load(&resign(&text), true).unwrap();
    let budget = Budget {
        steps: 8,
        heap: 0,
        stack: 8,
        depth: 1,
    };
    for nested_steps in [5, 6] {
        for x in 0..4 {
            for a in 0..4 {
                for d in 0..256 {
                    let args = [
                        Value::Token(Budget {
                            steps: nested_steps,
                            heap: 0,
                            stack: 0,
                            depth: 0,
                        }),
                        Value::int(2, B::from_u64(a)),
                    ];
                    let mem = vec![vec![B::from_u64(d)]];
                    let mut step = bennett_vm::step::Machine::new(
                        &obj,
                        "through",
                        &[B::from_u64(x)],
                        &mem,
                        &args,
                        budget,
                    )
                    .unwrap();
                    let initial = step.state.clone();
                    for _ in 0..8 {
                        let before = step.state.clone();
                        step.forward_step().unwrap();
                        let after = step.state.clone();
                        step.inverse_step().unwrap();
                        assert_eq!(step.state, before);
                        step.forward_step().unwrap();
                        assert_eq!(step.state, after);
                    }
                    let out = step.outcome();
                    let expected = if nested_steps == 6 {
                        (((x + a + d) % 4) ^ 1, 0)
                    } else {
                        (0, 9)
                    };
                    assert_eq!(
                        (out.payload.clone(), out.code.clone()),
                        (B::from_u64(expected.0), B::from_u64(expected.1))
                    );
                    let direct = bennett_vm::oracle::run_oracle(
                        &obj,
                        "through",
                        &[B::from_u64(x)],
                        &mut mem.clone(),
                        &args,
                        budget,
                    )
                    .unwrap();
                    assert_eq!(
                        (direct.payload, direct.code, direct.steps),
                        (out.payload, out.code, out.steps)
                    );
                    for _ in 0..8 {
                        step.inverse_step().unwrap();
                    }
                    assert_eq!(step.state, initial);
                }
            }
        }
    }
}

#[test]
fn separate_instance_locations_and_canonical_call_ranges() {
    use bennett_vm::{
        bigint::BigUint as B,
        value::{Budget, Value},
    };
    let text = bound_source().replace("lift.retval r", "acc = lift.mov r\nr = lift.call @sum (n, acc, base) (tk) (b) (db)\nr = lift.add acc r\nlift.retval r");
    let start = text.find("regs { zp:").unwrap();
    let end = text.find("\n\nsection data").unwrap();
    let text = format!(
        "{}regs {{  }}\n^entry():\nr = lift.mov a\nlift.ret r{}",
        &text[..start],
        &text[end..]
    );
    let (obj, _) = bennett_vm::load(&resign(&text), true).unwrap();
    let budget = Budget {
        steps: 11,
        heap: 0,
        stack: 14,
        depth: 1,
    };
    let args = [
        Value::Token(Budget::default()),
        Value::int(8, B::from_u64(3)),
        Value::int(8, B::from_u64(7)),
    ];
    let mut mem = vec![vec![B::zero(); 16]];
    let out = bennett_vm::oracle::run_oracle(&obj, "sum_db", &[B::zero()], &mut mem, &args, budget)
        .unwrap();
    assert_eq!(
        (out.payload, out.code, out.steps),
        (B::from_u64(10), B::zero(), 11)
    );
    let mut m =
        bennett_vm::step::Machine::new(&obj, "sum_db", &[B::zero()], &mem, &args, budget).unwrap();
    let initial = m.state.clone();
    for step in 1..=11 {
        m.forward_step().unwrap();
        if step == 3 {
            assert_eq!(m.state.pc, 7);
        }
        if step == 7 {
            assert_eq!(m.state.pc, 9);
        }
    }
    assert_eq!(m.outcome().payload, B::from_u64(10));
    for _ in 0..11 {
        m.inverse_step().unwrap();
    }
    assert_eq!(m.state, initial);
}
