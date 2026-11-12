thread_local! {
    static ACTIVATION_COPIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static METADATA_BYTES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
trait OwnedCloneBytes {
    fn cloned_bytes(&self) -> u64;
}
impl OwnedCloneBytes for String {
    fn cloned_bytes(&self) -> u64 {
        self.len() as u64
    }
}
impl OwnedCloneBytes for Vec<String> {
    fn cloned_bytes(&self) -> u64 {
        (self.len() * std::mem::size_of::<String>()) as u64
            + self.iter().map(String::len).sum::<usize>() as u64
    }
}
impl<T: ?Sized> OwnedCloneBytes for std::rc::Rc<T> {
    fn cloned_bytes(&self) -> u64 {
        0
    }
}
impl Clone for Activation {
    fn clone(&self) -> Self {
        ACTIVATION_COPIES.with(|n| n.set(n.get() + 1));
        METADATA_BYTES
            .with(|n| n.set(n.get() + self.function.cloned_bytes() + self.dest.cloned_bytes()));
        Self {
            id: self.id,
            function: self.function.clone(),
            function_index: self.function_index,
            block: self.block,
            index: self.index,
            dest: self.dest.clone(),
            call_start: self.call_start,
            args: self.args.clone(),
        }
    }
}
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

#[test]
fn disabled_trace_copies_no_unreported_pre_values() {
    let body = "x = const.i64 1\n".repeat(1024);
    let source = format!(
        "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\nfn cc @main () () () () ()\n^entry():\n{body}ret\n"
    );
    let obj = std::sync::Arc::new(crate::object::parse(&source).unwrap());
    let resources = Resources::new(&obj).unwrap();
    let mut run = RuntimeRun::new(obj, &resources, "main", vec![], vec![]).unwrap();
    run.machine.trace = TraceLog::Disabled;
    let before = crate::block_scaling::value_copies();
    let activations = ACTIVATION_COPIES.with(std::cell::Cell::get);
    let bytes = METADATA_BYTES.with(std::cell::Cell::get);
    for _ in 0..1024 {
        run.step().unwrap();
    }
    let copies = crate::block_scaling::value_copies() - before;
    let activations = ACTIVATION_COPIES.with(std::cell::Cell::get) - activations;
    let bytes = METADATA_BYTES.with(std::cell::Cell::get) - bytes;
    eprintln!(
        "unreported pre-value copies={copies}, activation copies={activations}, metadata bytes={bytes}"
    );
    assert_eq!(copies, 0, "unreported register pre-values copied");
    assert!(
        activations <= 1024,
        "diagnostic activation copied without an observer"
    );
    assert_eq!(bytes, 0, "immutable activation metadata copied");
}
