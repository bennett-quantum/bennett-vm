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

#[test]
fn control_snapshot_copies_one_node_per_row_at_any_depth() {
    let obj = std::sync::Arc::new(crate::object::parse("bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{}\n\nsection functions\nfn cc @main () () () () ()\n^entry():\nret\n").unwrap());
    let resources = Resources::new(&obj).unwrap();
    for depth in [1, 128] {
        let mut run = RuntimeRun::new(obj.clone(), &resources, "main", vec![], vec![]).unwrap();
        for id in 1..depth {
            let mut activation = run.machine.stack.last().unwrap().clone();
            activation.id = id;
            run.machine.stack.push(activation);
        }
        let probe = crate::execution_work::Probe::start();
        let mut retained = vec![];
        for _ in 0..1024 {
            retained.push(run.machine.stack.clone());
            run.machine.stack.last_mut().unwrap().index += 1;
        }
        let copies = probe.snapshot().path_copies;
        eprintln!("control depth={depth}, path copies={copies}");
        assert_eq!(copies, 1024, "control snapshot copied a search path");
        assert_eq!(retained[0].last().unwrap().index, 0);
    }
}
