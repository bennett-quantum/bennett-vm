//! Integer value rows: compute and reserve first, then publish without diagnostics.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum ValueOp {
    Const,
    Binary(&'static str),
    Divide(&'static str),
    Compare,
    Select,
    Cast(Cast),
}
#[derive(Clone, Copy)]
pub(super) enum Cast {
    Zero,
    Sign,
    Truncate,
    ToInt,
    ToPtr,
}
impl ValueOp {
    pub(super) fn parse(ins: &Instr) -> Option<Self> {
        Some(match ins.op.split('.').next()? {
            "const" => Self::Const,
            "add" => Self::Binary("add"),
            "sub" => Self::Binary("sub"),
            "mul" => Self::Binary("mul"),
            "and" => Self::Binary("and"),
            "or" => Self::Binary("or"),
            "xor" => Self::Binary("xor"),
            "shl" => Self::Binary("shl"),
            "lshr" => Self::Binary("lshr"),
            "ashr" => Self::Binary("ashr"),
            "udiv" => Self::Divide("udiv"),
            "urem" => Self::Divide("urem"),
            "sdiv" => Self::Divide("sdiv"),
            "srem" => Self::Divide("srem"),
            "icmp" => Self::Compare,
            "select" => Self::Select,
            "zext" => Self::Cast(Cast::Zero),
            "sext" => Self::Cast(Cast::Sign),
            "trunc" => Self::Cast(Cast::Truncate),
            "ptrtoint" => Self::Cast(Cast::ToInt),
            "inttoptr" => Self::Cast(Cast::ToPtr),
            _ => return None,
        })
    }
}

enum PreparedValue {
    Definition(Value),
    Fault(u64),
}

impl Machine {
    fn prepare_value(&self, ins: &Instr, op: ValueOp) -> Result<PreparedValue, String> {
        macro_rules! get {
            ($value:expr) => {
                match $value {
                    Ok(value) => value,
                    Err(code) => return Ok(PreparedValue::Fault(code)),
                }
            };
        }
        let xs = &ins.operands;
        let width = || -> Result<usize, String> {
            ins.op
                .rsplit_once(".i")
                .ok_or_else(|| err("missing width"))?
                .1
                .parse()
                .map_err(err)
        };
        let value = match op {
            ValueOp::Const => Value::int(width()?, crate::oracle::parse_const(&xs[0], width()?)?),
            ValueOp::Binary(base) | ValueOp::Divide(base) => {
                let a = get!(self.read(&xs[0]));
                let b = get!(self.bits(&xs[1]));
                let w = a.width(self.obj.ptr_width).unwrap();
                let bits = a.bits().unwrap();
                if matches!(op, ValueOp::Divide(_)) {
                    if let Some(code) = value::div_fault(base, bits, &b, w) {
                        return Ok(PreparedValue::Fault(code));
                    }
                    Value::int(w, value::divop(base, bits, &b, w))
                } else {
                    Value::int(w, value::binop(base, bits, &b, w)?)
                }
            }
            ValueOp::Compare => {
                let a = get!(self.read(&xs[0]));
                let b = get!(self.bits(&xs[1]));
                Value::int(
                    1,
                    u(value::icmp(
                        ins.op.split('.').nth(1).unwrap(),
                        a.bits().unwrap(),
                        &b,
                        a.width(self.obj.ptr_width).unwrap(),
                    ) as u64),
                )
            }
            ValueOp::Select => {
                let predicate = get!(self.bits(&xs[0]));
                get!(self.read(&xs[if predicate.is_zero() { 2 } else { 1 }]))
            }
            ValueOp::Cast(cast) => {
                let a = get!(self.read(&xs[0]));
                let bits = a.bits().unwrap();
                let w = match cast {
                    Cast::ToPtr => self.obj.ptr_width,
                    _ => width()?,
                };
                let out = match cast {
                    Cast::Sign => sign_extend(bits, a.width(self.obj.ptr_width).unwrap(), w),
                    _ => bits.mask(w),
                };
                match cast {
                    Cast::ToPtr => Value::Ptr(out),
                    _ => Value::int(w, out),
                }
            }
        };
        Ok(PreparedValue::Definition(value))
    }

    pub(super) fn step_prepared(&mut self, ins: &Instr, op: ValueOp) -> Result<(), String> {
        // No externally visible state changes until all diagnostics have succeeded.
        let value = self.prepare_value(ins, op)?;
        #[cfg(test)]
        preparation_boundary(PreparationStage::Value)?;
        let duration = self.quantum.prepare_classical_event()?;
        #[cfg(test)]
        preparation_boundary(PreparationStage::Accounting)?;
        if ins.lhs.len() != 1 {
            return Err(err("prepared value row requires one destination"));
        }
        let pure = matches!(value, PreparedValue::Definition(_));
        self.history.prepare_push(pure)?;
        self.definitions
            .try_reserve(1)
            .map_err(|e| err(format!("definition capacity: {e}")))?;
        #[cfg(test)]
        preparation_boundary(PreparationStage::Capacity)?;

        let a = self.stack.last().unwrap().clone();
        let mut frame = Frame {
            classes_before: self.quantum.wire_classes.clone(),
            classes_after: self.quantum.wire_classes.clone(),
            wire_boundary: self.quantum.wire_count(),
            region: None,
            definitions: vec![],
            next_activation: self.next_activation,
            memory_keys: BTreeSet::new(),
            data: None,
            after: None,
            control: self.stack.clone(),
            pure,
            events: FrameEvents::default(),
            marker: false,
            measured: false,
            tokens_created: vec![],
        };
        self.pending_region = None;
        self.register_changes.clear();
        self.data.writes.clear();
        self.definitions.clear();
        self.stack.last_mut().unwrap().index += 1;
        self.quantum.publish_classical_event(duration);
        match value {
            PreparedValue::Definition(value) => {
                self.define(&ins.lhs[0], value);
                frame.definitions = self.definitions.clone();
            }
            PreparedValue::Fault(code) => {
                self.fault = Some(code);
                self.results.clear();
            }
        }
        if !matches!(self.trace, TraceLog::Disabled) {
            let row = self.trace_row(&a, &self.data, &frame.memory_keys);
            self.trace.record(row, &self.obj);
        }
        self.history.push(frame, pure, false);
        self.steps += 1;
        #[cfg(test)]
        crate::execution_work::record(crate::execution_work::Operation::Step, 1);
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreparationStage {
    Value,
    Accounting,
    Capacity,
}
#[cfg(test)]
thread_local! {
    static REJECT: std::cell::Cell<Option<PreparationStage>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
fn preparation_boundary(stage: PreparationStage) -> Result<(), String> {
    if REJECT.with(|reject| reject.get() == Some(stage)) {
        Err(format!("injected preparation diagnostic at {stage:?}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &str) -> RuntimeRun {
        let text = format!(
            "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=128\n\nsection symbols\n{{}}\n\nsection functions\nfn cc @main () (x:i64) () () ()\n^entry():\n{body}\nret x\n"
        );
        let obj = std::sync::Arc::new(crate::object::parse(&text).unwrap());
        let mut resources = Resources::new(&obj).unwrap();
        resources.set_target(
            crate::target::Target::parse(include_str!(
                "../../tests/fixtures/r3-stepping/target.json"
            ))
            .unwrap(),
        );
        let id = resources.new_reg(2, "data").unwrap();
        resources.prepare(id, &[1, 0]);
        // Retain concrete shared bytes, a ledger and token across every pure row.
        resources.data.memory.write(&u(4095), &[0x12, 0x34, 0x56]);
        resources.mint_token(
            "unused".into(),
            value::Budget {
                steps: 1,
                heap: 0,
                stack: 0,
                depth: 0,
            },
        );
        RuntimeRun::new(obj, &resources, "main", vec![], vec![7, 8]).unwrap()
    }

    // Reversal itself is charged by the ISA target; state restoration does not
    // retract the execution/undo accounting stream.
    fn without_accounting(mut snapshot: Json) -> Json {
        let Json::Obj(fields) = &mut snapshot else {
            panic!("snapshot object")
        };
        fields
            .remove("target_resources")
            .expect("target accounting field");
        snapshot
    }

    #[test]
    fn diagnostics_at_each_preparation_boundary_publish_nothing_and_retry() {
        for stage in [
            PreparationStage::Value,
            PreparationStage::Accounting,
            PreparationStage::Capacity,
        ] {
            let mut run = run("x = const.i64 37");
            let before = run.snapshot("RUNNING").unwrap();
            let control = run.machine.stack.clone();
            let quantum = run.machine.quantum.clone();
            let accounting = quantum.target_accounting().unwrap();
            for _ in 0..2 {
                REJECT.with(|reject| reject.set(Some(stage)));
                let error = run.step().unwrap_err();
                REJECT.with(|reject| reject.set(None));
                assert!(error.contains("injected preparation diagnostic"));
                assert_eq!(run.snapshot("RUNNING").unwrap(), before, "{stage:?}");
                assert_eq!(run.machine.stack, control);
                assert!(run.machine.quantum.exact_equals(&quantum));
                assert_eq!(run.machine.quantum.target_accounting().unwrap(), accounting);
                assert_eq!(run.machine.reply_pos, 0);
                assert_eq!(run.machine.draw_pos, 0);
                assert_eq!(run.machine.data.entropy_pos, 0);
                assert!(run.machine.effects.saved.is_empty());
            }
            run.step().unwrap();
            assert_eq!(run.history_len(), 1);
            assert_eq!(run.machine.read("x"), Ok(Value::int(64, u(37))));
            run.reverse_to(0).unwrap();
            assert_eq!(
                without_accounting(run.snapshot("RUNNING").unwrap()),
                without_accounting(before.clone())
            );
            assert!(run.machine.quantum.exact_equals(&quantum));
            assert_eq!(
                run.machine.data.memory.ordered_bytes(),
                vec![(u(4095), 0x12), (u(4096), 0x34), (u(4097), 0x56)]
            );
        }
    }

    #[test]
    fn real_accounting_diagnostic_precedes_row_publication() {
        let mut run = run("x = const.i64 37");
        let target = std::sync::Arc::make_mut(run.machine.quantum.target.as_mut().unwrap());
        let Json::Obj(document) = &mut target.document else {
            panic!("target document")
        };
        let Json::Obj(capabilities) = document.get_mut("capabilities").unwrap() else {
            panic!("capabilities")
        };
        capabilities.remove("classical_step_time");
        let before = run.snapshot("RUNNING").unwrap();
        for _ in 0..2 {
            assert!(run.step().unwrap_err().contains("classical_step_time"));
            assert_eq!(run.history_len(), 0);
            assert_eq!(run.machine.stack.last().unwrap().index, 0);
            assert_eq!(run.machine.read("x"), Err(4));
            assert_eq!(run.snapshot("RUNNING").unwrap(), before);
        }
    }

    #[test]
    fn prepared_arithmetic_fault_retains_prefix_bytes_and_exact_control() {
        let mut run = run("x = const.i64 37\ny = const.i64 0\nx = udiv x y");
        run.step().unwrap();
        run.step().unwrap();
        let before = run.snapshot("RUNNING").unwrap();
        let bytes = run.machine.data.memory.ordered_bytes();
        for _ in 0..2 {
            run.step().unwrap();
            assert_eq!(run.fault(), Some(5));
            assert_eq!(run.history_len(), 3);
            assert!(!run.machine.history.get(2).pure);
            assert_eq!(run.machine.history.get(2).control.last().unwrap().index, 2);
            assert_eq!(run.machine.read("x"), Ok(Value::int(64, u(37))));
            assert_eq!(run.machine.data.memory.ordered_bytes(), bytes);
            run.reverse_to(2).unwrap();
            assert_eq!(
                without_accounting(run.snapshot("RUNNING").unwrap()),
                without_accounting(before.clone())
            );
        }
    }
}
