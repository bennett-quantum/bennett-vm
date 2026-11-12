use crate::object::Instr;
use std::cell::Cell;
thread_local! { static COPIES: Cell<u64> = const { Cell::new(0) }; }
pub(crate) fn copies() -> u64 {
    COPIES.with(Cell::get)
}
impl Clone for Instr {
    fn clone(&self) -> Self {
        COPIES.with(|n| n.set(n.get() + 1));
        Self {
            lhs: self.lhs.clone(),
            op: self.op.clone(),
            operands: self.operands.clone(),
            nested: self.nested.clone(),
            line: self.line,
            raw: self.raw.clone(),
        }
    }
}
