use super::*;

#[test]
fn preparing_integer_rows_does_not_clone_source_values() {
    let source = "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{}\n\nsection functions\nfn cc @main (a:i64, b:i64) (x:i64) () () ()\n^entry():\nx = add a b\nret x\n";
    let obj = std::sync::Arc::new(crate::object::parse(source).unwrap());
    let resources = Resources::new(&obj).unwrap();
    let run = RuntimeRun::new(
        obj,
        &resources,
        "main",
        vec![Value::int(64, u(9)), Value::int(64, u(3))],
        vec![],
    )
    .unwrap();
    let ins = &run.machine.obj.functions[0].blocks[0].instrs[0];
    let before = crate::block_scaling::value_copies();
    let result = run
        .machine
        .prepare_value(ins, ValueOp::parse(ins).unwrap())
        .unwrap();
    let copies = crate::block_scaling::value_copies() - before;
    eprintln!("prepared source value copies={copies}");
    assert_eq!(copies, 0, "read-only preparation copied source values");
    let PreparedValue::Definition(Value::Int { bits, .. }) = result else {
        panic!("missing sum")
    };
    assert_eq!(bits.to_u64(), Some(12));
}
