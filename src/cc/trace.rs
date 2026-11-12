//! Lossless diagnostic deltas, encoded eagerly only for explicit observers.
use super::*;

/// Typed diagnostic deltas retain pre-values even after uncall or tail rewind.
/// Source identity always resolves through the original object, including JIT rows.
#[derive(Clone)]
pub(super) struct TraceRow {
    step: u64,
    function: String,
    block: usize,
    index: usize,
    registers: Vec<(Key, Option<Value>, Option<Value>)>,
    memory: Vec<(BigUint, u8, u8)>,
}
impl TraceRow {
    // Preserve fallible conversions and source-row checks for unreported rows.
    // Only f64 conversion can fail; integer decimal formatting is not a check.
    fn check_json(&self, obj: &Object) {
        #[cfg(test)]
        reverse_work::record_validation();
        let block = &obj.function(&self.function).unwrap().blocks[self.block];
        let _ = &block.instrs[self.index];
        self.registers
            .iter()
            .flat_map(|(_, pre, post)| pre.iter().chain(post))
            .filter(|value| matches!(value, Value::F64(_)))
            .for_each(|value| {
                let _ = value.to_json();
            });
    }
    fn to_json(&self, obj: &Object) -> Json {
        #[cfg(test)]
        crate::step_scaling::record_trace_json_build();
        let block = &obj.function(&self.function).unwrap().blocks[self.block];
        let ins = &block.instrs[self.index];
        let registers = self
            .registers
            .iter()
            .map(|(k, pre, post)| {
                Json::obj(vec![
                    ("activation", Json::u64(k.0)),
                    ("name", Json::Str(k.1.clone())),
                    (
                        "pre",
                        pre.as_ref().map(Value::to_json).unwrap_or(Json::Null),
                    ),
                    (
                        "post",
                        post.as_ref().map(Value::to_json).unwrap_or(Json::Null),
                    ),
                ])
            })
            .collect();
        let memory = self
            .memory
            .iter()
            .map(|(address, pre, post)| {
                Json::obj(vec![
                    ("address", Json::u(address)),
                    ("pre", Json::u64(*pre as u64)),
                    ("post", Json::u64(*post as u64)),
                ])
            })
            .collect();
        Json::obj(vec![
            ("step", Json::u64(self.step)),
            ("function", Json::Str(self.function.clone())),
            ("block", Json::Str(block.name.clone())),
            ("index", Json::u64(self.index as u64)),
            ("line", Json::u64(ins.line as u64)),
            ("op", Json::Str(ins.op.to_string())),
            ("lhs", strs(&ins.lhs)),
            ("operands", strs(&ins.operands)),
            ("registers", Json::Arr(registers)),
            ("memory", Json::Arr(memory)),
        ])
    }
}
#[derive(Clone)]
pub(super) enum TraceLog {
    Disabled,
    Deferred(crate::append_log::AppendLog<TraceRow>),
    Observed(Option<crate::append_log::AppendLog<Json>>),
}
impl TraceLog {
    /// CLI defaults retain eager diagnostic behavior. BENNETT_VM_TRACE=off
    /// defers encoding until report export; on selects eager observation.
    pub(super) fn cli(mode: &Mode) -> Result<Self, String> {
        let enabled = match std::env::var("BENNETT_VM_TRACE").as_deref() {
            Ok("on") | Err(std::env::VarError::NotPresent) => true,
            Ok("off") => false,
            _ => return Err("BENNETT_VM_TRACE must be off or on".into()),
        };
        Ok(if *mode == Mode::Program {
            Self::Disabled
        } else if enabled {
            // Reverse reports no trace. Keep conversion diagnostics, then drop
            // each row instead of retaining JSON that can never be projected.
            Self::Observed((*mode != Mode::Reverse).then(crate::append_log::AppendLog::new))
        } else {
            Self::new(false)
        })
    }
    pub(super) fn new(observed: bool) -> Self {
        let json_rows = crate::append_log::AppendLog::new();
        let typed_rows = crate::append_log::AppendLog::new();
        let eager = Self::Observed(Some(json_rows));
        observed
            .then_some(eager)
            .unwrap_or(Self::Deferred(typed_rows))
    }
    pub(super) fn record(&mut self, row: TraceRow, obj: &Object) {
        match self {
            Self::Disabled => unreachable!("disabled trace received a row"),
            Self::Deferred(rows) => rows.push(row),
            Self::Observed(rows) => rows
                .as_mut()
                .map_or_else(|| row.check_json(obj), |rows| rows.push(row.to_json(obj))),
        }
    }
    pub(super) fn project(&self, obj: &Object) -> Vec<Json> {
        match self {
            Self::Disabled => vec![],
            Self::Deferred(rows) => rows.iter().map(|row| row.to_json(obj)).collect(),
            Self::Observed(rows) => rows.as_ref().map_or_else(Vec::new, |rows| rows.to_vec()),
        }
    }
    pub(super) fn truncate(&mut self, n: usize) {
        match self {
            Self::Disabled => (),
            Self::Deferred(rows) => rows.truncate(n),
            Self::Observed(rows) => rows.iter_mut().for_each(|rows| rows.truncate(n)),
        }
    }
}

impl Machine {
    pub(super) fn trace_row(
        &self,
        a: &Activation,
        before: &Data,
        addresses: &BTreeSet<BigUint>,
    ) -> TraceRow {
        let mut keys: BTreeSet<Key> = self.definitions.iter().map(|(k, _)| k.clone()).collect();
        keys.extend(
            self.register_changes
                .iter()
                .filter(|(k, pre)| **pre != current(&self.env, k))
                .map(|(k, _)| k.clone()),
        );
        keys.extend(self.nan_registers.keys().cloned());
        let registers = keys
            .into_iter()
            .map(|k| {
                let pre = self
                    .register_changes
                    .get(&k)
                    .cloned()
                    .unwrap_or_else(|| current(&self.env, &k));
                let post = current(&self.env, &k);
                (k, pre, post)
            })
            .collect();
        let memory = addresses
            .iter()
            .filter_map(|a| {
                let pre = *before.memory.get(a).unwrap_or(&0);
                let post = *self.data.memory.get(a).unwrap_or(&0);
                (pre != post)
                    .then_some((a, pre, post))
                    .map(|(a, pre, post)| (a.clone(), pre, post))
            })
            .collect();
        TraceRow {
            step: self.steps,
            function: a.function.to_string(),
            block: a.block,
            index: a.index,
            registers,
            memory,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &str, observed: bool) -> RuntimeRun {
        let source = format!(
            "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=64\n\nsection symbols\n{{}}\n\nsection functions\nfn cc @main () () () () ()\n^entry():\n{body}\n"
        );
        let obj = std::sync::Arc::new(crate::object::parse(&source).unwrap());
        let resources = Resources::new(&obj).unwrap();
        RuntimeRun::new(obj, &resources, "main", vec![], vec![])
            .unwrap()
            .collect_trace(observed)
    }

    #[test]
    fn negative_zero_pre_value_keeps_its_bits_and_legacy_json_encoding() {
        let mut reports = vec![];
        for observed in [false, true] {
            let mut run = run("f = select p g g\nret", observed);
            run.machine.define_at(0, "f", Value::F64(-0.0));
            run.machine.define_at(0, "g", Value::F64(0.0));
            run.machine.define_at(0, "p", Value::int(1, u(1)));
            run.step().unwrap();
            if let TraceLog::Deferred(rows) = &run.machine.trace {
                let (_, Some(Value::F64(pre)), Some(Value::F64(post))) =
                    &rows.iter().next().unwrap().registers[0]
                else {
                    panic!("missing typed floating delta")
                };
                assert_eq!(pre.to_bits(), (-0.0f64).to_bits());
                assert_eq!(post.to_bits(), 0.0f64.to_bits());
            }
            let trace = run.machine.trace.project(&run.machine.obj);
            // RFC 8785 serializes both zero signs as 0, retaining the f64 type.
            let expected = json::parse(r#"[{"activation":0,"name":"f","pre":{"type":"f64","value":0},"post":{"type":"f64","value":0}}]"#).unwrap();
            assert_eq!(trace[0].get("registers"), Some(&expected));
            reports.push(trace);
        }
        assert_eq!(reports[0], reports[1]);
    }

    #[test]
    fn deferred_nan_payloads_preserve_the_observed_nonfinite_diagnostic() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        // Two IEEE-754 NaN encodings: positive quiet payload and negative payload.
        // JSON has no nonfinite encoding; preserve the existing explicit panic.
        for bits in [0x7ff8_0000_0000_0123, 0xfff0_0000_0000_0456] {
            let mut deferred = run("n = const.i64 1\nret", false);
            deferred
                .machine
                .define_at(0, "f", Value::F64(f64::from_bits(bits)));
            deferred.step().unwrap();
            deferred.step().unwrap();
            let TraceLog::Deferred(rows) = &deferred.machine.trace else {
                panic!("deferred trace missing")
            };
            for row in rows.iter() {
                let (_, Some(Value::F64(pre)), Some(Value::F64(post))) = &row.registers[0] else {
                    panic!("NaN delta missing")
                };
                assert_eq!(pre.to_bits(), bits);
                assert_eq!(post.to_bits(), bits);
            }
            let lazy = catch_unwind(AssertUnwindSafe(|| {
                deferred.machine.trace.project(&deferred.machine.obj)
            }))
            .unwrap_err();
            let mut observed = run("n = const.i64 1\nret", true);
            observed
                .machine
                .define_at(0, "f", Value::F64(f64::from_bits(bits)));
            let eager = catch_unwind(AssertUnwindSafe(|| observed.step())).unwrap_err();
            let message = |payload: Box<dyn std::any::Any + Send>| {
                *payload.downcast::<&'static str>().unwrap()
            };
            assert_eq!(message(lazy), message(eager));
        }
    }

    #[test]
    fn host_reply_delta_projects_without_consuming_or_reissuing_input() {
        let raw = std::fs::read_to_string(
            crate::pins::isa().join("reference-interpreter/tests/fixtures/classical.bisa"),
        )
        .unwrap();
        let obj = std::sync::Arc::new(crate::load(&raw, false).unwrap().0);
        let resources = Resources::new(&obj).unwrap();
        let mut reports = vec![];
        for observed in [false, true] {
            let mut run = RuntimeRun::new(obj.clone(), &resources, "host", vec![], vec![])
                .unwrap()
                .collect_trace(observed);
            let initial = run.machine.data.clone();
            for _ in 0..4 {
                run.step().unwrap();
            }
            assert_eq!(
                run.request(),
                Some(("org.bennett.test".into(), vec![165, 0, 0]))
            );
            run.reply(&[1, 2, 3, 4, 5]).unwrap();
            while !run.terminal() {
                run.step().unwrap();
            }
            let report = run.snapshot("COMPLETED").unwrap();
            let trace = report.get("trace").unwrap().as_arr().unwrap();
            let expected = json::parse(r#"[{"address":16,"pre":165,"post":1},{"address":17,"pre":0,"post":2},{"address":18,"pre":0,"post":3}]"#).unwrap();
            assert_eq!(trace[4].get("memory"), Some(&expected));
            assert_eq!(run.machine.data.read(&u(16), 3).unwrap(), [1, 2, 3]);
            assert_eq!(run.machine.reply_pos, 1);
            assert_eq!(run.machine.data.hosts.len(), 1);
            assert_eq!(run.snapshot("COMPLETED").unwrap(), report);
            assert_eq!(run.machine.reply_pos, 1);
            assert!(run.request().is_none());
            run.reverse_to(4).unwrap();
            assert_eq!(run.machine.data.read(&u(16), 3).unwrap(), [165, 0, 0]);
            assert_eq!(run.machine.trace.project(&obj).len(), 4);
            // Deliberate re-execution supplies the same fixed external input;
            // snapshot projection above itself consumes no input.
            run.reply(&[1, 2, 3, 4, 5]).unwrap();
            while !run.terminal() {
                run.step().unwrap();
            }
            assert_eq!(run.snapshot("COMPLETED").unwrap(), report);
            run.reverse_to(0).unwrap();
            assert!(run.machine.data.memory_eq(&initial));
            assert_eq!(run.machine.data.memory, initial.memory);
            assert_eq!(run.machine.data.table, initial.table);
            assert_eq!(run.machine.data.handles, initial.handles);
            assert_eq!(run.machine.data.tokens, initial.tokens);
            assert!(run.machine.env.is_empty());
            assert!(run.machine.trace.project(&obj).is_empty());
            reports.push(report);
        }
        assert_eq!(reports[0], reports[1]);
    }
}

#[cfg(test)]
#[path = "../../tests/reverse_trace_work.rs"]
mod reverse_work;
