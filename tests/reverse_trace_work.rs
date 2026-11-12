use super::*;

#[test]
fn reverse_trace_checks_rows_without_retaining_unreported_json() {
    let source = "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{}\n\nsection functions\nfn cc @main () () () () ()\n^entry():\nret\n";
    let obj = crate::object::parse(source).unwrap();
    let mut trace = TraceLog::cli(&Mode::Reverse).unwrap();
    let before = crate::step_scaling::trace_builds();
    for step in 0..1024 {
        trace.record(
            TraceRow {
                step,
                function: "main".into(),
                block: 0,
                index: 0,
                registers: vec![],
                memory: vec![],
            },
            &obj,
        );
    }
    assert_eq!(crate::step_scaling::trace_builds() - before, 1024);
    assert_eq!(trace.project(&obj).len(), 0, "unreported rows retained");
}

#[test]
#[should_panic(expected = "RFC 8785: nonfinite JSON number")]
fn reverse_unreported_trace_keeps_nonfinite_diagnostics() {
    let obj = crate::object::parse("bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{}\n\nsection functions\nfn cc @main () () () () ()\n^entry():\nret\n").unwrap();
    let mut trace = TraceLog::cli(&Mode::Reverse).unwrap();
    trace.record(
        TraceRow {
            step: 0,
            function: "main".into(),
            block: 0,
            index: 0,
            registers: vec![((0, "f".into()), Some(Value::F64(f64::NAN)), None)],
            memory: vec![],
        },
        &obj,
    );
}
