use super::*;

#[test]
fn pure_step_value_copies_are_independent_of_call_arity() {
    for arity in [1, 64] {
        let params = (0..arity)
            .map(|i| format!("a{i}:i64"))
            .collect::<Vec<_>>()
            .join(", ");
        let body = "x = const.i64 1\n".repeat(1024);
        let source = format!(
            "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\nfn cc @main ({params}) () () () ()\n^entry():\n{body}ret\n"
        );
        let obj = std::sync::Arc::new(crate::object::parse(&source).unwrap());
        let resources = Resources::new(&obj).unwrap();
        let mut run = RuntimeRun::new(
            obj,
            &resources,
            "main",
            vec![Value::int(64, u(0)); arity],
            vec![],
        )
        .unwrap();
        let before = crate::block_scaling::value_copies();
        for _ in 0..1024 {
            run.step().unwrap();
        }
        let copies = crate::block_scaling::value_copies() - before;
        eprintln!("arity={arity}, value copies={copies}");
        assert!(
            copies <= 8 * 1024,
            "call arguments copied on every pure row"
        );
    }
}
