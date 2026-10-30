//! ISA §5.7 transformation witnesses; reference rows remain observable.
use bennett_vm::{
    jit::{Passes, Program},
    object,
};
use std::path::Path;
fn fixture(name: &str) -> object::Object {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/tests/fixtures")
        .join(name);
    bennett_vm::load(&std::fs::read_to_string(p).unwrap(), false)
        .unwrap()
        .0
}
#[test]
fn transformations_change_private_graph_without_changing_reference_body() {
    let obj = fixture("sum_db.bisa");
    let before = bennett_vm::identity::compute(&obj).unwrap().object_hash;
    let plan = Program::compile(&obj, Passes::all(), false).unwrap();
    assert!(
        plan.stats().inlined_rows > 0,
        "inline lproc bodies, retaining call rows"
    );
    assert!(
        plan.stats().structured_edges > 0,
        "recover direct continuations"
    );
    assert!(
        plan.stats().reindexed_rows > 0,
        "private dispatch layout changes"
    );
    assert_eq!(
        before,
        bennett_vm::identity::compute(&obj).unwrap().object_hash
    );
}
#[test]
fn native_graph_executes_every_row_and_stops_at_callback_boundary() {
    let obj = fixture("inc8.bisa");
    let plan = Program::compile(&obj, Passes::all(), true).unwrap();
    let start = plan.row("inc8", 0, 0).unwrap();
    let mut seen = vec![];
    plan.run(start, |id, ins| {
        seen.push(ins.raw.clone());
        Ok(if seen.len() == 2 { None } else { Some(id + 1) })
    })
    .unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].contains("lift.const"));
    // Errors return through the native ABI without unwinding through generated frames.
    assert_eq!(
        plan.run(start, |_, _| Err("witness".into())).unwrap_err(),
        "witness"
    );
}
#[test]
fn native_integer_kernels_match_width_arithmetic_exhaustively_and_at_boundaries() {
    use bennett_vm::{bigint::BigUint as B, value};
    for op in [
        "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr",
    ] {
        for w in [1, 2, 3, 4, 8, 16, 31, 32, 63, 64] {
            let mask = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
            let values: Vec<u64> = if w <= 4 {
                (0..=mask).collect()
            } else {
                vec![
                    0,
                    1,
                    2,
                    w as u64 - 1,
                    w as u64,
                    w as u64 + 1,
                    mask >> 1,
                    (mask >> 1) + 1,
                    mask,
                ]
            };
            for a in &values {
                for b in &values {
                    let expected =
                        value::binop_reference(op, &B::from_u64(*a), &B::from_u64(*b), w);
                    let actual = bennett_vm::jit::native_binop(op, *a, *b, w).unwrap();
                    assert_eq!(actual, expected.to_u64().unwrap(), "{op} w={w} a={a} b={b}");
                }
            }
        }
    }
}
#[test]
fn simplification_folds_values_but_keeps_each_reference_boundary_and_source() {
    let mut obj = fixture("inc8.bisa");
    let add = &mut obj.functions[0].blocks[0].instrs[1];
    add.operands[0] = "one".into();
    // Keeping raw source intentionally verifies that executor IR is private.
    for host in [false, true] {
        let p = Program::compile(&obj, Passes::all(), host).unwrap();
        assert_eq!(p.stats().simplified_rows, 1);
        let mut rows = vec![];
        p.run(p.row("inc8", 0, 0).unwrap(), |id, ins| {
            rows.push((ins.op.clone(), ins.operands.clone(), ins.raw.clone()));
            Ok(if rows.len() < 3 { Some(id + 1) } else { None })
        })
        .unwrap();
        assert_eq!(rows.len(), 3, "folding cannot change the budget unit");
        assert_eq!(rows[1].0, "lift.const.i8");
        assert_eq!(rows[1].1, ["2"]);
        assert!(rows[1].2.contains("lift.add"));
    }
}
#[test]
fn simplification_does_not_propagate_locations_through_calls_or_block_entries() {
    let mut obj = fixture("sum_db.bisa");
    // Every recursive activation and block starts its own dataflow environment.
    let p = Program::compile(
        &obj,
        Passes {
            simplify: true,
            ..Passes::default()
        },
        false,
    )
    .unwrap();
    let f = obj.functions.iter_mut().find(|f| f.name == "sum").unwrap();
    let row = f
        .blocks
        .iter()
        .flat_map(|b| &b.instrs)
        .find(|i| i.op == "lift.call")
        .unwrap();
    let mut executed = None;
    for (b, block) in f.blocks.iter().enumerate() {
        if let Some(i) = block.instrs.iter().position(|i| i.line == row.line) {
            p.run(p.row("sum", b, i).unwrap(), |_, ins| {
                executed = Some(ins.op.clone());
                Ok(None)
            })
            .unwrap();
        }
    }
    assert_eq!(executed.as_deref(), Some("lift.call"));
}
#[test]
fn native_boundary_transports_panics_and_rejects_invalid_dispatch() {
    let p = Program::compile(&fixture("inc8.bisa"), Passes::all(), true).unwrap();
    let start = p.row("inc8", 0, 0).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        p.run(start, |_, _| panic!("native-boundary-witness"))
    }));
    assert!(result.is_err());
    assert!(
        p.run(usize::MAX, |_, _| Ok(None))
            .unwrap_err()
            .contains("entry outside")
    );
    assert!(
        p.run(start, |_, _| Ok(Some(usize::MAX - 1)))
            .unwrap_err()
            .contains("successor outside")
    );
}
